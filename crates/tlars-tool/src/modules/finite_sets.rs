//! `tlc2.module.FiniteSets`.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::modules::eval_exception;
use crate::value::Value;
use tlars_msg::Ec;

pub(crate) fn is_finite_set(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    Ok(Value::Bool(a[0].is_finite(ev)?))
}

/// `Cardinality(S)`: the class decides, so a non-set raises 2181 while an infinite set
/// reaches `size()` and fails there (`FiniteSets.java:27-34`).
pub(crate) fn cardinality(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if !a[0].is_enumerable() {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_COMPUTING_CARDINALITY, &[&shown]));
    }
    Ok(Value::Int(a[0].size(ev)?))
}
