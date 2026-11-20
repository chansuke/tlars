//! `tlc2.module.TLC`.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::value::Value;
use tlars_msg::Ec;

/// `deepCopy`, `deepNormalize`, then `Values.ppr(toStringUnchecked())`.
fn shown(ev: &mut Ev<'_>, v: &Value) -> EvalResult<String> {
    v.deep_normalize(ev)?;
    v.ppr(ev)
}

/// Prints `v1  v2` to `ToolIO.out` and returns `v2` (`TLC.java:63-78`).
pub(crate) fn print(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let line = format!("{}  {}", shown(ev, &a[0])?, shown(ev, &a[1])?);
    ev.out().raw(&format!("{line}\n"));
    Ok(a[1].clone())
}

/// Prints `v1` to `ToolIO.out` and returns TRUE (`TLC.java:86-101`).
pub(crate) fn print_t(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let line = shown(ev, &a[0])?;
    ev.out().raw(&format!("{line}\n"));
    Ok(Value::Bool(true))
}

/// `new StringValue(v.toStringUnchecked())`.
pub(crate) fn to_string(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let text = a[0].to_tlc_string(ev)?;
    Ok(Value::Str(ev.tool().graph().interner().intern(&text)))
}

pub(crate) fn assert(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if matches!(a[0], Value::Bool(true)) {
        return Ok(a[0].clone());
    }
    let shown = a[1].ppr(ev)?;
    Err(super::eval_exception(ev, Ec::TLC_VALUE_ASSERT_FAILED, &[&shown]))
}
