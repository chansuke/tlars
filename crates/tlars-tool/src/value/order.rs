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

use super::{SetEnumValue, UserValue, Value};
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use tlars_msg::Ec;
use std::cmp::Ordering;
use std::sync::Arc;

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
            (Value::User(_), _) => Ok(self.user_compare(ev, other)? == Ordering::Equal),
            _ => Err(EvalError::unimplemented(format!("equality of {} (M2 unit 3)", self.kind_string()))),
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
            (Value::User(_), _) => self.user_compare(ev, other),
            _ => Err(EvalError::unimplemented(format!("comparison of {} (M2 unit 3)", self.kind_string()))),
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
                _ => Err(EvalError::unimplemented("membership in this special set (M2 unit 3)".into())),
            },
            _ => Err(EvalError::unimplemented(format!("membership in {} (M2 unit 3)", self.kind_string()))),
        }
    }

    /// `size()` (Cardinality), with `TLC_MODULE_ARGUMENT_ERROR`-style text for infinite sets.
    pub(crate) fn size(&self, ev: &mut Ev<'_>) -> EvalResult<i32> {
        todo!()
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
