//! `tlc2.module.TLCEval`: the `@Evaluation` override of `TLC!TLCEval`.
//!
//! `TLCEval(e)` evaluates `e` eagerly and turns the lazy set and function classes into their
//! enumerated form, so that a repeated subexpression is computed once instead of once per
//! use. Being `@Evaluation` it receives the argument *node*, so it decides for itself
//! whether to evaluate at all, and every failure it lets out reaches the spec wrapped in
//! `TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE` (`modules::wrap_evaluating`).

use super::wrap_evaluating;
use crate::ctx::Ctx;
use crate::error::EvalResult;
use crate::eval::{Ev, EvalControl};
use crate::state::StateRef;
use crate::value::Value;
use tlars_sany::graph::{ExprId, ExprOrOpArg, Level};

/// `TLCEval.convert` (`TLCEval.java:48-60`), the legacy `TLC.TLCEval` body: the enumerated
/// set if there is one, else the function record if there is one, else the value itself. So
/// `1..3` becomes `{1, 2, 3}` and `<<1>>` becomes a `FcnRcdValue`, while `Nat` and `"s"`
/// come back untouched.
fn convert(ev: &mut Ev<'_>, eval: Value) -> EvalResult<Value> {
    if let Some(set) = eval.to_set_enum(ev)? {
        return Ok(Value::SetEnum(set));
    }
    if let Some(fcn) = eval.to_fcn_rcd(ev)? {
        return Ok(Value::FcnRcd(fcn));
    }
    Ok(eval)
}

/// `TLCEval.tlcEval` (`TLCEval.java:62-91`).
/// The `LazyFn` shape: an `@Evaluation`-style override that always answers, so never falls
/// back to the TLA+ definition.
pub(crate) fn tlc_eval(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    tlc_eval_value(ev, args, c, s0, s1, control).map(Some)
}

fn tlc_eval_value(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    let result = tlc_eval_impl(ev, args, c, s0, s1, control);
    result.map_err(|e| wrap_evaluating(ev, "TLC", "TLCEval", e))
}

fn tlc_eval_impl(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    let arg = args[0].expect_expr();
    // SANY's level of the node, not `getLevelBound`: a value that depends on a variable
    // cannot be cached on the node at all. A constant expression under a context (a bound
    // variable, an operator parameter) cannot either, because the node is shared by every
    // binding of it.
    if ev.tool().graph().level(arg) > Level::Constant || !c.is_deep_empty() {
        let value = ev.eval(arg, c, s0, s1, control)?;
        return convert(ev, value);
    }
    tlc_eval_const(ev, arg)
}

/// `TLCEval.tlcEvalConst`: evaluated once with an empty context and an empty state, stored
/// on the argument's node and returned unchanged ever after. `tool.eval(arg, c, s0, cm)`
/// passes `TLCState.Empty` as the successor state and `EvalControl.Clear`
/// (`Tool.java:1771-1773`), so the caller's control is deliberately dropped: the value may
/// not depend on it.
fn tlc_eval_const(ev: &mut Ev<'_>, arg: ExprId) -> EvalResult<Value> {
    if let Some(cached) = ev.tool().tlc_eval_values.lock().expect("TLCEval tool objects").get(&arg) {
        return Ok(cached.clone());
    }
    let value = ev.eval(arg, &Ctx::EMPTY, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)?;
    let value = convert(ev, value)?;
    // Every worker shares the stored value, and `convert` may have handed back one that is
    // not normalised.
    value.deep_normalize(ev)?;
    ev.tool().tlc_eval_values.lock().expect("TLCEval tool objects").insert(arg, value.clone());
    Ok(value)
}
