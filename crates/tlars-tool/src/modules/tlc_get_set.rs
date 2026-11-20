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

/// The string keys whose value tlars cannot build yet: the run's configuration, its spec
/// objects, the per-worker register maps, and the version record.
const UNIMPLEMENTED: &[&str] = &["config", "spec", "revision", "action", "all", "all:named"];

/// `getStatistics()`' fields, in the order `AbstractChecker` fills them
/// (`AbstractChecker.java:683-709`). The record prints them in intern-token order, which is
/// `TLCGetSet`'s declaration order, not this one.
const STATS_FIELDS: &[&str] = &["queue", "distinct", "initial", "generated", "diameter", "duration", "worker"];

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
    if UNIMPLEMENTED.contains(&key.as_str()) {
        return Err(EvalError::unimplemented(format!("TLCGet(\"{key}\") (M3 unit 5)")));
    }
    // Every counter below is a `TLCGlobals.mainChecker` call, so without a checker the NPE
    // TLC catches turns into `TLCGet(key) was undefined`.
    if matches!(key.as_str(), "diameter" | "generated" | "distinct" | "queue" | "stats") && ev.tool().run_counters().is_none() {
        return Err(eval_exception(ev, Ec::TLC_MODULE_TLCGET_UNDEFINED, &[&key]));
    }
    match key.as_str() {
        // `IntValue.gen(getProgress())`, already an int in Java.
        "diameter" => return Ok(Value::Int(ev.tool().run_counters().expect("checked above").diameter() as i32)),
        "generated" => return narrow(ev, ev.tool().run_counters().expect("checked above").generated()),
        "distinct" => return narrow(ev, ev.tool().run_counters().expect("checked above").distinct()),
        "queue" => return narrow(ev, ev.tool().run_counters().expect("checked above").queue()),
        // `duration` reads the clock alone and needs no checker, but tlars has no clock
        // outside a run.
        "duration" => {
            let Some(counters) = ev.tool().run_counters() else {
                return Err(EvalError::unimplemented("TLCGet(\"duration\") outside a run (M3 unit 5)".into()));
            };
            return narrow(ev, counters.duration_secs().max(0) as u64);
        }
        "stats" => return statistics(ev),
        _ => {}
    }
    // `System.getProperty(key.substring(2), key)`: the default is the whole key, `-D` and all.
    if let Some(property) = key.strip_prefix("-D") {
        let text = ev.tool().property(property).unwrap_or_else(|| key.clone());
        return Ok(Value::Str(ev.tool().graph().interner().intern(&text)));
    }
    Err(eval_exception(ev, Ec::TLC_MODULE_TLCGET_UNDEFINED, &[&key]))
}

/// `IntValue.gen(Math.toIntExact(v))`: a counter past `Integer.MAX_VALUE` is an
/// `ArithmeticException`, which the module reports as `TLC_MODULE_OVERFLOW`.
fn narrow(ev: &mut Ev<'_>, v: u64) -> EvalResult<Value> {
    match i32::try_from(v) {
        Ok(n) => Ok(Value::Int(n)),
        Err(_) => Err(eval_exception(ev, Ec::TLC_MODULE_OVERFLOW, &[&v.to_string()])),
    }
}

/// `IntValue.narrowToIntValue(v)`: a value that does not fit becomes -1.
fn narrow_to_int(v: u64) -> Value {
    Value::Int(i32::try_from(v).unwrap_or(-1))
}

/// `AbstractChecker.getStatistics()` (`:683-709`).
fn statistics(ev: &mut Ev<'_>) -> EvalResult<Value> {
    let counters = ev.tool().run_counters().expect("checked by the caller");
    let worker = ev.worker_id();
    let values = vec![
        narrow_to_int(counters.queue()),
        narrow_to_int(counters.distinct()),
        narrow_to_int(counters.initial()),
        narrow_to_int(counters.generated()),
        narrow_to_int(u64::from(counters.diameter())),
        narrow_to_int(counters.duration_secs().max(0) as u64),
        Value::Int(i32::from(worker)),
    ];
    let interner = ev.tool().graph().interner();
    let names = STATS_FIELDS.iter().map(|n| interner.intern(n)).collect();
    Ok(Value::Record(Value::record_arc(names, values, false)))
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
            // `System.setProperty(key.substring(2), val.toString())`, so a string value
            // keeps the quotes its `toString` prints.
            _ if key.starts_with("-D") => {
                let text = a[1].to_tlc_string(ev)?;
                ev.tool().set_property(&key["-D".len()..], text);
                return Ok(Value::Bool(true));
            }
            _ => {}
        }
    }
    let shown = a[0].ppr(ev)?;
    Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["first", "TLCSet", "nonnegative integer", &shown]))
}
