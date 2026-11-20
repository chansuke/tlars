//! `tlc2.module.TLCGetSet`: the Java override of `TLC!TLCGet` and `TLC!TLCSet`.
//!
//! `TLCGet` is an `@Evaluation` override, so it gets the argument node and evaluates it
//! itself, and every failure it raises reaches the spec wrapped in
//! `TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE`. `TLCSet` is an ordinary `@TLAPlusOperator`, whose
//! `EvalException` passes through unwrapped.

use super::{eval_exception, wrap_evaluating};
use crate::ctx::Ctx;
use crate::error::{EvalError, EvalResult};
use crate::eval::{EvalControl, Ev};
use crate::state::StateRef;
use crate::value::Value;
use tlars_msg::Ec;
use tlars_sany::Atom;
use tlars_sany::graph::ExprOrOpArg;

/// `TLCGetSet.NAMED_REGISTER_PREFIX`: a string key with this prefix names a register rather
/// than one of the keys below.
const NAMED_REGISTER_PREFIX: &str = "s:";

/// The string keys whose value comes from the running checker. tlars does not hand the
/// evaluator the checker's counters, configuration or spec objects yet.
const FROM_CHECKER: &[&str] = &[
    "diameter",
    "generated",
    "distinct",
    "queue",
    "duration",
    "stats",
    "config",
    "spec",
    "revision",
    "action",
    "all",
    "all:named",
];

/// `TLCGetSet.TLCGetEval` (`TLCGetSet.java:160-185`).
pub(crate) fn tlc_get(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    let result = tlc_get_eval(ev, args, c, s0, s1, control);
    result.map_err(|e| wrap_evaluating(ev, "TLC", "TLCGet", e))
}

fn tlc_get_eval(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    let vidx = ev.eval(args[0].expect_expr(), c, s0, s1, control)?;
    // A negative index falls through to the argument error, as the inner `if` does in Java.
    if let Value::Int(i) = vidx
        && i >= 0
    {
        let stored = ev.tool().registers.lock().expect("registers").get(i as usize);
        return match stored {
            Some(v) => Ok(v),
            None => Err(eval_exception(ev, Ec::TLC_MODULE_TLCGET_UNDEFINED, &[&i.to_string()])),
        };
    }
    if let Value::Str(name) = vidx {
        return tlc_get_string(ev, name, s0, control);
    }
    let shown = vidx.ppr(ev)?;
    Err(eval_exception(ev, Ec::TLC_MODULE_ONE_ARGUMENT_ERROR, &["TLCGet", "nonnegative integer", &shown]))
}

/// `TLCGetStringValue` (`TLCGetSet.java:187-461`).
fn tlc_get_string(ev: &mut Ev<'_>, name: Atom, s0: StateRef<'_>, control: EvalControl) -> EvalResult<Value> {
    let key = ev.tool().graph().interner().text(name).to_string();
    if key == "level" {
        // `level` is the height stored in the state being worked on, and is 0 by definition
        // in ASSUME, POSTCONDITION and the initial predicate (`:379-392`).
        if control.has(EvalControl::CONST) || control.has(EvalControl::INIT) {
            return Ok(Value::Int(crate::state::INIT_LEVEL as i32 - 1));
        }
        return Ok(Value::Int(s0.level() as i32));
    }
    if key.starts_with(NAMED_REGISTER_PREFIX) && key != "all:named" {
        let stored = ev.tool().registers.lock().expect("registers").get_named(name);
        return match stored {
            Some(v) => Ok(v),
            None => Err(eval_exception(ev, Ec::TLC_MODULE_TLCGET_UNDEFINED, &[&key])),
        };
    }
    if FROM_CHECKER.contains(&key.as_str()) {
        return Err(EvalError::unimplemented(format!("TLCGet(\"{key}\") (M3 unit 5)")));
    }
    if key.starts_with("-D") {
        return Err(EvalError::unimplemented(format!("TLCGet(\"{key}\"), which reads a Java system property (M3 unit 5)")));
    }
    Err(eval_exception(ev, Ec::TLC_MODULE_TLCGET_UNDEFINED, &[&key]))
}

/// `TLCGetSet.TLCSet` (`TLCGetSet.java:463-535`).
pub(crate) fn tlc_set(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if let Value::Int(i) = a[0]
        && i >= 0
    {
        ev.tool().registers.lock().expect("registers").set(i as usize, a[1].clone());
        return Ok(Value::Bool(true));
    }
    if let Value::Str(name) = a[0] {
        let key = ev.tool().graph().interner().text(name).to_string();
        // Every string key answers TRUE; only a TRUE second argument does anything.
        let asked = matches!(a[1], Value::Bool(true));
        match key.as_str() {
            "exit" => {
                if asked {
                    return Err(EvalError::unimplemented("TLCSet(\"exit\", TRUE), which stops the checker (M3 unit 5)".into()));
                }
                return Ok(Value::Bool(true));
            }
            "pause" => {
                if asked {
                    return Err(EvalError::unimplemented("TLCSet(\"pause\", TRUE), which waits on standard input (M3 unit 5)".into()));
                }
                return Ok(Value::Bool(true));
            }
            _ if key.starts_with(NAMED_REGISTER_PREFIX) => {
                ev.tool().registers.lock().expect("registers").set_named(name, a[1].clone());
                return Ok(Value::Bool(true));
            }
            _ if key.starts_with("-D") => {
                return Err(EvalError::unimplemented(format!(
                    "TLCSet(\"{key}\", ...), which sets a Java system property (M3 unit 5)"
                )));
            }
            _ => {}
        }
    }
    let shown = a[0].ppr(ev)?;
    Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["first", "TLCSet", "nonnegative integer", &shown]))
}
