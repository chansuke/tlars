//! The set operators: `Reducible`'s eager `cup` / `cap` / `diff`, `isSubsetEq`, and the
//! constructors of the lazy classes the evaluator builds when no side reduces.
//!
//! Which side reduces decides the representation, and the representation is observable:
//! `{1} \cup SUBSET {}` is a SetEnum built from the left, `SUBSET {} \cup {1}` a SetEnum
//! built from the right (TLC swaps the operands), and `SUBSET {} \cup Nat` a SetCupValue
//! that prints and enumerates symbolically.

use super::Source;
use super::enumerate::EnumOrder;
use super::{SetBinValue, SetOfFcnsValue, SetOfRcdsValue, SetOfTuplesValue, SubsetValue, UnionValue, Value, ValueVec};
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use std::cmp::Ordering;
use std::sync::{Arc, OnceLock};
use tlars_sany::Atom;

/// The elements of a `Reducible` in the order its own array holds them, read one at a time
/// so that a large interval is not materialised before the set bound stops it.
enum Reducible<'v> {
    Slice(&'v [Value]),
    Interval { lo: i32, size: i32 },
}

impl Reducible<'_> {
    fn len(&self) -> usize {
        match self {
            Reducible::Slice(s) => s.len(),
            Reducible::Interval { size, .. } => *size as usize,
        }
    }
    fn at(&self, i: usize) -> Value {
        match self {
            Reducible::Slice(s) => s[i].clone(),
            Reducible::Interval { lo, .. } => Value::Int(lo.wrapping_add(i as i32)),
        }
    }
}

impl Value {
    fn reducible(&self, ev: &mut Ev<'_>) -> EvalResult<Reducible<'_>> {
        match self {
            Value::SetEnum(s) => Ok(Reducible::Slice(s.snapshot())),
            Value::Interval(i) => Ok(Reducible::Interval { lo: i.lo, size: i.size(ev)? }),
            _ => unreachable!("the Reducible classes are SetEnumValue and IntervalValue"),
        }
    }

    /// `Reducible.cup(other)` (`SetEnumValue.java:216-242`, `IntervalValue.java:189-213`):
    /// the receiver's elements in its own order, then the elements of `other` it lacks.
    pub(crate) fn cup(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Value> {
        let r = self.cup_inner(ev, other);
        self.fp(r)
    }

    fn cup_inner(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Value> {
        let elems = self.reducible(ev)?;
        if elems.len() == 0 {
            return Ok(other.clone());
        }
        if !other.is_reducible() {
            return Ok(lazy_cup(self.clone(), other.clone()));
        }
        let mut vals = ValueVec::new(ev);
        for i in 0..elems.len() {
            vals.push(elems.at(i))?;
        }
        let mut rest = other.elements(ev, EnumOrder::Native)?.expect("a Reducible value enumerates");
        while let Some(e) = rest.next(ev)? {
            if !self.member(ev, &e)? {
                vals.push(e)?;
            }
        }
        Ok(Value::set_enum_of(vals.into_vec(), false))
    }

    /// `Reducible.cap(other)`: the receiver's elements that `other` holds. An interval's
    /// result is normalised, a SetEnum's result is normalised exactly when the receiver is.
    pub(crate) fn cap(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Value> {
        let r = self.cap_inner(ev, other);
        self.fp(r)
    }

    fn cap_inner(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Value> {
        self.filter(ev, other, true)
    }

    /// `Reducible.diff(other)`: the receiver's elements that `other` lacks.
    pub(crate) fn diff(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Value> {
        let r = self.diff_inner(ev, other);
        self.fp(r)
    }

    fn diff_inner(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Value> {
        self.filter(ev, other, false)
    }

    fn filter(&self, ev: &mut Ev<'_>, other: &Value, keep: bool) -> EvalResult<Value> {
        let is_norm = matches!(self, Value::Interval(_)) || self.is_normalized();
        let elems = self.reducible(ev)?;
        let (len, other) = (elems.len(), other.clone());
        let mut vals = ValueVec::new(ev);
        for i in 0..len {
            let elem = elems.at(i);
            if other.member(ev, &elem)? == keep {
                vals.push(elem)?;
            }
        }
        Ok(Value::set_enum_of(vals.into_vec(), is_norm))
    }

    /// `isSubsetEq(other)` (`EnumerableValue.java:44-60`), with the two overrides that
    /// answer without enumerating.
    pub(crate) fn is_subset_eq(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<bool> {
        let r = self.is_subset_eq_inner(ev, other);
        self.fp(r)
    }

    fn is_subset_eq_inner(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<bool> {
        if let (Value::Interval(a), Value::Interval(b)) = (self, other)
            && b.lo <= a.lo
            && b.hi >= a.hi
        {
            return Ok(true);
        }
        // `SUBSET S \subseteq SUBSET T` reduces to `S \subseteq T`, but not for a k-subset
        // on either side (`SubsetValue.java:118-137`).
        if let (Value::Subset(a), Value::Subset(b)) = (self, other)
            && a.base.is_enumerable()
        {
            return a.base.is_subset_eq(ev, &b.base);
        }
        let mut elems = self.elements(ev, EnumOrder::Native)?.expect("an enumerable value enumerates");
        while let Some(e) = elems.next(ev)? {
            if !other.member(ev, &e)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `UnionValue.union(val)` (`UnionValue.java:215-239`): a set of sets that are all
    /// SetEnums is united at once, and the result's own elements are the duplicate check, so
    /// it stays unnormalised.
    /// `UnionValue.union(Value)` is a static method, so a failure inside it is not wrapped
    /// by the receiver's source the way the instance operators are.
    pub(crate) fn union(&self, ev: &mut Ev<'_>) -> EvalResult<Value> {
        let combinable = match self {
            Value::SetEnum(s) => s.snapshot().iter().all(|e| matches!(e, Value::SetEnum(_))),
            _ => false,
        };
        if !combinable {
            return Ok(Value::Union(Arc::new(UnionValue { set: self.clone(), converted: OnceLock::new(), source: Source::none() })));
        }
        let Value::SetEnum(outer) = self else { unreachable!("combinable implies a SetEnum") };
        let mut vals = ValueVec::new(ev);
        for inner in outer.snapshot().to_vec() {
            let Value::SetEnum(inner) = inner else { unreachable!("combinable implies sets of sets") };
            for elem in inner.snapshot().to_vec() {
                // The result's own `member` is the duplicate check, and it searches the
                // array being filled, which is unnormalised, so the scan uses `equals`.
                let mut found = false;
                for i in 0..vals.size() {
                    if vals.get(i).tlc_equals(ev, &elem)? {
                        found = true;
                        break;
                    }
                }
                if !found {
                    vals.push(elem)?;
                }
            }
        }
        Ok(Value::set_enum_of(vals.into_vec(), false))
    }
}

/// `new SetCupValue(set1, set2)`.
pub(crate) fn lazy_cup(left: Value, right: Value) -> Value {
    Value::SetCup(Arc::new(SetBinValue { left, right, converted: OnceLock::new(), source: Source::none() }))
}

/// `new SetCapValue(set1, set2)`.
pub(crate) fn lazy_cap(left: Value, right: Value) -> Value {
    Value::SetCap(Arc::new(SetBinValue { left, right, converted: OnceLock::new(), source: Source::none() }))
}

/// `new SetDiffValue(set1, set2)`.
pub(crate) fn lazy_diff(left: Value, right: Value) -> Value {
    Value::SetDiff(Arc::new(SetBinValue { left, right, converted: OnceLock::new(), source: Source::none() }))
}

/// `new SubsetValue(set)`: `SUBSET S` is never built eagerly.
pub(crate) fn subset(base: Value) -> Value {
    Value::Subset(Arc::new(SubsetValue { base, converted: OnceLock::new(), source: Source::none() }))
}

/// `new SetOfTuplesValue(sets)`.
pub(crate) fn set_of_tuples(sets: Vec<Value>) -> Value {
    Value::SetOfTuples(Arc::new(SetOfTuplesValue { sets: sets.into_boxed_slice(), converted: OnceLock::new(), source: Source::none() }))
}

/// `new SetOfFcnsValue(domain, range)`.
pub(crate) fn set_of_fcns(domain: Value, range: Value) -> Value {
    Value::SetOfFcns(Arc::new(SetOfFcnsValue { domain, range, converted: OnceLock::new(), source: Source::none() }))
}

/// `new SetOfRcdsValue(names, values, false)`, whose constructor sorts the fields by name
/// token and refuses a repeated one (`SetOfRcdsValue.java:287-321`).
pub(crate) fn set_of_rcds(ev: &mut Ev<'_>, names: Vec<Atom>, sets: Vec<Value>) -> EvalResult<Value> {
    let (mut names, mut sets) = (names, sets);
    let duplicate = |ev: &mut Ev<'_>, name: Atom| {
        let text = ev.tool().graph().interner().text(name);
        EvalError::detailed(format!("Field name {text} occurs multiple times in set of records."), None)
    };
    for i in 1..names.len() {
        match names[0].tok().cmp(&names[i].tok()) {
            Ordering::Equal => return Err(duplicate(ev, names[0])),
            Ordering::Greater => {
                names.swap(0, i);
                sets.swap(0, i);
            }
            Ordering::Less => {}
        }
    }
    for i in 2..names.len() {
        let (name, set) = (names[i], sets[i].clone());
        let mut j = i;
        let mut cmp = Ordering::Less;
        while j > 0 {
            cmp = name.tok().cmp(&names[j - 1].tok());
            if cmp != Ordering::Less {
                break;
            }
            names[j] = names[j - 1];
            sets[j] = sets[j - 1].clone();
            j -= 1;
        }
        // The name is read back from the partly shifted array, as TLC reads it.
        if cmp == Ordering::Equal {
            return Err(duplicate(ev, names[i]));
        }
        names[j] = name;
        sets[j] = set;
    }
    Ok(Value::SetOfRcds(Arc::new(SetOfRcdsValue {
        names: names.into_boxed_slice(),
        sets: sets.into_boxed_slice(),
        converted: OnceLock::new(),
        source: Source::none(),
    })))
}
