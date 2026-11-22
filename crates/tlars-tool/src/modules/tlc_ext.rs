//! `tlc2.module.TLCExt`, the Java side of `TLCExt.tla`.
//!
//! The classic `@TLAPlusOperator` methods are here as strict natives; the `@Evaluation`
//! methods, which see their arguments unevaluated, are lazy ones.

use super::eval_exception;
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use crate::value::Value;
use tlars_msg::Ec;

/// `TLCGetSet.NAMED_REGISTER_PREFIX`.
const NAMED_REGISTER_PREFIX: &str = "s:";

/// `TLCExt.tlcNoOp(val)` (`TLCExt.java:303-306`): the value itself. It exists so a Java
/// breakpoint can be set inside TLC's evaluator.
pub(crate) fn tlc_no_op(_ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    Ok(a[0].clone())
}

/// `TLCExt.tlcFingerprint(val)` (`:382-388`): the value's fingerprint under the run's
/// polynomial, narrowed to an int by `FP64.Hash`, which is a plain `(int) fp` cast.
pub(crate) fn tlc_fingerprint(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    a[0].deep_normalize(ev)?;
    let fp = a[0].fingerprint(ev, ev.tool().fp64.new_fp())?;
    Ok(Value::Int(fp.raw() as i32))
}

/// `TLCExt.tlcModelValue(str)` (`:308-317`): `ModelValue.add(str)`, which hands back the
/// model value of that name and creates it when there is none. The argument error names
/// `ModelValue`, not the TLA+ operator.
pub(crate) fn tlc_model_value(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Value::Str(name) = a[0] else {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ONE_ARGUMENT_ERROR, &["ModelValue", "string", &shown]));
    };
    let text = ev.tool().graph().interner().text(name).to_string();
    match ev.tool().add_model_value(name, &text) {
        Some(mv) => Ok(Value::Model(mv)),
        // `Assert.fail(EC.TLC_FEATURE_UNSUPPORTED, ...)` from `ModelValue`'s constructor.
        None => {
            let what = format!("TLC supports at most {} model values, but model value {text} exceeds this limit.", i16::MAX as i32 + 1);
            let message = ev.out().text(Ec::TLC_FEATURE_UNSUPPORTED, &[&what]);
            Err(EvalError::runtime(Ec::TLC_FEATURE_UNSUPPORTED, message))
        }
    }
}

/// `TLCExt.tlcGetOrDefault(vidx, defVal)` (`:425-460`), a copy of `TLCGetSet`'s register
/// reads with every failure replaced by the default: a non-negative or negative integer
/// index, a `"s:"`-prefixed name, or anything else all answer `defVal` when there is nothing
/// stored. The registers live in the worker threads and in the checker, so with neither —
/// during `processConstantDefns` — every read answers the default.
pub(crate) fn tlc_get_or_default(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let default = a[1].clone();
    if ev.tool().main_checker().is_none() {
        return Ok(default);
    }
    let stored = match a[0] {
        // `localValues` is a `ConcurrentHashMap<Integer, IValue>`, so a negative index is
        // simply a key nothing was ever stored under.
        Value::Int(i) => ev.tool().registers.lock().expect("registers").get_i32(i),
        Value::Str(name) => {
            let key = ev.tool().graph().interner().text(name).to_string();
            if !key.starts_with(NAMED_REGISTER_PREFIX) {
                return Ok(default);
            }
            ev.tool().registers.lock().expect("registers").get_named(name)
        }
        _ => None,
    };
    Ok(stored.unwrap_or(default))
}
