//! TLC's equality, total order and membership: the single owner of
//! - cardinality-first set order, functions by size/domain/values, records by field count,
//!   field tokens, then values; strings and model values by intern token;
//! - the untyped-model-value rule (less than, and unequal to, any non-model value) and
//!   typed-model-value errors;
//! - every "Attempted to compare ..." / "Attempted to check equality of ..." text.
//!
//! Dispatch is receiver-first, as in Java (`a.equals(b)` runs `a`'s class's code), because
//! the error text names the receiver's class. Cross-representation results (Interval vs
//! SetEnum, Tuple vs FcnRcd, Record vs string-domain FcnRcd) are defined once over
//! [`Canon`] (grafted from candidate 2). A representation fast path (e.g. Interval vs
//! Interval by bounds) goes through [`fast_checked`], which in debug builds recomputes the
//! answer through `Canon` and panics on disagreement, so "fast path agrees" is a structural
//! check, not a promise.

use super::{FcnRcdValue, IntervalValue, RecordValue, SetEnumValue, UserValue, Value};
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use std::cmp::Ordering;
use std::sync::Arc;
use tlars_msg::Ec;
use tlars_sany::Atom;

impl Value {
    /// `IValue.equals`. The evaluator calls `lhs.tlc_equals(ev, rhs)` for `lhs = rhs`.
    pub(crate) fn tlc_equals(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<bool> {
        if matches!(other, Value::Model(_)) {
            return Err(EvalError::unimplemented("ModelValue.modelValueEquals (M3)".into()));
        }
        // The scalar classes accept their own class and model values, and fail otherwise
        // naming the receiver (`BoolValue.java`, `IntValue.java`, `StringValue.java`).
        let mismatch = |ev: &mut Ev<'_>, verb: &str, noun: &str| -> EvalResult<bool> {
            let (a, b) = (self.ppr(ev)?, other.ppr(ev)?);
            Err(EvalError::detailed(format!("Attempted to {verb} equality of {noun} {a} with non-{noun}:\n{b}"), None))
        };
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => Ok(a == b),
            (Value::Bool(_), _) => mismatch(ev, "compare", "boolean"),
            (Value::Int(a), Value::Int(b)) => Ok(a == b),
            (Value::Int(_), _) => mismatch(ev, "check", "integer"),
            (Value::Str(a), Value::Str(b)) => Ok(a == b),
            (Value::Str(_), _) => mismatch(ev, "check", "string"),
            (Value::Interval(a), Value::Interval(b)) => Ok(if a.lo > a.hi { b.lo > b.hi } else { a.lo == b.lo && a.hi == b.hi }),
            (Value::Interval(a), _) => Value::SetEnum(a.to_set_enum_arc()).tlc_equals(ev, other),
            (Value::SetEnum(a), _) => {
                let Some(b) = other.to_set_enum(ev)? else {
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to check equality of the set {x} with the value:\n{y}"), None));
                };
                let (xs, ys) = (normalized(ev, a)?.to_vec(), normalized(ev, &b)?.to_vec());
                if xs.len() != ys.len() {
                    return Ok(false);
                }
                for (x, y) in xs.iter().zip(&ys) {
                    if !x.tlc_equals(ev, y)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            // A value that is not a tuple is compared as a function instead, so
            // `<<1, 2>> = [i \in 1..2 |-> i]` holds (`TupleValue.java:93-116`).
            (Value::Tuple(a), _) => match other.to_tuple(ev)? {
                Some(b) => {
                    if a.elems.len() != b.elems.len() {
                        return Ok(false);
                    }
                    for (x, y) in a.elems.iter().zip(b.elems.iter()) {
                        if !x.tlc_equals(ev, y)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                None => {
                    let f = self.to_fcn_rcd(ev)?.expect("a tuple converts to a function");
                    Value::FcnRcd(f).tlc_equals(ev, other)
                }
            },
            (Value::Record(a), _) => {
                let Some(b) = other.to_rcd(ev)? else {
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to check equality of record:\n{x}\nwith non-record\n{y}"), None));
                };
                let (an, av) = record_normalized(ev, a)?;
                let (bn, bv) = record_normalized(ev, &b)?;
                if an.len() != bn.len() || an != bn {
                    return Ok(false);
                }
                for (x, y) in av.iter().zip(bv.iter()) {
                    if !x.tlc_equals(ev, y)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Value::FcnRcd(a), _) => {
                let Some(b) = other.to_fcn_rcd(ev)? else {
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to check equality of the function {x} with the value:\n{y}"), None));
                };
                fcn_equals(ev, a, &b)
            }
            (Value::FcnLambda(a), _) => {
                let f = a.to_fcn_rcd(ev)?;
                Value::FcnRcd(f).tlc_equals(ev, other)
            }
            (Value::User(_), _) => Ok(self.user_compare(ev, other)? == Ordering::Equal),
            _ => Err(EvalError::unimplemented(format!("equality of {} (M2 unit 5)", self.kind_string()))),
        }
    }

    /// `UserValue.compareTo` (`UserValue.java`): another special set is compared by the
    /// receiver's `UserObj` (`Naturals.java:226-240`, `Integers.java`, `Strings.java`), a
    /// model value is below, anything else is an error.
    fn user_compare(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Ordering> {
        let Value::User(u) = self else { unreachable!("user_compare on a user value") };
        if !matches!(other, Value::User(_)) {
            if matches!(other, Value::Model(_)) {
                return Ok(Ordering::Greater);
            }
            let (a, b) = (self.ppr(ev)?, other.ppr(ev)?);
            return Err(EvalError::detailed(format!("Attempted to compare overridden value {a} with non-overridden value:\n{b}"), None));
        }
        let (rank, name) = match **u {
            UserValue::Nat => (0, "Nat"),
            UserValue::Int => (1, "Int"),
            UserValue::String => {
                // `Strings.compareTo`: only STRING itself and model values compare.
                return match other {
                    Value::User(o) if matches!(**o, UserValue::String) => Ok(Ordering::Equal),
                    Value::Model(_) => Ok(Ordering::Greater),
                    _ => {
                        let shown = other.ppr(ev)?;
                        Err(crate::modules::eval_exception(ev, Ec::TLC_MODULE_COMPARE_VALUE, &["STRING", &shown]))
                    }
                };
            }
            _ => return Err(EvalError::unimplemented("comparison of this special set (M2 unit 3)".into())),
        };
        match other {
            Value::User(o) if matches!(**o, UserValue::Nat) => Ok(rank.cmp(&0)),
            Value::User(o) if matches!(**o, UserValue::Int) => Ok(rank.cmp(&1)),
            Value::Model(_) => Ok(Ordering::Greater),
            _ => {
                let shown = other.ppr(ev)?;
                Err(crate::modules::eval_exception(ev, Ec::TLC_MODULE_COMPARE_VALUE, &[name, &shown]))
            }
        }
    }

    /// `IValue.compareTo`.
    pub(crate) fn tlc_compare(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Ordering> {
        if matches!(other, Value::Model(_)) {
            return Err(EvalError::unimplemented("ModelValue.modelValueCompareTo (M3)".into()));
        }
        let mismatch = |ev: &mut Ev<'_>, noun: &str| -> EvalResult<Ordering> {
            let (a, b) = (self.ppr(ev)?, other.ppr(ev)?);
            Err(EvalError::detailed(format!("Attempted to compare {noun} {a} with non-{noun}:\n{b}"), None))
        };
        match (self, other) {
            (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
            (Value::Bool(_), _) => mismatch(ev, "boolean"),
            (Value::Int(a), Value::Int(b)) => Ok(a.cmp(b)),
            (Value::Int(_), _) => mismatch(ev, "integer"),
            // `UniqueString.compareTo`: by intern token, not by text.
            (Value::Str(a), Value::Str(b)) => Ok(a.tok().cmp(&b.tok())),
            (Value::Str(_), _) => mismatch(ev, "string"),
            (Value::Interval(a), Value::Interval(b)) => {
                let (sa, sb) = (a.size(), b.size());
                Ok(if sa != sb { sa.cmp(&sb) } else if sa == 0 { Ordering::Equal } else { a.lo.cmp(&b.lo) })
            }
            (Value::Interval(a), _) => Value::SetEnum(a.to_set_enum_arc()).tlc_compare(ev, other),
            (Value::SetEnum(a), _) => {
                let Some(b) = other.to_set_enum(ev)? else {
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to compare the set {x} with the value:\n{y}"), None));
                };
                let (xs, ys) = (normalized(ev, a)?.to_vec(), normalized(ev, &b)?.to_vec());
                if xs.len() != ys.len() {
                    return Ok(xs.len().cmp(&ys.len()));
                }
                for (x, y) in xs.iter().zip(&ys) {
                    let c = x.tlc_compare(ev, y)?;
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                Ok(Ordering::Equal)
            }
            // A tuple's domain is 1..Len, so equal lengths mean equal domains
            // (`TupleValue.java:67-91`).
            (Value::Tuple(a), _) => match other.to_tuple(ev)? {
                Some(b) => {
                    if a.elems.len() != b.elems.len() {
                        return Ok(a.elems.len().cmp(&b.elems.len()));
                    }
                    for (x, y) in a.elems.iter().zip(b.elems.iter()) {
                        let c = x.tlc_compare(ev, y)?;
                        if c != Ordering::Equal {
                            return Ok(c);
                        }
                    }
                    Ok(Ordering::Equal)
                }
                None => {
                    let f = self.to_fcn_rcd(ev)?.expect("a tuple converts to a function");
                    Value::FcnRcd(f).tlc_compare(ev, other)
                }
            },
            (Value::Record(a), _) => {
                let Some(b) = other.to_rcd(ev)? else {
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to compare record:\n{x}\nwith non-record\n{y}"), None));
                };
                let (an, av) = record_normalized(ev, a)?;
                let (bn, bv) = record_normalized(ev, &b)?;
                if an.len() != bn.len() {
                    return Ok(an.len().cmp(&bn.len()));
                }
                for (x, y) in an.iter().zip(bn.iter()) {
                    let c = x.tok().cmp(&y.tok());
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                for (x, y) in av.iter().zip(bv.iter()) {
                    let c = x.tlc_compare(ev, y)?;
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                Ok(Ordering::Equal)
            }
            (Value::FcnRcd(a), _) => {
                let Some(b) = other.to_fcn_rcd(ev)? else {
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to compare the function {x} with the value:\n{y}"), None));
                };
                fcn_compare(ev, a, &b)
            }
            (Value::FcnLambda(a), _) => {
                let f = a.to_fcn_rcd(ev)?;
                Value::FcnRcd(f).tlc_compare(ev, other)
            }
            (Value::User(_), _) => self.user_compare(ev, other),
            _ => Err(EvalError::unimplemented(format!("comparison of {} (M2 unit 5)", self.kind_string()))),
        }
    }

    /// `member(elem)`: `elem \in self`.
    pub(crate) fn member(&self, ev: &mut Ev<'_>, elem: &Value) -> EvalResult<bool> {
        if matches!(elem, Value::Model(_)) {
            return Err(EvalError::unimplemented("membership of a model value (M3)".into()));
        }
        let not_a_set = |ev: &mut Ev<'_>, what: &str| -> EvalResult<bool> {
            let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
            Err(EvalError::detailed(format!("Attempted to check if the value:\n{e}\nis an element of the {what} {s}"), None))
        };
        match self {
            Value::Bool(_) => not_a_set(ev, "boolean"),
            Value::Int(_) => not_a_set(ev, "integer"),
            Value::Str(_) => not_a_set(ev, "string"),
            Value::Interval(i) => match elem {
                Value::Int(x) => Ok(*x >= i.lo && *x <= i.hi),
                _ if i.lo <= i.hi => {
                    let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                    Err(EvalError::detailed(format!("Attempted to check if the value:\n{e}\nis in the integer interval {s}"), None))
                }
                _ => Ok(false),
            },
            // `ValueVec.search`: binary search once normalised, `equals` scan before.
            Value::SetEnum(set) => match set.norm.get() {
                Some(norm) => {
                    let (mut lo, mut hi) = (0, norm.len());
                    while lo < hi {
                        let mid = lo + (hi - lo) / 2;
                        match elem.tlc_compare(ev, &norm[mid])? {
                            Ordering::Equal => return Ok(true),
                            Ordering::Less => hi = mid,
                            Ordering::Greater => lo = mid + 1,
                        }
                    }
                    Ok(false)
                }
                None => {
                    for e in set.built.iter() {
                        if e.tlc_equals(ev, elem)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
            },
            Value::User(u) => match (&**u, elem) {
                (UserValue::Nat, Value::Int(x)) => Ok(*x >= 0),
                (UserValue::Int, Value::Int(_)) => Ok(true),
                (UserValue::String, Value::Str(_)) => Ok(true),
                (UserValue::Nat | UserValue::Int | UserValue::String, _) => {
                    let set = match **u {
                        UserValue::Nat => "Nat",
                        UserValue::Int => "Int",
                        _ => "STRING",
                    };
                    let shown = elem.ppr(ev)?;
                    Err(crate::modules::eval_exception(ev, Ec::TLC_MODULE_CHECK_MEMBER_OF, &[&shown, set]))
                }
                _ => Err(EvalError::unimplemented("membership in this special set (M2 unit 5)".into())),
            },
            Value::Tuple(_) => Err(EvalError::detailed("Attempted to check set membership in a tuple value.".into(), None)),
            Value::Record(_) => {
                let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                Err(EvalError::detailed(format!("Attempted to check if element:\n{e}\nis in the record:\n{s}"), None))
            }
            Value::FcnRcd(_) | Value::FcnLambda(_) => {
                let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                Err(EvalError::detailed(format!("Attempted to check if the value:\n{e}\nis an element of the function {s}"), None))
            }
            _ => Err(EvalError::unimplemented(format!("membership in {} (M2 unit 5)", self.kind_string()))),
        }
    }

    /// `size()`: the cardinality a set reports and the number of points a function has.
    pub(crate) fn size(&self, ev: &mut Ev<'_>) -> EvalResult<i32> {
        let uncountable = |ev: &mut Ev<'_>, noun: &str| -> EvalResult<i32> {
            let shown = self.ppr(ev)?;
            Err(EvalError::detailed(format!("Attempted to compute the number of elements in the {noun} {shown}."), None))
        };
        match self {
            Value::Bool(_) => uncountable(ev, "boolean"),
            Value::Int(_) => uncountable(ev, "integer"),
            Value::Str(_) => uncountable(ev, "string"),
            Value::Model(_) => uncountable(ev, "model value"),
            Value::User(_) => uncountable(ev, "overridden value"),
            Value::Undef => uncountable(ev, "value"),
            Value::Interval(i) => Ok(i.size()),
            Value::SetEnum(s) => Ok(normalized(ev, s)?.len() as i32),
            Value::Tuple(t) => Ok(t.elems.len() as i32),
            Value::Record(r) => Ok(r.names.len() as i32),
            Value::FcnRcd(f) => {
                f.normalize(ev)?;
                Ok(f.values.len() as i32)
            }
            Value::FcnLambda(f) => match f.converted.get() {
                Some(r) => Value::FcnRcd(r.clone()).size(ev),
                None => f.params_size(ev),
            },
            _ => Err(EvalError::unimplemented(format!("the cardinality of {} (M2 unit 5)", self.kind_string()))),
        }
    }
}

/// `SetEnumValue.normalize()` / `ValueVec.sort(true)`: binary insertion, dedup on
/// `compareTo == 0` (the element already placed stays). Publishes into `norm` on success
/// only.
pub(crate) fn normalized<'a>(ev: &mut Ev<'_>, set: &'a SetEnumValue) -> EvalResult<&'a [Value]> {
    if let Some(norm) = set.norm.get() {
        return Ok(norm);
    }
    let mut out: Vec<Value> = Vec::with_capacity(set.built.len());
    let mut elems = set.built.iter();
    if let Some(first) = elems.next() {
        out.push(first.clone());
    }
    for elem in elems {
        let (mut lo, mut hi, mut idx) = (0, out.len(), 0);
        let mut cmp = Ordering::Equal;
        while lo < hi {
            idx = lo + (hi - lo) / 2;
            cmp = elem.tlc_compare(ev, &out[idx])?;
            match cmp {
                Ordering::Equal => break,
                Ordering::Less => hi = idx,
                Ordering::Greater => lo = idx + 1,
            }
        }
        if cmp != Ordering::Equal {
            out.insert(if cmp == Ordering::Less { idx } else { idx + 1 }, elem.clone());
        }
    }
    Ok(set.norm.get_or_init(|| out.into_boxed_slice()))
}

/// `RecordValue.normalize()` / `sort()` (`RecordValue.java:619-651`): the smallest field
/// name is swapped to the front, then insertion sort, both in place, so the duplicate the
/// error names is read from the partly sorted array as TLC reads it.
pub(crate) fn record_normalized<'a>(ev: &mut Ev<'_>, r: &'a RecordValue) -> EvalResult<(&'a [Atom], &'a [Value])> {
    if let Some((names, values)) = r.norm.get() {
        return Ok((names, values));
    }
    let mut names = r.names.to_vec();
    let mut values = r.values.to_vec();
    let duplicate = |ev: &mut Ev<'_>, name: Atom| {
        let text = ev.tool().graph().interner().text(name);
        EvalError::detailed(format!("Field name {text} occurs multiple times in record."), None)
    };
    for i in 1..names.len() {
        match names[0].tok().cmp(&names[i].tok()) {
            Ordering::Equal => return Err(duplicate(ev, names[i])),
            Ordering::Greater => {
                names.swap(0, i);
                values.swap(0, i);
            }
            Ordering::Less => {}
        }
    }
    for i in 2..names.len() {
        let (name, value) = (names[i], values[i].clone());
        let mut j = i;
        let mut cmp = Ordering::Less;
        while j > 0 {
            cmp = name.tok().cmp(&names[j - 1].tok());
            if cmp != Ordering::Less {
                break;
            }
            names[j] = names[j - 1];
            values[j] = values[j - 1].clone();
            j -= 1;
        }
        if cmp == Ordering::Equal {
            return Err(duplicate(ev, names[i]));
        }
        names[j] = name;
        values[j] = value;
    }
    let _ = r.norm.set((names.into_boxed_slice(), values.into_boxed_slice()));
    let (names, values) = r.norm.get().expect("just published");
    Ok((names, values))
}

/// `FcnRcdValue.normalize()` / `sort()` (`FcnRcdValue.java:714-749`), the same algorithm over
/// an explicit domain. Only the explicit case sorts: an interval domain is already ordered.
pub(crate) fn fcn_normalized<'a>(ev: &mut Ev<'_>, f: &'a FcnRcdValue) -> EvalResult<(&'a [Value], &'a [Value])> {
    if let Some((domain, values)) = f.norm.get() {
        return Ok((domain, values));
    }
    let super::FcnDomain::Explicit(built) = &f.domain else { unreachable!("an interval domain is normalised") };
    let mut domain = built.to_vec();
    let mut values = f.values.to_vec();
    fcn_sort(ev, &mut domain, &mut values)?;
    let _ = f.norm.set((domain.into_boxed_slice(), values.into_boxed_slice()));
    let (domain, values) = f.norm.get().expect("just published");
    Ok((domain, values))
}

/// `FcnRcdValue.sort()` in place. `toString` runs it over a private copy, so a function the
/// evaluator never normalises keeps printing in construction order.
pub(crate) fn fcn_sort(ev: &mut Ev<'_>, domain: &mut [Value], values: &mut [Value]) -> EvalResult<()> {
    let duplicate = |ev: &mut Ev<'_>, v: &Value| -> EvalResult<EvalError> {
        let shown = v.to_tlc_string(ev)?;
        Ok(EvalError::detailed(format!("The value\n{shown}\noccurs multiple times in the function domain."), None))
    };
    for i in 1..domain.len() {
        match domain[0].tlc_compare(ev, &domain[i])? {
            Ordering::Equal => return Err(duplicate(ev, &domain[i].clone())?),
            Ordering::Greater => {
                domain.swap(0, i);
                values.swap(0, i);
            }
            Ordering::Less => {}
        }
    }
    for i in 2..domain.len() {
        let (d, v) = (domain[i].clone(), values[i].clone());
        let mut j = i;
        let mut cmp = Ordering::Less;
        while j > 0 {
            cmp = d.tlc_compare(ev, &domain[j - 1])?;
            if cmp != Ordering::Less {
                break;
            }
            domain[j] = domain[j - 1].clone();
            values[j] = values[j - 1].clone();
            j -= 1;
        }
        if cmp == Ordering::Equal {
            return Err(duplicate(ev, &domain[i].clone())?);
        }
        domain[j] = d;
        values[j] = v;
    }
    Ok(())
}

/// `IntervalValue.equals(IntervalValue)`: every empty interval is the same set.
fn interval_equals(a: IntervalValue, b: IntervalValue) -> bool {
    (a.size() == 0 && b.size() == 0) || (a.lo == b.lo && a.hi == b.hi)
}

/// `FcnRcdValue.equals` (`FcnRcdValue.java:261-340`). An interval domain is compared against
/// an explicit one element by element rather than being materialised.
fn fcn_equals(ev: &mut Ev<'_>, a: &FcnRcdValue, b: &FcnRcdValue) -> EvalResult<bool> {
    let (ad, av) = a.normalize(ev)?;
    let (bd, bv) = b.normalize(ev)?;
    let not_integer = |ev: &mut Ev<'_>, v: &Value| -> EvalResult<EvalError> {
        let shown = v.ppr(ev)?;
        Ok(EvalError::detailed(format!("Attempted to compare an integer with non-integer:\n{shown}."), None))
    };
    // The side with the interval walks the other side's domain as lo, lo+1, ...
    let (interval, explicit) = match (a.interval(), b.interval()) {
        (Some(ai), Some(bi)) => {
            if !interval_equals(ai, bi) {
                return Ok(false);
            }
            return values_equal(ev, av, bv);
        }
        (Some(ai), None) => {
            if bd.expect("an explicit domain").len() as i64 != i64::from(ai.size()) {
                return Ok(false);
            }
            (ai, bd.expect("an explicit domain"))
        }
        (None, Some(bi)) => {
            if av.len() != bv.len() {
                return Ok(false);
            }
            (bi, ad.expect("an explicit domain"))
        }
        (None, None) => {
            if av.len() != bv.len() {
                return Ok(false);
            }
            let (ad, bd) = (ad.expect("an explicit domain"), bd.expect("an explicit domain"));
            for (x, y) in ad.iter().zip(bd.iter()) {
                if !x.tlc_equals(ev, y)? {
                    return Ok(false);
                }
            }
            return values_equal(ev, av, bv);
        }
    };
    for (i, d) in explicit.iter().enumerate() {
        let Value::Int(n) = d else { return Err(not_integer(ev, d)?) };
        if i64::from(*n) != i64::from(interval.lo) + i as i64 {
            return Ok(false);
        }
    }
    values_equal(ev, av, bv)
}

fn values_equal(ev: &mut Ev<'_>, av: &[Value], bv: &[Value]) -> EvalResult<bool> {
    for (x, y) in av.iter().zip(bv.iter()) {
        if !x.tlc_equals(ev, y)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `FcnRcdValue.compareTo` (`FcnRcdValue.java:149-259`): size, then domain, then values. The
/// "non-integer" text differs by one colon between the two directions, as in TLC.
fn fcn_compare(ev: &mut Ev<'_>, a: &FcnRcdValue, b: &FcnRcdValue) -> EvalResult<Ordering> {
    let (ad, av) = a.normalize(ev)?;
    let (bd, bv) = b.normalize(ev)?;
    if av.len() != bv.len() {
        return Ok(av.len().cmp(&bv.len()));
    }
    match (a.interval(), b.interval()) {
        (Some(ai), bi) => {
            if av.is_empty() {
                return Ok(Ordering::Equal);
            }
            match bi {
                Some(bi) => {
                    let c = ai.lo.cmp(&bi.lo);
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                None => {
                    for (i, d) in bd.expect("an explicit domain").iter().enumerate() {
                        let Value::Int(n) = d else {
                            let shown = d.ppr(ev)?;
                            return Err(EvalError::detailed(format!("Attempted to compare integer with non-integer:\n{shown}."), None));
                        };
                        let c = (i64::from(ai.lo) + i as i64).cmp(&i64::from(*n));
                        if c != Ordering::Equal {
                            return Ok(c);
                        }
                    }
                }
            }
            values_compare(ev, av, bv)
        }
        (None, Some(bi)) => {
            for (i, d) in ad.expect("an explicit domain").iter().enumerate() {
                let Value::Int(n) = d else {
                    let shown = d.ppr(ev)?;
                    return Err(EvalError::detailed(format!("Attempted to compare integer with non-integer\n{shown}."), None));
                };
                let c = i64::from(*n).cmp(&(i64::from(bi.lo) + i as i64));
                if c != Ordering::Equal {
                    return Ok(c);
                }
            }
            values_compare(ev, av, bv)
        }
        (None, None) => {
            let (ad, bd) = (ad.expect("an explicit domain"), bd.expect("an explicit domain"));
            for (x, y) in ad.iter().zip(bd.iter()) {
                let c = x.tlc_compare(ev, y)?;
                if c != Ordering::Equal {
                    return Ok(c);
                }
            }
            values_compare(ev, av, bv)
        }
    }
}

fn values_compare(ev: &mut Ev<'_>, av: &[Value], bv: &[Value]) -> EvalResult<Ordering> {
    for (x, y) in av.iter().zip(bv.iter()) {
        let c = x.tlc_compare(ev, y)?;
        if c != Ordering::Equal {
            return Ok(c);
        }
    }
    Ok(Ordering::Equal)
}

/// The one representation-independent view that `tlc_equals`, `tlc_compare` and
/// `fingerprint` are written against: sets as normalized element slices, functions
/// (tuples, records, FcnRcd, materialized FcnLambda) as normalized `(key, value)` pairs.
pub(crate) enum Canon<'v> {
    Scalar(&'v Value),
    Set(Arc<SetEnumValue>),
    Fcn(Arc<super::FcnRcdValue>),
}

pub(crate) fn canon<'v>(ev: &mut Ev<'_>, v: &'v Value) -> EvalResult<Canon<'v>> {
    todo!("to_set_enum / to_fcn_rcd, error text from the receiver's class")
}

/// Run `fast`; in debug builds also run `slow` (the `Canon` path) and assert equality.
pub(crate) fn fast_checked<T: PartialEq + std::fmt::Debug>(
    ev: &mut Ev<'_>,
    fast: impl FnOnce(&mut Ev<'_>) -> EvalResult<T>,
    slow: impl FnOnce(&mut Ev<'_>) -> EvalResult<T>,
) -> EvalResult<T> {
    let answer = fast(ev)?;
    if cfg!(debug_assertions) {
        assert_eq!(answer, slow(ev)?, "representation fast path disagrees with Canon");
    }
    Ok(answer)
}
