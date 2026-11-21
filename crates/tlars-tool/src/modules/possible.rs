//! `tlc2.module._Possible`, the Java side of `_Possible.tla`.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::value::Value;

/// `_Possible.counts()` (`_Possible.java:53-83`): the `s:_possible` register of every worker,
/// merged by name. With no checker and no simulator there are no worker registers to read, so
/// the result is the empty record; `minLevel = VariableLevel` is what stops TLC folding this
/// into a constant at startup, when that is the only answer it could give.
pub(crate) fn counts(ev: &mut Ev<'_>, _a: &[Value]) -> EvalResult<Value> {
    let counts = if ev.tool().main_checker().is_some() { ev.possible_counts()? } else { Vec::new() };
    Ok(crate::eval::possible_record(counts))
}
