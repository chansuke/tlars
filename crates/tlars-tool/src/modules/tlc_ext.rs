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

/// `CounterExample`'s field names (`CounterExample.java:48-50`), in the order its
/// constructor writes them: the edges, the states, and `console`, which is only there when
/// the counterexample was not the one printed to the console.
const ACTIONS: &str = "action";
const STATES: &str = "state";
const CONSOLE: &str = "console";

/// `new CounterExample()` (`CounterExample.java:52-54`): the empty graph, which is what
/// `TLCExt!CounterExample` answers wherever no counterexample is bound.
pub(crate) fn empty_counter_example(ev: &Ev<'_>) -> Value {
    counter_example_record(ev, Value::set_enum_of(Vec::new(), false), Value::set_enum_of(Vec::new(), false), true)
}

/// One state of a counterexample as `CounterExample`'s constructor reads it: the state
/// number `TLCStateInfo` took from the state's level, the record `toRecordValue()` builds,
/// and the action that produced it (`None` = `Action.UNKNOWN`, which is what an initial
/// state reports).
pub struct CounterExampleStep {
    pub number: i32,
    pub record: Value,
    pub action: Option<crate::actions::ActionId>,
}

/// `new CounterExample(trace, isConsole)` (`CounterExample.java:77-115`) for a safety
/// violation, which has no loop ordinal: the states as `<<number, record>>` pairs and one
/// edge `<<source, action, target>>` per step after the first, both as unnormalised sets in
/// trace order.
pub fn counter_example_of(ev: &mut Ev<'_>, trace: &[CounterExampleStep], is_console: bool) -> EvalResult<Value> {
    let mut states: Vec<Value> = Vec::with_capacity(trace.len());
    let mut actions: Vec<Value> = Vec::new();
    for step in trace {
        let pair = Value::tuple(vec![Value::Int(step.number), step.record.clone()]);
        if let Some(last) = states.last() {
            // `new RecordValue(info.getAction())`, which in breadth-first search is the
            // action whose replay produced this state.
            let action = match step.action {
                Some(id) => {
                    let a = ev.tool().action(id);
                    super::tlc_get_set::action_value(ev, a)?
                }
                None => super::tlc_get_set::unknown_action_value(ev)?,
            };
            actions.push(Value::tuple(vec![last.clone(), action, pair.clone()]));
        }
        states.push(pair);
    }
    let actions = Value::set_enum_of(actions, false);
    let states = Value::set_enum_of(states, false);
    Ok(counter_example_record(ev, actions, states, is_console))
}

/// `CounterExample`'s `super(..)` call: the fields in constructor order, unnormalised.
pub(crate) fn counter_example_record(ev: &Ev<'_>, actions: Value, states: Value, is_console: bool) -> Value {
    let interner = ev.tool().graph().interner();
    interner.ensure(tlars_sany::JavaClass::CounterExample);
    let mut names = vec![interner.intern(ACTIONS), interner.intern(STATES)];
    let mut values = vec![actions, states];
    if !is_console {
        names.push(interner.intern(CONSOLE));
        values.push(Value::Bool(false));
    }
    Value::Record(Value::counter_example_arc(names, values))
}

/// `TLCExt.error(..)` (`TLCExt.java:197-205`), the `@Evaluation` override of
/// `CounterExample`: the counterexample the postcondition check bound in the context, or the
/// empty graph. The binding is on the `OpDefNode` of `TLCExt!CounterExample` itself, so an
/// application that names that definition is answered by the context before this method is
/// reached; this is the path an application through an INSTANCE takes.
pub(crate) fn counter_example(
    ev: &mut Ev<'_>,
    _args: &[ExprOrOpArg],
    c: &Ctx,
    _s0: StateRef<'_>,
    _s1: StateRef<'_>,
    _control: EvalControl,
) -> EvalResult<Option<Value>> {
    if let Some(def) = ev.tool().counter_example_def()
        && let Some(crate::ctx::Binding::Value(v)) = c.lookup(tlars_sany::graph::SymbolRef::OpDef(def))
    {
        return Ok(Some(v.clone()));
    }
    Ok(Some(empty_counter_example(ev)))
}

/// `TLCExt.getTrace(..)` (`TLCExt.java:207-290`), the override of `Trace`: the behaviour
/// from an initial state up to and including `s0`, one record of the variables per state.
/// The ALIAS binds the operator in the context instead (`Tool.evalAlias`), so this is the
/// path every other scope takes.
pub(crate) fn get_trace(
    ev: &mut Ev<'_>,
    _args: &[ExprOrOpArg],
    _c: &Ctx,
    s0: StateRef<'_>,
    _s1: StateRef<'_>,
    _control: EvalControl,
) -> EvalResult<Option<Value>> {
    let result = get_trace_impl(ev, s0);
    result.map(Some).map_err(|e| wrap_evaluating(ev, "TLCExt", "Trace", e))
}

fn get_trace_impl(ev: &mut Ev<'_>, s0: StateRef<'_>) -> EvalResult<Value> {
    // `Assert.fail(EC.GENERAL, ..)` while a variable is still unassigned, which is the state
    // a POSTCONDITION and an initial predicate before its last conjunct are evaluated in.
    if !ev.all_assigned(s0) {
        let unassigned = ev.unassigned_of(s0);
        let plural = if unassigned.len() > 1 { "s" } else { "" };
        let what = format!(
            "In evaluating TLCExt!Trace, the state is not completely specified yet (variable{plural} {} undefined).",
            unassigned.join(", ")
        );
        let message = ev.out().text(Ec::GENERAL, &[&what]);
        return Err(EvalError::runtime(Ec::GENERAL, message));
    }
    // `TLCGlobals.simulator != null` hands back the behaviour the simulator is on; tlars has
    // no simulator before M5, so that branch is never taken (`simulate::simulate`).
    // `s0.isInitial()`: nothing precedes it.
    if s0.level() == crate::state::INIT_LEVEL {
        let record = ev.trace_record(s0)?;
        return Ok(Value::tuple(vec![record]));
    }
    let StateRef::Full(state) = s0 else {
        // A level above the first with a variable still unbound cannot happen: the check
        // above rejected it.
        return Err(EvalError::unimplemented("TLCExt!Trace over a state that is not a complete successor (M3 unit 6)".into()));
    };
    match state.uid() {
        // `getTrace0(s0, getTraceInfo(s0))`: the behaviour that ends at `s0`'s predecessor,
        // then `s0` itself.
        Some(uid) => {
            let mut records = prefix_records(ev, uid)?;
            records.push(ev.trace_record(s0)?);
            Ok(Value::tuple(records))
        }
        // `s0.uid == TLCState.INIT_UID`: `s0` has no trace record, so the behaviour is
        // rebuilt up to the state the worker is expanding and `s0` appended. TLC restores
        // `IdThread.setCurrentState` afterwards, because the rebuild moved it.
        None => {
            let saved = ev.saved_expansion();
            let Some(current) = saved.as_ref().map(|c| c.state.clone()) else {
                // `IdThread.getCurrentState()` is null outside a worker's expansion, which
                // TLC dereferences.
                return Err(EvalError::unimplemented(
                    "TLCExt!Trace over a state with no trace record outside a worker's expansion (TLC raises a NullPointerException)"
                        .into(),
                ));
            };
            let mut records = if current.is_initial() {
                vec![ev.trace_record(StateRef::Full(&current))?]
            } else {
                let Some(uid) = current.uid() else {
                    return Err(EvalError::unimplemented(
                        "TLCExt!Trace over a state the trace store has no record of (M3 unit 6)".into(),
                    ));
                };
                let mut records = prefix_records(ev, uid)?;
                records.push(ev.trace_record(StateRef::Full(&current))?);
                ev.restore_expansion(saved);
                records
            };
            records.push(ev.trace_record(s0)?);
            Ok(Value::tuple(records))
        }
    }
}

/// `TLCGlobals.mainChecker.getTraceInfo(s)`: the states before the one the trace record
/// `uid` names, each as `new RecordValue(state)`.
fn prefix_records(ev: &mut Ev<'_>, uid: crate::state::TraceUid) -> EvalResult<Vec<Value>> {
    let Some(checker) = ev.tool().main_checker() else {
        return Err(EvalError::unimplemented("TLCExt!Trace with no checker running (TLC raises a NullPointerException)".into()));
    };
    let fps = checker.trace_fingerprints(uid);
    // `records.size() - 1`: the record's own fingerprint is dropped, so the reconstruction
    // stops at its predecessor.
    let Some(prefix) = fps.split_last().map(|(_, rest)| rest.to_vec()) else { return Ok(Vec::new()) };
    let Some(steps) = ev.reconstruct_states(&prefix)? else {
        return Err(EvalError::unimplemented("TLCExt!Trace over a behaviour TLC cannot regenerate (M3 unit 6)".into()));
    };
    let mut records = Vec::with_capacity(steps.len());
    for step in &steps {
        let state = step.state.clone();
        records.push(ev.trace_record(StateRef::Full(&state))?);
    }
    Ok(records)
}

/// `TLCExt.lassoOrdinal(val)` (`TLCExt.java:186-193`), the override of `ToTrace`: the
/// counterexample's states as a sequence indexed by state number. A record of the same shape
/// is refused, because the check is on the value's Java class.
pub(crate) fn to_trace(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Value::Record(rcd) = &a[0] else {
        return Err(not_a_counter_example(ev, &a[0]));
    };
    if !rcd.counter_example {
        return Err(not_a_counter_example(ev, &a[0]));
    }
    // `(SetEnumValue) this.select(new StringValue(STATES))`, a linear field scan that
    // normalises nothing, so the states come out in the order the counterexample built them.
    let name = Value::Str(ev.tool().graph().interner().intern(STATES));
    let states = a[0].select(ev, &name)?.expect("a counterexample has a state field");
    let Value::SetEnum(set) = &states else {
        return Err(EvalError::class_cast(states.java_class(), "SetEnumValue"));
    };
    let elems = set.snapshot().to_vec();
    let mut trace = vec![None; elems.len()];
    for elem in &elems {
        let Value::Tuple(pair) = elem else {
            return Err(EvalError::class_cast(elem.java_class(), "TupleValue"));
        };
        let Some(Value::Int(number)) = pair.elems.first() else {
            let class = pair.elems.first().map_or("UndefValue", Value::java_class);
            return Err(EvalError::class_cast(class, "IntValue"));
        };
        // `v[number - 1] = ..`: a number outside the array is an index error, and a gap in
        // the numbering leaves a Java null in the tuple.
        let Some(slot) = usize::try_from(*number - 1).ok().and_then(|i| trace.get_mut(i)) else {
            return Err(EvalError::index_out_of_bounds((*number - 1) as usize, elems.len()));
        };
        *slot = pair.elems.get(1).cloned();
    }
    let mut out = Vec::with_capacity(trace.len());
    for slot in trace {
        match slot {
            Some(v) => out.push(v),
            // A tuple element holding a Java null, which every later read treats as its own
            // kind of value.
            None => {
                return Err(EvalError::unimplemented(
                    "ToTrace over a counterexample whose state numbers leave a gap (M4)".into(),
                ));
            }
        }
    }
    Ok(Value::tuple(out))
}

fn not_a_counter_example(ev: &mut Ev<'_>, v: &Value) -> EvalError {
    match v.ppr(ev) {
        Ok(shown) => eval_exception(ev, Ec::TLC_MODULE_ONE_ARGUMENT_ERROR, &["ToTrace", "CounterExample", &shown]),
        Err(e) => e,
    }
}

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
    let graph = ev.tool().graph();
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

/// `TLCExt.tlcDefer(expression)` (`TLCExt.java:292-301`): the arguments are wrapped in a
/// `Callable` and handed to both states, and only the simulator's `TLCStateMutExt` keeps one
/// (`TLCState.setCallable` is a no-op, `TLCState.java:194-196`). So in breadth-first search
/// nothing is evaluated and the operator is simply TRUE.
pub(crate) fn tlc_defer(
    _ev: &mut Ev<'_>,
    _args: &[ExprOrOpArg],
    _c: &Ctx,
    _s0: StateRef<'_>,
    _s1: StateRef<'_>,
    _control: EvalControl,
) -> EvalResult<Option<Value>> {
    Ok(Some(Value::Bool(true)))
}

/// `TLCExt.tlcEval2(expression, closure)` (`TLCExt.java:321-380`), the override of
/// `TLCCache`.
///
/// A constant-level expression is cached per expression node under the closure's value, so
/// the same closure answers with the first value even when the expression reads a bound
/// variable the closure does not mention. A state-level one goes through
/// `TLCState.getCached`/`setCached`, which only the simulator's `TLCStateMutExt` implements:
/// on a plain state `setCached` answers null, so the expression is evaluated here *and*
/// again by the TLA+ fallback. An action- or temporal-level one is only evaluated by the
/// fallback.
pub(crate) fn tlc_eval2(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    let result = tlc_eval2_impl(ev, args, c, s0, s1, control);
    result.map_err(|e| wrap_evaluating(ev, "TLCExt", "TLCCache", e))
}

fn tlc_eval2_impl(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    let (expr, closure) = (args[0].expect_expr(), args[1].expect_expr());
    match ev.tool().graph().level(expr) {
        tlars_sany::graph::Level::Constant => {
            let key = ev.eval(closure, c, s0, s1, control)?;
            // `HashMap.get(key)`: the key's own hash decides which entries are compared at
            // all, so a stored key of another kind is never handed to `equals`.
            let hash = value_hash(ev, &key)?;
            for (stored, value) in ev.tool().tlc_cache_get(expr, hash) {
                // `key.equals(entry.key)`: the lookup's key is the receiver, so its class
                // decides the text of a comparison that fails.
                if key.tlc_equals(ev, &stored)? {
                    return Ok(Some(value));
                }
            }
            let value = ev.eval(expr, c, s0, s1, control)?;
            // `value.initialize()`: deep-normalise and fingerprint, so the cached value is
            // usable from any worker.
            value.deep_normalize(ev)?;
            value.fingerprint(ev, crate::fp64::Fp::zero())?;
            ev.tool().tlc_cache_put(expr, key, hash, value.clone());
            Ok(Some(value))
        }
        tlars_sany::graph::Level::State => {
            // `s0.getCached(key)` is null and `s0.setCached(key, ..)` answers null on every
            // state but the simulator's, so the only lasting effects are the closure's
            // evaluation, the `hashCode()` it takes (which fingerprints the value), and the
            // expression's own evaluation. The three node hash codes that make up the key
            // are pure.
            let key = ev.eval(closure, c, s0, StateRef::Empty, EvalControl::CLEAR)?;
            value_hash(ev, &key)?;
            ev.eval(expr, c, s0, s1, control)?;
            Ok(None)
        }
        // "For action or temporal formulas, all we do is to evaluate the TLA+
        // expression--nothing is cached!"
        tlars_sany::graph::Level::Action | tlars_sany::graph::Level::Temporal => Ok(None),
    }
}

/// `Value.hashCode()` (`Value.java:296-307`): the two halves of the value's fingerprint
/// under a fresh polynomial, xored.
fn value_hash(ev: &mut Ev<'_>, v: &Value) -> EvalResult<i32> {
    let fp = v.fingerprint(ev, ev.tool().fp64.new_fp())?.raw();
    Ok(((fp >> 32) as i32) ^ (fp as i32))
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
