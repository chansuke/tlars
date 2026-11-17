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

use super::{SetEnumValue, Value};
use crate::eval::Ev;
use crate::error::EvalResult;
use std::cmp::Ordering;
use std::sync::Arc;

impl Value {
    /// `IValue.equals`. The evaluator calls `lhs.tlc_equals(ev, rhs)` for `lhs = rhs`.
    pub(crate) fn tlc_equals(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<bool> {
        todo!()
    }

    /// `IValue.compareTo`.
    pub(crate) fn tlc_compare(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Ordering> {
        todo!()
    }

    /// `member(elem)`: `elem \in self`.
    pub(crate) fn member(&self, ev: &mut Ev<'_>, elem: &Value) -> EvalResult<bool> {
        todo!()
    }

    /// `size()` (Cardinality), with `TLC_MODULE_ARGUMENT_ERROR`-style text for infinite sets.
    pub(crate) fn size(&self, ev: &mut Ev<'_>) -> EvalResult<i32> {
        todo!()
    }
}

/// `SetEnumValue.normalize()` / `ValueVec.sort(true)`: binary insertion, dedup on
/// `compareTo == 0`. Publishes into `norm` on success only.
pub(crate) fn normalized<'a>(ev: &mut Ev<'_>, set: &'a SetEnumValue) -> EvalResult<&'a [Value]> {
    todo!()
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
