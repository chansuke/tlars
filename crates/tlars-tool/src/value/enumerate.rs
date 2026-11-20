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
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;

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
    Interval { lo: i32, next: i64, hi: i64 },
}

impl ValueEnumeration {
    pub(crate) fn next(&mut self, ev: &mut Ev<'_>) -> EvalResult<Option<Value>> {
        match &mut self.state {
            EnumState::Interval { next, hi, .. } => {
                if *next > *hi {
                    return Ok(None);
                }
                let v = Value::Int(*next as i32);
                *next += 1;
                Ok(Some(v))
            }
        }
    }

    /// `reset()`: start over, as the odometer of `ContextEnumerator` does.
    pub(crate) fn reset(&mut self) {
        match &mut self.state {
            EnumState::Interval { lo, next, .. } => *next = i64::from(*lo),
        }
    }
}

impl Value {
    /// `elements(ordering)`; `Ok(None)` if the value is not enumerable (caller reports
    /// `getNonEnumerableErrorMsg`).
    pub(crate) fn elements(&self, ev: &mut Ev<'_>, order: EnumOrder) -> EvalResult<Option<ValueEnumeration>> {
        match self {
            Value::Interval(i) => {
                Ok(Some(ValueEnumeration { state: EnumState::Interval { lo: i.lo, next: i64::from(i.lo), hi: i64::from(i.hi) } }))
            }
            Value::Bool(_) | Value::Int(_) | Value::Str(_) | Value::Model(_) | Value::User(_) | Value::Undef => Ok(None),
            _ => Err(EvalError::unimplemented(format!("enumerating {} (M2 unit 3)", self.kind_string()))),
        }
    }

    /// `getNonEnumerableErrorMsg(domain)` (`Value.java:447-450`, `Naturals.java:275`,
    /// `Integers.java`).
    pub(crate) fn non_enumerable_message(&self, ev: &mut Ev<'_>, domain_text: &str) -> EvalResult<String> {
        let shown = self.ppr(ev)?;
        let set = match self {
            Value::User(u) => match **u {
                super::UserValue::Nat => Some("Nat"),
                super::UserValue::Int => Some("Int"),
                super::UserValue::String => None,
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
