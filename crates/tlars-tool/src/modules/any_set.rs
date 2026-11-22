//! `tlc2.module.AnySet`: the one `UserValue(new AnySet())` behind `AnySet!ANY` and
//! `TLC!Any`, since `TLC.Any()` is `return AnySet.ANY()`.
//!
//! The `UserObj` answers only four questions — `member` is true for *every* value,
//! `isFinite` is false, `toString` is `ANY` and `compareTo` always raises
//! `TLC_MODULE_COMPARE_VALUE` — and leaves the rest to `UserValue` and `Value`. In
//! particular `isEmpty` is deliberately not overridden, so `[ANY -> {}]` reaches
//! `UserObj.isEmpty` and TLC refuses to say whether `ANY` is empty. Those answers live on
//! [`crate::value::UserValue::AnySet`] in the value layer, next to `Nat`, `Int`, `STRING`
//! and `Seq(S)`.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::value::{UserValue, Value};
use std::sync::Arc;

/// `AnySet.ANY()` (`AnySet.java:22-25`), which `TLC.Any()` (`TLC.java:440-443`) returns too.
/// TLC keeps one instance in a static field, but nothing can observe the identity:
/// `compareTo` throws before any equality on it could matter, and the value is immutable.
pub(crate) fn any(_: &mut Ev<'_>, _: &[Value]) -> EvalResult<Value> {
    Ok(Value::User(Arc::new(UserValue::AnySet)))
}
