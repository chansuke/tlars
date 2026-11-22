//! `tlc2.module.TLCExt`, the Java side of `TLCExt.tla`.
//!
//! The classic `@TLAPlusOperator` methods are here as strict natives; the `@Evaluation`
//! methods, which see their arguments unevaluated, are lazy ones.

use super::{eval_exception, wrap_evaluating};
use crate::ctx::Ctx;
use crate::error::{EvalError, EvalErrorKind, EvalResult};
use crate::eval::{EvalControl, Ev};
use crate::state::StateRef;
use crate::value::Value;
use tlars_msg::Ec;
use tlars_sany::graph::{ExprNode, ExprOrOpArg};

/// `TLCGetSet.NAMED_REGISTER_PREFIX`.
const NAMED_REGISTER_PREFIX: &str = "s:";

/// `TLCExt.assertError(err, exp)` (`TLCExt.java:76-94`): TRUE when evaluating `exp` failed
/// with exactly the message `err`. The failure's message is what `getMessage()` returns, so
/// under `-tool` it carries the message framing and an exact text no longer matches.
pub(crate) fn assert_error(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    let result = assert_error_impl(ev, args, c, s0, s1, control);
    result.map(Some).map_err(|e| wrap_evaluating(ev, "TLCExt", "AssertError", e))
}

fn assert_error_impl(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    // `Assert.check(args[0] instanceof StringNode, ..)`: a check on the *node*, so a
    // definition or an expression that evaluates to a string fails it too. The reason names
    // the node as `SemanticNode.toString()` prints it.
    let err_node = args[0].expect_expr();
    if !matches!(ev.tool().graph().expr(err_node), ExprNode::String { .. }) {
        let shown = ev.tool().graph().expr_tlc_text(err_node);
        let reason =
            format!("In computing AssertError, a non-string expression ({shown}) was used as the err of an AssertError(err, exp).");
        return Err(EvalError::detailed(reason, None));
    }
    match ev.eval(args[1].expect_expr(), c, s0, s1, control) {
        Ok(_) => Ok(Value::Bool(false)),
        // `catch (EvalException | TLCRuntimeException e)`: a fingerprint failure, a stack
        // overflow and an unchecked JVM exception all travel on.
        Err(e) if matches!(e.kind, EvalErrorKind::Eval | EvalErrorKind::Runtime) => {
            // `tool.eval(args[0], c, s0)`: the three-argument overload, so no successor state
            // and `EvalControl.Clear`.
            let err = ev.eval(err_node, c, s0, StateRef::Empty, EvalControl::CLEAR)?;
            // `(StringValue) ..`, which the `instanceof StringNode` check above guarantees.
            let Value::Str(text) = err else {
                return Err(EvalError::class_cast(err.java_class(), "StringValue"));
            };
            let text = ev.tool().graph().interner().text(text).to_string();
            Ok(Value::Bool(text == e.message))
        }
        Err(e) => Err(e),
    }
}

/// `TLCExt.tlcDefByName(definitionName)` (`TLCExt.java:390-423`): the body of the root
/// module's definition of that name, evaluated in the scope the application stands in. It
/// evaluates the *body*, so a definition that has a Java override is evaluated from its TLA+
/// text instead — which is why `TLCEvalDefinition("Nat")` is the `{}` that `Naturals.tla`
/// writes rather than the set of naturals.
pub(crate) fn tlc_def_by_name(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    let result = tlc_def_by_name_impl(ev, args, c, s0, s1, control);
    result.map(Some).map_err(|e| wrap_evaluating(ev, "TLCExt", "TLCEvalDefinition", e))
}

fn tlc_def_by_name_impl(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    let v = ev.eval(args[0].expect_expr(), c, s0, s1, control)?;
    let argument_error = |ev: &mut Ev<'_>, wanted: &str, shown: &str| {
        eval_exception(ev, Ec::TLC_MODULE_ONE_ARGUMENT_ERROR, &["TLCEvalDefinition", wanted, shown])
    };
    let Value::Str(name) = v else {
        let shown = v.ppr(ev)?;
        return Err(argument_error(ev, "string", &shown));
    };
    let graph = ev.tool().graph().clone();
    let Some(def) = graph.module_op_def(graph.root(), name) else {
        let shown = v.ppr(ev)?;
        // The template adds no article, so TLC's own `"name of a definition .."` reads as
        // written and `"a zero-arity definition"` below comes out as `a a zero-arity`.
        return Err(argument_error(ev, "name of a definition reachable from the root module", &shown));
    };
    if !graph.op_def(def).params.is_empty() {
        let signature = graph.signature(def);
        return Err(argument_error(ev, "a zero-arity definition", &signature));
    }
    let body = graph.op_def(def).body.expect("a definition of the module context has a body");
    ev.eval(body, c, s0, s1, control)
}

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
