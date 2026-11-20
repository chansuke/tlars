//! Native (`elements()`, TLC's UNDEFINED) and normalised (`elements(NORMALIZED)`) orders of
//! every set class. The single owner of:
//! - SetEnum: always normalises first;
//! - Interval: low to high;
//! - Subset: k = 0..n, k-subsets in lexicographic index order over normalised S;
//! - SetOfFcns / SetOfRcds / SetOfTuples: odometer, *last* component fastest, each
//!   component in its own native order;
//! - Union: outer elements, then each inner set's elements (duplicates possible);
//! - SetCup: all of left, then all of right (duplicates); SetCap/SetDiff: left filtered;
//! - SetPred: inner set's order, filtered by forcing the predicate.
//! `x' \in S` and Init's `x \in S` use `Native`; `\E`, `\A`, CHOOSE use `Normalized`.

use super::{IntervalValue, SetPredValue, Value};
use crate::ctx::{Binding, Ctx};
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use std::sync::Arc;
use tlars_sany::Atom;
use tlars_sany::graph::SymbolRef;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EnumOrder {
    Native,
    Normalized,
    /// `-generate` / probabilistic mode.
    Randomized,
}

/// `ValueEnumeration`. Stepping may evaluate user code (SetPred), so it takes `ev`.
pub(crate) struct ValueEnumeration {
    state: EnumState,
}

/// How a `Product` turns one element per constituent set into one element of the set
/// (`SetOfFcnsOrRcdsValue.Product.elementOf`).
enum Build {
    Tuple,
    /// Field names, already sorted by token, and `isNorm = true`.
    Record(Box<[Atom]>),
    /// The function's domain, already normalised, and `isNorm = true`.
    Fcn(Arc<[Value]>),
    FcnInterval(IntervalValue),
}

enum EnumState {
    /// `EnumerableValue.EMPTY_ENUMERATION`.
    Empty,
    /// `SubsetValue.emptyEnumeration`: the one subset of an empty set.
    OnlyEmptySet { done: bool },
    Interval { lo: i32, next: i64, hi: i64 },
    Slice { elems: Box<[Value]>, next: usize },
    /// `SetCupValue.Enumerator`.
    Cup { first: Box<ValueEnumeration>, second: Box<ValueEnumeration> },
    /// `SetCapValue.Enumerator` (`keep`) and `SetDiffValue.Enumerator` (`!keep`): the
    /// enumerable side, filtered by membership in the other.
    Filter { inner: Box<ValueEnumeration>, other: Value, keep: bool },
    /// `UnionValue.Enumerator`; `inner` is `None` once the outer set is exhausted.
    Union { outer: Box<ValueEnumeration>, inner: Option<Box<ValueEnumeration>> },
    /// `SetPredValue.Enumerator`.
    Pred { inner: Box<ValueEnumeration>, set: Arc<SetPredValue> },
    /// `SubsetValue.elementsNormalized`: subsets by increasing size, each size in
    /// lexicographic order of indices into the normalised base set.
    Subset { elems: Arc<[Value]>, n: usize, k: usize, indices: Vec<usize> },
    /// The odometer of `[D -> R]`, `[f : S]` and `S \X T`, with the last component fastest.
    Product { enums: Vec<ValueEnumeration>, current: Vec<Value>, done: bool, build: Build },
}

impl ValueEnumeration {
    fn of(state: EnumState) -> ValueEnumeration {
        ValueEnumeration { state }
    }

    pub(crate) fn next(&mut self, ev: &mut Ev<'_>) -> EvalResult<Option<Value>> {
        match &mut self.state {
            EnumState::Empty => Ok(None),
            EnumState::OnlyEmptySet { done } => {
                if *done {
                    return Ok(None);
                }
                *done = true;
                Ok(Some(Value::set_enum_normalized(Vec::new())))
            }
            EnumState::Interval { next, hi, .. } => {
                if *next > *hi {
                    return Ok(None);
                }
                let v = Value::Int(*next as i32);
                *next += 1;
                Ok(Some(v))
            }
            EnumState::Slice { elems, next } => {
                let v = elems.get(*next).cloned();
                *next += 1;
                Ok(v)
            }
            EnumState::Cup { first, second } => match first.next(ev)? {
                Some(v) => Ok(Some(v)),
                None => second.next(ev),
            },
            EnumState::Filter { inner, other, keep } => {
                let (other, keep) = (other.clone(), *keep);
                while let Some(elem) = inner.next(ev)? {
                    if other.member(ev, &elem)? == keep {
                        return Ok(Some(elem));
                    }
                }
                Ok(None)
            }
            EnumState::Union { outer, inner } => {
                loop {
                    let Some(cur) = inner.as_mut() else { return Ok(None) };
                    if let Some(v) = cur.next(ev)? {
                        return Ok(Some(v));
                    }
                    match outer.next(ev)? {
                        None => {
                            *inner = None;
                            return Ok(None);
                        }
                        Some(set) => {
                            if !set.is_enumerable() {
                                // TLC's message prints the enumerator's identity hash
                                // (`UnionValue.java:371-373` passes the inner class's `this`),
                                // which no reimplementation can match.
                                return Err(EvalError::unimplemented(
                                    "TLC's message for a non-enumerable element found while enumerating a UNION".into(),
                                ));
                            }
                            *inner = Some(Box::new(set.elements(ev, EnumOrder::Native)?.expect("an enumerable value enumerates")));
                        }
                    }
                }
            }
            EnumState::Pred { inner, set } => {
                let set = set.clone();
                while let Some(elem) = inner.next(ev)? {
                    let c1 = set.bind(ev, &elem)?;
                    let (s0, s1) = set.env.states();
                    let res = ev.eval(set.pred, &c1, s0, s1, set.env.control)?;
                    match res {
                        Value::Bool(true) => return Ok(Some(elem)),
                        Value::Bool(false) => {}
                        _ => {
                            let pred = ev.tool().graph().expr_tlc_text(set.pred);
                            return Err(EvalError::detailed(format!("Evaluating predicate {pred} yielded non-Boolean value."), None));
                        }
                    }
                }
                Ok(None)
            }
            EnumState::Subset { elems, n, k, indices } => {
                let (n, elems) = (*n, elems.clone());
                if *k > n {
                    return Ok(None);
                }
                if *k == 0 {
                    subset_reset(k, indices, n, 1);
                    return Ok(Some(Value::set_enum_normalized(Vec::new())));
                }
                let mut vals: Vec<Value> = vec![Value::Undef; *k];
                let mut i = *k as isize - 1;
                for j in (0..*k).rev() {
                    vals[j] = elems[indices[j]].clone();
                    if indices[j] + *k - j == n {
                        i = j as isize - 1;
                    }
                }
                let result = Value::set_enum_normalized(vals);
                if indices[0] == n - *k {
                    subset_reset(k, indices, n, *k + 1);
                    return Ok(Some(result));
                }
                let mut i = i as usize;
                indices[i] += 1;
                i += 1;
                while i < *k {
                    indices[i] = indices[i - 1] + 1;
                    i += 1;
                }
                Ok(Some(result))
            }
            EnumState::Product { enums, current, done, build } => {
                if *done {
                    return Ok(None);
                }
                let elems = current.clone();
                if enums.is_empty() {
                    *done = true;
                    return build.element(elems).map(Some);
                }
                for i in (0..enums.len()).rev() {
                    match enums[i].next(ev)? {
                        Some(v) => {
                            current[i] = v;
                            break;
                        }
                        None => {
                            if i == 0 {
                                *done = true;
                                break;
                            }
                            enums[i].reset(ev)?;
                            current[i] = enums[i].next(ev)?.expect("a reset enumeration of a non-empty set has an element");
                        }
                    }
                }
                build.element(elems).map(Some)
            }
        }
    }

    /// `reset()`: start over, as the odometer of `ContextEnumerator` does.
    pub(crate) fn reset(&mut self, ev: &mut Ev<'_>) -> EvalResult<()> {
        match &mut self.state {
            EnumState::Empty => Ok(()),
            EnumState::OnlyEmptySet { done } => {
                *done = false;
                Ok(())
            }
            EnumState::Interval { lo, next, .. } => {
                *next = i64::from(*lo);
                Ok(())
            }
            EnumState::Slice { next, .. } => {
                *next = 0;
                Ok(())
            }
            EnumState::Cup { first, second } => {
                first.reset(ev)?;
                second.reset(ev)
            }
            EnumState::Filter { inner, .. } | EnumState::Pred { inner, .. } => inner.reset(ev),
            EnumState::Union { outer, inner } => {
                outer.reset(ev)?;
                match outer.next(ev)? {
                    Some(set) if set.is_enumerable() => {
                        *inner = Some(Box::new(set.elements(ev, EnumOrder::Native)?.expect("an enumerable value enumerates")));
                        Ok(())
                    }
                    // `UnionValue.Enumerator.reset` dereferences the next element without a
                    // null check (`UnionValue.java:357-361`), so an exhausted or
                    // non-enumerable one raises a JVM exception TLC never prints on purpose.
                    _ => Err(EvalError::unimplemented("TLC's NullPointerException from resetting a UNION enumeration".into())),
                }
            }
            EnumState::Subset { n, k, indices, .. } => {
                let n = *n;
                subset_reset(k, indices, n, 0);
                Ok(())
            }
            EnumState::Product { enums, current, done, .. } => {
                for i in 0..enums.len() {
                    enums[i].reset(ev)?;
                    current[i] = enums[i].next(ev)?.expect("a reset enumeration of a non-empty set has an element");
                }
                *done = false;
                Ok(())
            }
        }
    }
}

/// `SubsetValue.elementsNormalized`'s `reset(k)`.
fn subset_reset(k: &mut usize, indices: &mut [usize], n: usize, to: usize) {
    *k = to;
    if to > n {
        return;
    }
    for (i, idx) in indices.iter_mut().enumerate().take(to) {
        *idx = i;
    }
}

impl Build {
    fn element(&self, values: Vec<Value>) -> EvalResult<Value> {
        Ok(match self {
            Build::Tuple => Value::tuple(values),
            Build::Record(names) => Value::Record(Value::record_arc(names.to_vec(), values, true)),
            Build::Fcn(domain) => Value::FcnRcd(Value::fcn_rcd_arc(domain.to_vec(), values, true)),
            Build::FcnInterval(iv) => Value::FcnRcd(Value::fcn_rcd_interval_arc(*iv, values)),
        })
    }
}

impl SetPredValue {
    /// The predicate's context for one candidate element: the bound variable, or the
    /// components of a `<<x, y>> \in S` list (`SetPredValue.java:378-400`).
    pub(crate) fn bind(&self, ev: &mut Ev<'_>, elem: &Value) -> EvalResult<Ctx> {
        let mut c1 = self.env.ctx.clone();
        if !self.tuple {
            return Ok(c1.cons(SymbolRef::Param(self.vars[0]), Binding::Value(elem.clone())));
        }
        let tuple = elem.to_tuple(ev)?.filter(|t| t.elems.len() == self.vars.len());
        let Some(tuple) = tuple else {
            let shown = elem.ppr(ev)?;
            let n = self.vars.len();
            return Err(EvalError::detailed(
                format!("Attempted to check if the value:\n{shown}\nis an element of a set of {n}-tuples."),
                None,
            ));
        };
        for (&id, v) in self.vars.iter().zip(tuple.elems.iter()) {
            c1 = c1.cons(SymbolRef::Param(id), Binding::Value(v.clone()));
        }
        Ok(c1)
    }
}

impl Value {
    /// `elements(ordering)`; `Ok(None)` if the value is not enumerable (caller reports
    /// `getNonEnumerableErrorMsg`).
    pub(crate) fn elements(&self, ev: &mut Ev<'_>, order: EnumOrder) -> EvalResult<Option<ValueEnumeration>> {
        if !self.is_enumerable() {
            return Ok(None);
        }
        if order == EnumOrder::Randomized {
            return Err(EvalError::unimplemented("a randomized enumeration (M5)".into()));
        }
        // `EnumerableValue.elements(NORMALIZED)` converts and sorts first. SubsetValue
        // overrides it because its own order is already normalised, and an Interval's
        // materialised form is in the same order as the interval itself.
        let converts = order == EnumOrder::Normalized
            && !matches!(self, Value::Subset(_) | Value::KSubset(_) | Value::Interval(_) | Value::SetEnum(_));
        if converts {
            let set = self.to_set_enum(ev)?.expect("an enumerable value converts");
            let elems: Box<[Value]> = super::order::normalized(ev, &set)?.into();
            return Ok(Some(ValueEnumeration::of(EnumState::Slice { elems, next: 0 })));
        }
        self.native_elements(ev).map(Some)
    }

    /// `elements()`: the class's own order, which is what `Ordering.UNDEFINED` asks for.
    fn native_elements(&self, ev: &mut Ev<'_>) -> EvalResult<ValueEnumeration> {
        let slice = |ev: &mut Ev<'_>, set: &Arc<super::SetEnumValue>| -> EvalResult<ValueEnumeration> {
            let elems: Box<[Value]> = super::order::normalized(ev, set)?.into();
            Ok(ValueEnumeration::of(EnumState::Slice { elems, next: 0 }))
        };
        match self {
            Value::Interval(i) => {
                Ok(ValueEnumeration::of(EnumState::Interval { lo: i.lo, next: i64::from(i.lo), hi: i64::from(i.hi) }))
            }
            // `SetEnumValue.Enumerator` normalises in its constructor, so a SetEnum has one
            // order only.
            Value::SetEnum(set) => slice(ev, set),
            Value::SetCup(s) => match s.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    if !s.left.is_enumerable() || !s.right.is_enumerable() {
                        let (a, b) = (s.left.ppr(ev)?, s.right.ppr(ev)?);
                        return Err(EvalError::detailed(
                            format!("Attempted to enumerate S \\cup T when S:\n{a}\nand T:\n{b}\nare not both enumerable"),
                            None,
                        ));
                    }
                    let first = Box::new(s.left.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    let second = Box::new(s.right.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    Ok(ValueEnumeration::of(EnumState::Cup { first, second }))
                }
            },
            Value::SetCap(s) => match s.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    let (inner, other) = if s.left.is_enumerable() {
                        (&s.left, s.right.clone())
                    } else if s.right.is_enumerable() {
                        (&s.right, s.left.clone())
                    } else {
                        let (a, b) = (s.left.ppr(ev)?, s.right.ppr(ev)?);
                        return Err(EvalError::detailed(
                            format!("Attempted to enumerate S \\cap T when neither S:\n{a}\nnor T:\n{b}\nis enumerable"),
                            None,
                        ));
                    };
                    let inner = Box::new(inner.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    Ok(ValueEnumeration::of(EnumState::Filter { inner, other, keep: true }))
                }
            },
            Value::SetDiff(s) => match s.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    if !s.left.is_enumerable() {
                        let a = s.left.ppr(ev)?;
                        return Err(EvalError::detailed(format!("Attempted to enumerate S \\ T when S:\n{a}\nis not enumerable."), None));
                    }
                    let inner = Box::new(s.left.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    Ok(ValueEnumeration::of(EnumState::Filter { inner, other: s.right.clone(), keep: false }))
                }
            },
            Value::Union(u) => match u.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    if !u.set.is_enumerable() {
                        // The message prints the enumerator's identity hash
                        // (`UnionValue.java:343-344`).
                        return Err(EvalError::unimplemented("TLC's message for enumerating UNION of a non-enumerable set".into()));
                    }
                    let mut outer = Box::new(u.set.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    let inner = match outer.next(ev)? {
                        None => None,
                        Some(set) => {
                            if !set.is_enumerable() {
                                return Err(EvalError::detailed(
                                    "Attempted to enumerate UNION(s), but some element of s is nonenumerable.".into(),
                                    None,
                                ));
                            }
                            Some(Box::new(set.elements(ev, EnumOrder::Native)?.expect("enumerable")))
                        }
                    };
                    Ok(ValueEnumeration::of(EnumState::Union { outer, inner }))
                }
            },
            Value::SetPred(p) => match p.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    if !p.inner.is_enumerable() {
                        let shown = p.inner.ppr(ev)?;
                        return Err(EvalError::detailed(
                            format!("Attempted to enumerate {{ x \\in S : p(x) }} when S:\n{shown}\nis not enumerable"),
                            None,
                        ));
                    }
                    let inner = Box::new(p.inner.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    Ok(ValueEnumeration::of(EnumState::Pred { inner, set: p.clone() }))
                }
            },
            Value::Subset(s) => match s.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    let n = s.base.size(ev)? as usize;
                    if n == 0 {
                        return Ok(ValueEnumeration::of(EnumState::OnlyEmptySet { done: false }));
                    }
                    let Some(base) = s.base.to_set_enum(ev)? else {
                        let shown = s.base.ppr(ev)?;
                        return Err(EvalError::detailed(
                            format!("Attempted to compute the value of an expression of form\nSUBSET S, but S is a non-enumerable value:\n{shown}"),
                            None,
                        ));
                    };
                    let elems: Arc<[Value]> = super::order::normalized(ev, &base)?.into();
                    Ok(ValueEnumeration::of(EnumState::Subset { elems, n, k: 0, indices: vec![0; n] }))
                }
            },
            Value::KSubset(_) => Err(EvalError::unimplemented("enumerating a k-subset (M5)".into())),
            Value::SetOfTuples(s) => match s.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    if self.is_empty(ev)? {
                        return Ok(ValueEnumeration::of(EnumState::Empty));
                    }
                    let mut enums = Vec::with_capacity(s.sets.len());
                    for (i, set) in s.sets.iter().enumerate() {
                        if !set.is_enumerable() {
                            let shown = set.ppr(ev)?;
                            return Err(EvalError::detailed(
                                format!("Attempted to enumerate a set of the form s1 \\X s2 ... \\X sn,\nbut can't enumerate s{i}:\n{shown}"),
                                None,
                            ));
                        }
                        enums.push(set.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    }
                    product(ev, enums, Build::Tuple)
                }
            },
            Value::SetOfRcds(s) => match s.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    if self.is_empty(ev)? {
                        return Ok(ValueEnumeration::of(EnumState::Empty));
                    }
                    let mut enums = Vec::with_capacity(s.sets.len());
                    for (i, set) in s.sets.iter().enumerate() {
                        if !set.is_enumerable() {
                            let (name, shown) = (ev.tool().graph().interner().text(s.names[i]).to_string(), set.ppr(ev)?);
                            return Err(EvalError::detailed(
                                format!("Attempted to enumerate a set of the form [l1 : v1, ..., ln : vn],\nbut can't enumerate the value of the `{name}' field:\n{shown}"),
                                None,
                            ));
                        }
                        enums.push(set.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    }
                    product(ev, enums, Build::Record(s.names.clone()))
                }
            },
            Value::SetOfFcns(s) => match s.converted.get().cloned() {
                Some(set) => slice(ev, &set),
                None => {
                    if self.is_empty(ev)? {
                        return Ok(ValueEnumeration::of(EnumState::Empty));
                    }
                    // An interval domain keeps its `intv`, so the functions it builds print
                    // as sequences (`SetOfFcnsValue.DomIVEnumerator`).
                    let (count, build) = match &s.domain {
                        Value::Interval(iv) => (iv.size(ev)? as usize, Build::FcnInterval(*iv)),
                        domain => {
                            let Some(set) = domain.to_set_enum(ev)? else {
                                let shown = domain.ppr(ev)?;
                                return Err(EvalError::detailed(
                                    format!("Attempted to enumerate a set of the form [D -> R],but the domain D:\n{shown}\ncannot be enumerated."),
                                    None,
                                ));
                            };
                            let elems: Arc<[Value]> = super::order::normalized(ev, &set)?.into();
                            (elems.len(), Build::Fcn(elems))
                        }
                    };
                    if count > 0 && !s.range.is_enumerable() {
                        let shown = s.range.ppr(ev)?;
                        return Err(EvalError::detailed(
                            format!("Attempted to enumerate a set of the form [D -> R],but the range R:\n{shown}\ncannot be enumerated."),
                            None,
                        ));
                    }
                    let mut enums = Vec::with_capacity(count);
                    for _ in 0..count {
                        enums.push(s.range.elements(ev, EnumOrder::Native)?.expect("enumerable"));
                    }
                    product(ev, enums, build)
                }
            },
            _ => unreachable!("is_enumerable covers the set classes"),
        }
    }
}

/// The odometer's first element, primed as TLC's enumerators prime it in their constructor:
/// one empty constituent makes the whole enumeration done.
fn product(ev: &mut Ev<'_>, mut enums: Vec<ValueEnumeration>, build: Build) -> EvalResult<ValueEnumeration> {
    let mut current = Vec::with_capacity(enums.len());
    let mut done = false;
    for e in enums.iter_mut() {
        match e.next(ev)? {
            Some(v) => current.push(v),
            None => {
                done = true;
                break;
            }
        }
    }
    Ok(ValueEnumeration::of(EnumState::Product { enums, current, done, build }))
}

impl Value {
    /// `getNonEnumerableErrorMsg(domain)` (`Value.java:447-450`, `Naturals.java:275`,
    /// `Integers.java`).
    pub(crate) fn non_enumerable_message(&self, ev: &mut Ev<'_>, domain_text: &str) -> EvalResult<String> {
        let shown = self.ppr(ev)?;
        let set = match self {
            Value::User(u) => match **u {
                super::UserValue::Nat => Some("Nat"),
                super::UserValue::Int => Some("Int"),
                super::UserValue::String => None,
                // `Sequences.getNonEnumerableErrorMsg` (`Sequences.java:506-517`).
                super::UserValue::Seq(_) => {
                    return Ok(format!(
                        "TLC encountered the non-enumerable quantifier bound\n{shown}\n{domain_text}\n\
                         In TLA+, Seq(S) represents the set of all finite sequences whose elements come from the set S. Even when S\n\
                         is a finite set, the number of possible sequences in Seq(S) is unbounded because sequences can have any\n\
                         finite length (e.g., length 0, 1, 2, and so on). As a result, TLC cannot evaluate expressions that\n\
                         universally (\\A) or existentially (\\E) quantify over {shown}, because this would require checking an\n\
                         infinite number of cases. Note that for a finite set of sequences s, TLC handles s \\subseteq Seq(S).\n\
                         See https://explain.tlapl.us/seq-unenumerable for additional details."
                    ));
                }
                _ => return Err(EvalError::unimplemented("the non-enumerable message of this special set (M2 unit 3)".into())),
            },
            _ => None,
        };
        Ok(match set {
            Some(set) => format!(
                "TLC encountered the non-enumerable quantifier bound\n{shown}\n{domain_text}\n\
                 The set {set} contains infinitely many elements. As a result, TLC cannot evaluate expressions that\n\
                 universally (\\A) or existentially (\\E) quantify over {shown}, because this would require checking an\n\
                 infinite number of cases. Note that TLC handles set membership like T \\subseteq {set} for any finite set T."
            ),
            None => format!("TLC encountered a non-enumerable quantifier bound\n{shown}.\n{domain_text}"),
        })
    }
}
