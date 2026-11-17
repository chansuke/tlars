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

use super::Value;
use crate::eval::Ev;
use crate::error::EvalResult;

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

enum EnumState {
    Slice { elems: std::sync::Arc<super::SetEnumValue>, next: usize },
    Interval { next: i64, hi: i64 },
    Odometer { /* per-component enumerations, last fastest */ },
    Concat { /* SetCup / Union parts */ },
    Filter { /* SetPred / SetCap / SetDiff */ },
}

impl ValueEnumeration {
    pub(crate) fn next(&mut self, ev: &mut Ev<'_>) -> EvalResult<Option<Value>> {
        todo!()
    }
}

impl Value {
    /// `elements(ordering)`; `Ok(None)` if the value is not enumerable (caller reports
    /// `getNonEnumerableErrorMsg`).
    pub(crate) fn elements(&self, ev: &mut Ev<'_>, order: EnumOrder) -> EvalResult<Option<ValueEnumeration>> {
        todo!()
    }
}
