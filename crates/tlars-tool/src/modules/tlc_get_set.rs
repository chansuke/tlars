//! `tlc2.module.TLCGetSet`: the Java override of `TLC!TLCGet` and `TLC!TLCSet`.
//!
//! `TLCGet` is an `@Evaluation` override, so it gets the argument node and evaluates it
//! itself, and every failure it raises reaches the spec wrapped in
//! `TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE`. `TLCSet` is an ordinary `@TLAPlusOperator`, whose
//! `EvalException` passes through unwrapped.

use super::{eval_exception, wrap_evaluating};
use crate::actions::Action;
use crate::ctx::{Binding, Ctx};
use crate::error::{EvalError, EvalResult};
use crate::eval::{EvalControl, Ev};
use crate::state::StateRef;
use crate::tool::NamedCheck;
use crate::value::Value;
use tlars_msg::Ec;
use std::io::Read;
use tlars_sany::graph::{ExprOrOpArg, Loc, OpDefId, SymbolRef};
use tlars_sany::{Atom, JavaClass};

/// `TLCGetSet.NAMED_REGISTER_PREFIX`: a string key with this prefix names a register rather
/// than one of the keys below.
const NAMED_REGISTER_PREFIX: &str = "s:";

/// `RecordValue(Location)`' fields (`RecordValue.java:108-128`).
const LOCATION_FIELDS: &[&str] = &["beginLine", "beginColumn", "endLine", "endColumn", "module"];

/// `Action.UNNAMED_ACTION` (`Action.java:35`).
const UNNAMED_ACTION: &str = "UnnamedAction";

/// `SyntaxTreeNode.nullSTN`'s file name (`SyntaxTreeNode.java:215-222`), which is the module
/// `Action.UNKNOWN` reports.
const BUILTINS_MODULE: &str = "--TLA+ BUILTINS--";

/// `TLCGet("spec")`' fields, in the order `TLCGetStringValue` fills the array
/// (`TLCGetSet.java:320-377`). The record prints them in intern-token order instead, which
/// is `TLCGetSet`'s declaration order.
const SPEC_FIELDS: &[&str] = &[
    "inits",
    "actions",
    "temporals",
    "invariants",
    "impliedinits",
    "impliedtemporals",
    "variables",
    "actionconstraints",
    "constraints",
    "impliedactions",
];

/// `TLCGet("revision")`' fields, in fill order (`:291-318`).
const REVISION_FIELDS: &[&str] = &["count", "timestamp", "date", "tag", "calver"];

/// `getStatistics()`' fields, in the order `AbstractChecker` fills them
/// (`AbstractChecker.java:683-709`). The record prints them in intern-token order, which is
/// `TLCGetSet`'s declaration order, not this one.
const STATS_FIELDS: &[&str] = &["queue", "distinct", "initial", "generated", "diameter", "duration", "worker"];

/// `createConfig()`' fields, in the order `AbstractChecker` fills them (`:713-736`).
const CONFIG_FIELDS: &[&str] = &["mode", "deadlock", "worker", "seed", "fingerprint", "install"];

/// Calling a method of `tlc2.module.TLCGetSet` initialises the class, which interns its
/// `UniqueString` statics. Nothing before this point touches them, so a spec that reaches
/// `TLCGet`/`TLCSet` before the checker exists (`processConstantDefns`) is where they land.
fn ensure_class(ev: &Ev<'_>) {
    ev.tool().graph().interner().ensure(JavaClass::TlcRuntime);
}

/// `TLCGetSet.TLCGetEval` (`TLCGetSet.java:160-185`).
pub(crate) fn tlc_get(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    ensure_class(ev);
    let result = tlc_get_eval(ev, args, c, s0, s1, control);
    // `TLCGetEval` never returns null, so it never falls back to the TLA+ definition.
    result.map(Some).map_err(|e| wrap_evaluating(ev, "TLC", "TLCGet", e))
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
    // None of these three reads the checker, so each answers in ASSUME and POSTCONDITION too.
    if key == "spec" {
        return spec_value(ev);
    }
    if key == "revision" {
        return Ok(revision_value(ev));
    }
    if key == "action" {
        // `s0.getAction()` is null unless TLC runs on `TLCStateMutExt` (`-debugger` or
        // `-simulate`), so breadth-first search always reports `Action.UNKNOWN` (`:429-433`).
        return unknown_action_value(ev);
    }
    // `TLCGet("all")` and `TLCGet("all:named")` read worker 0's registers through
    // `TLCGlobals.mainChecker`; with no checker the `if` chain falls through to the throw.
    if matches!(key.as_str(), "all" | "all:named") {
        if ev.tool().main_checker().is_none() {
            return Err(eval_exception(ev, Ec::TLC_MODULE_TLCGET_UNDEFINED, &[&key]));
        }
        if ev.tool().globals().workers > 1 {
            return Err(EvalError::unimplemented(format!("TLCGet(\"{key}\") with more than one worker (M6)")));
        }
        return if key == "all" { all_values(ev) } else { all_named_values(ev) };
    }
    // `duration` reads the clock alone and needs no checker in TLC, but tlars has no clock
    // outside a run.
    if key == "duration" {
        let Some(checker) = ev.tool().main_checker() else {
            return Err(EvalError::unimplemented("TLCGet(\"duration\") outside a run (M3 unit 5)".into()));
        };
        return narrow(ev, checker.duration_secs());
    }
    if matches!(key.as_str(), "diameter" | "generated" | "distinct" | "queue" | "stats" | "config") {
        // Each of these goes through `TLCGlobals.mainChecker`, so with no checker the NPE TLC
        // catches turns into `TLCGet(key) was undefined`.
        let Some(checker) = ev.tool().main_checker() else {
            return Err(eval_exception(ev, Ec::TLC_MODULE_TLCGET_UNDEFINED, &[&key]));
        };
        return match key.as_str() {
            // `IntValue.gen(getProgress())`, an int in Java already.
            "diameter" => Ok(Value::Int(checker.diameter() as i32)),
            "generated" => narrow(ev, checker.generated() as i64),
            "distinct" => narrow(ev, checker.distinct() as i64),
            "queue" => narrow(ev, checker.queue() as i64),
            "stats" => Ok(statistics(ev, checker)),
            _ => Ok(config(ev)),
        };
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
fn narrow(ev: &mut Ev<'_>, v: i64) -> EvalResult<Value> {
    match i32::try_from(v) {
        Ok(n) => Ok(Value::Int(n)),
        Err(_) => Err(eval_exception(ev, Ec::TLC_MODULE_OVERFLOW, &[&v.to_string()])),
    }
}

/// `IntValue.narrowToIntValue(v)`: a value that does not fit becomes -1.
fn narrow_to_int(v: i64) -> Value {
    Value::Int(i32::try_from(v).unwrap_or(-1))
}

/// `RecordValue(Location)` (`RecordValue.java:108-128`).
fn location_value(ev: &mut Ev<'_>, loc: Loc) -> EvalResult<Value> {
    let interner = ev.tool().graph().interner();
    interner.ensure(JavaClass::RecordValue);
    // `new StringValue(location.sourceAsUniqueString())` on a location with no file wraps a
    // null `UniqueString`, which TLC only survives until something prints it. Every node
    // tlars reports here comes from a parsed module, so this is unreachable rather than
    // modelled.
    let Some(module) = loc.module else {
        return Err(EvalError::unimplemented("a location record for a node with no module (M3 unit 5)".into()));
    };
    let names = LOCATION_FIELDS.iter().map(|n| interner.intern(n)).collect();
    let values = vec![
        Value::Int(loc.begin_line),
        Value::Int(loc.begin_col),
        Value::Int(loc.end_line),
        Value::Int(loc.end_col),
        Value::Str(module),
    ];
    Ok(Value::Record(Value::record_arc(names, values, false)))
}

/// `RecordValue(Action)` (`RecordValue.java:172-196`): `name` and `location`, and, when the
/// action carries parameters bound to values, `context` and `parameters` after them.
fn action_record(ev: &mut Ev<'_>, name: Option<Atom>, loc: Loc, op_def: Option<OpDefId>, ctx: &Ctx) -> EvalResult<Value> {
    let interner = ev.tool().graph().interner();
    interner.ensure(JavaClass::RecordValue);
    let name = name.unwrap_or_else(|| interner.intern(UNNAMED_ACTION));
    let location = location_value(ev, loc)?;
    let mut names = vec![interner.intern("name"), interner.intern("location")];
    let mut values = vec![Value::Str(name), location];
    let params = bound_parameters(ev, op_def, ctx);
    if !params.is_empty() {
        // `getParameters()` collects into a `LinkedHashMap`, so both the context record and
        // the tuple of names follow the definition's parameter order.
        let context = Value::record_arc(params.iter().map(|&(n, _)| n).collect(), params.iter().map(|(_, v)| v.clone()).collect(), false);
        names.push(interner.intern("context"));
        values.push(Value::Record(context));
        names.push(interner.intern("parameters"));
        values.push(Value::tuple(params.iter().map(|&(n, _)| Value::Str(n)).collect()));
    }
    Ok(Value::Record(Value::record_arc(names, values, false)))
}

/// `Action.getParameters()` (`Action.java:182-187`): the definition's formal parameters that
/// the action's context binds to a value, in declaration order.
fn bound_parameters(ev: &Ev<'_>, op_def: Option<OpDefId>, ctx: &Ctx) -> Vec<(Atom, Value)> {
    let graph = ev.tool().graph();
    let Some(d) = op_def else { return Vec::new() };
    graph
        .op_def(d)
        .params
        .iter()
        .filter_map(|&p| match ctx.lookup(SymbolRef::Param(p)) {
            Some(Binding::Value(v)) => Some((graph.param(p).name, v.clone())),
            _ => None,
        })
        .collect()
}

/// `init2Value` / `action2Value` (`TLCGetSet.java:552-576`) without `-coverage`: the action's
/// own record, whose location is the predicate's.
pub(crate) fn action_value(ev: &mut Ev<'_>, a: &Action) -> EvalResult<Value> {
    let loc = ev.tool().graph().loc(a.pred);
    let (name, op_def, ctx) = (a.name, a.op_def, a.ctx.clone());
    action_record(ev, name, loc, op_def, &ctx)
}

/// `property2Value` (`:577-589`): an invariant or temporal property is an `Action` too, so it
/// reports the body's location under the name the cfg gave it.
fn check_value(ev: &mut Ev<'_>, c: &NamedCheck) -> EvalResult<Value> {
    let loc = ev.tool().graph().loc(c.expr);
    let (name, ctx) = (c.name, c.ctx.clone());
    action_record(ev, Some(name), loc, None, &ctx)
}

/// `constraint2Value` (`:600-606`) for the `OpDefNode` branch, which is the one a cfg
/// CONSTRAINT or ACTION_CONSTRAINT takes: `RecordValue(OpDefNode)` reports the *definition*'s
/// location, not the body's.
fn constraint_value(ev: &mut Ev<'_>, c: &NamedCheck) -> EvalResult<Value> {
    let interner = ev.tool().graph().interner();
    interner.ensure(JavaClass::RecordValue);
    let Some(d) = c.op_def else {
        return Err(EvalError::unimplemented("a constraint that is not a named definition (M3 unit 5)".into()));
    };
    let (name, loc) = {
        let def = ev.tool().graph().op_def(d);
        (def.name, def.loc)
    };
    let location = location_value(ev, loc)?;
    let names = vec![interner.intern("name"), interner.intern("location")];
    Ok(Value::Record(Value::record_arc(names, vec![Value::Str(name), location], false)))
}

/// `variable2Value` (`:590-598`) without `-coverage`: `RecordValue(OpDeclNode)`.
fn variable_value(ev: &mut Ev<'_>, decl: tlars_sany::graph::DeclId) -> EvalResult<Value> {
    let interner = ev.tool().graph().interner();
    interner.ensure(JavaClass::RecordValue);
    let (name, loc) = {
        let d = ev.tool().graph().decl(decl);
        (d.name, d.loc)
    };
    let location = location_value(ev, loc)?;
    let names = vec![interner.intern("name"), interner.intern("location")];
    Ok(Value::Record(Value::record_arc(names, vec![Value::Str(name), location], false)))
}

/// `Action.UNKNOWN` (`Action.java:37`): the predicate is `SemanticNode.nullSN`, whose syntax
/// node is `SyntaxTreeNode.nullSTN` — four zero coordinates in `--TLA+ BUILTINS--`.
pub(crate) fn unknown_action_value(ev: &mut Ev<'_>) -> EvalResult<Value> {
    let module = ev.tool().graph().interner().intern(BUILTINS_MODULE);
    let loc = Loc { module: Some(module), begin_line: 0, begin_col: 0, end_line: 0, end_col: 0 };
    action_record(ev, None, loc, None, &Ctx::EMPTY)
}

/// `TLCGet("spec")` (`TLCGetSet.java:320-377`). PROPERTY fills `impliedinits`,
/// `impliedactions` and `impliedtemporals`.
fn spec_value(ev: &mut Ev<'_>) -> EvalResult<Value> {
    // `processConfigPossible` appends the `_Track` wrappers of the generated module to
    // `modelConstraints` and `actionConstraints` (`SpecProcessor.java:1864-1883`), so they
    // are among the constraints this record names. tlars evaluates `_Track` directly and
    // generates no module, and the names TLC would print carry the millisecond the module
    // was named after, so there is nothing to answer with here.
    if !ev.tool().possible.is_empty() {
        return Err(EvalError::unimplemented(
            "TLCGet(\"spec\") for a cfg with _POSSIBLE, whose constraints name the generated module (M3 unit 5)".into(),
        ));
    }
    let set = |elems: Vec<Value>| Value::set_enum_of(elems, false);
    let inits = ev.tool().init_actions().iter().map(|a| action_value(ev, a)).collect::<EvalResult<Vec<_>>>();
    let inits = set(inits?);
    let actions = ev.tool().actions().iter().map(|a| action_value(ev, a)).collect::<EvalResult<Vec<_>>>();
    let actions = set(actions?);
    // `getTemporals()` is the SPECIFICATION's temporal conjuncts, which carry no definition
    // and so report `UnnamedAction`.
    let temporals = ev
        .tool()
        .temporal()
        .fairness
        .iter()
        .map(|(expr, ctx)| {
            let loc = ev.tool().graph().loc(*expr);
            action_record(ev, None, loc, None, ctx)
        })
        .collect::<EvalResult<Vec<_>>>();
    let temporals = set(temporals?);
    // `.filter(a -> !a.isInternal())`: tlars only ever records the cfg's own invariants.
    let invariants = ev.tool().invariants.iter().map(|c| check_value(ev, c)).collect::<EvalResult<Vec<_>>>();
    let invariants = set(invariants?);
    let implied_inits = ev.tool().implied_inits.iter().map(|c| check_value(ev, c)).collect::<EvalResult<Vec<_>>>();
    let implied_inits = set(implied_inits?);
    let implied_actions = ev.tool().implied_actions.iter().map(|c| check_value(ev, c)).collect::<EvalResult<Vec<_>>>();
    let implied_actions = set(implied_actions?);
    // `getImpliedTemporals()`, which only a cfg PROPERTY fills; `property2Value` like the
    // other PROPERTY checks.
    let implied_temporals = (0..ev.tool().temporal().properties.len())
        .map(|i| {
            let (name, expr, ctx) = {
                let t = &ev.tool().temporal().properties[i];
                (t.name, t.expr, t.ctx.clone())
            };
            let loc = ev.tool().graph().loc(expr);
            action_record(ev, Some(name), loc, None, &ctx)
        })
        .collect::<EvalResult<Vec<_>>>();
    let implied_temporals = set(implied_temporals?);
    let variables = ev.tool().vars().decls.iter().map(|&d| variable_value(ev, d)).collect::<EvalResult<Vec<_>>>();
    let variables = set(variables?);
    let action_constraints = ev.tool().action_constraints.iter().map(|c| constraint_value(ev, c)).collect::<EvalResult<Vec<_>>>();
    let action_constraints = set(action_constraints?);
    let constraints = ev.tool().model_constraints.iter().map(|c| constraint_value(ev, c)).collect::<EvalResult<Vec<_>>>();
    let constraints = set(constraints?);
    let interner = ev.tool().graph().interner();
    interner.ensure(JavaClass::RecordValue);
    let names = SPEC_FIELDS.iter().map(|n| interner.intern(n)).collect();
    let values = vec![
        inits,
        actions,
        temporals,
        invariants,
        implied_inits,
        implied_temporals,
        variables,
        action_constraints,
        constraints,
        implied_actions,
    ];
    Ok(Value::Record(Value::record_arc(names, values, false)))
}

/// `TLCGet("revision")` (`TLCGetSet.java:291-318`), which reads the jar manifest's build
/// identity: the commit count, the build timestamp as epoch seconds and as
/// `yyyy-MM-dd'T'HH:mm:ss.S'Z'` in UTC, the git short revision and the CalVer version.
///
/// tlars reports its own identity here, as 2262 and 2187 do (docs/DESIGN.md). `count` and
/// `tag` are the same fallbacks TLC's own code takes for a jar with no manifest, `calver` is
/// tlars' version, the string 2262 prints where TLC prints `Version.number()`, and the build
/// time is the zero every 2187 field tlars has no value for reports, rather than a plausible
/// date tlars never recorded. The corpus pins `DOMAIN` alone, because TLC's own values here
/// change with every build.
fn revision_value(ev: &mut Ev<'_>) -> Value {
    let interner = ev.tool().graph().interner();
    interner.ensure(JavaClass::RecordValue);
    let values = vec![
        Value::Int(0),
        Value::Int(0),
        Value::Str(interner.intern("1970-01-01T00:00:00.0Z")),
        Value::Str(interner.intern("development")),
        Value::Str(interner.intern(env!("CARGO_PKG_VERSION"))),
    ];
    let names = REVISION_FIELDS.iter().map(|n| interner.intern(n)).collect();
    Value::Record(Value::record_arc(names, values, false))
}

/// `Value.hashCode()` (`Value.java:296-307`): the two halves of the value's fingerprint under
/// the run's polynomial, xored. It is what orders a `java.util.HashMap` keyed by values.
pub(crate) fn value_hash(ev: &mut Ev<'_>, v: &Value) -> EvalResult<i32> {
    let fp = v.fingerprint(ev, ev.tool().fp64.new_fp())?.raw();
    Ok(((fp >> 32) as i32) ^ (fp as i32))
}

/// `AbstractChecker.getAllValues()` (`:544-563`): `[i \in I |-> [w \in W |-> TLCGet(i)]]`,
/// built as a `HashMap` keyed by `IntValue`, so the domain comes out in that map's iteration
/// order. Only indices worker 0 wrote appear, which with one worker is every index written.
fn all_values(ev: &mut Ev<'_>) -> EvalResult<Value> {
    let (indexed, capacity) = {
        let registers = ev.tool().registers.lock().expect("registers");
        let indexed: Vec<(usize, Value)> = registers.indexed().map(|(i, v)| (i, v.clone())).collect();
        (indexed, registers.indexed_len())
    };
    let mut map = tlars_java::JavaHashMap::with_capacity(capacity);
    for (i, v) in indexed {
        let key = Value::Int(i32::try_from(i).expect("a register index fits an int"));
        let hash = value_hash(ev, &key)?;
        map.put(key, hash, Value::tuple(vec![v]), |a, b| matches!((a, b), (Value::Int(x), Value::Int(y)) if x == y));
    }
    let (domain, values): (Vec<Value>, Vec<Value>) = map.iter().map(|(k, v)| (k.clone(), v.clone())).unzip();
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, values, false)))
}

/// `AbstractChecker.getAllNamedRegisterValues()` (`:565-578`): the same shape keyed by the
/// register names. `PrintT` normalises what it prints, but `ToString` and the error messages
/// do not, so the `HashMap` order shows through.
///
/// It is filled from `((IdThread) workers[0]).getNamedRegisters().keySet()`, a
/// `ConcurrentHashMap<UniqueString, IValue>`, so the order the keys reach the `HashMap` in is
/// that map's iteration order and not the order they were first set in.
fn all_named_values(ev: &mut Ev<'_>) -> EvalResult<Value> {
    let named: Vec<(Atom, Value)> = ev.tool().registers.lock().expect("registers").named().to_vec();
    let mut registers = tlars_java::JavaConcurrentHashMap::new();
    for (name, v) in named {
        // `UniqueString.hashCode()` is `String.hashCode()` of its text (`:207-211`).
        let hash = tlars_java::string_hash_code(&ev.tool().graph().interner().text(name));
        registers.put(name, hash, v, |a, b| a == b);
    }
    if registers.needs_tree_bin() {
        return Err(EvalError::unimplemented(
            "TLCGet(\"all:named\") where the register names fill a ConcurrentHashMap tree bin".into(),
        ));
    }
    let named: Vec<(Atom, Value)> = registers.iter().map(|(&n, v)| (n, v.clone())).collect();
    let mut map = tlars_java::JavaHashMap::new();
    for (name, v) in named {
        let key = Value::Str(name);
        let hash = value_hash(ev, &key)?;
        map.put(key, hash, Value::tuple(vec![v]), |a, b| matches!((a, b), (Value::Str(x), Value::Str(y)) if x == y));
    }
    let (domain, values): (Vec<Value>, Vec<Value>) = map.iter().map(|(k, v)| (k.clone(), v.clone())).unzip();
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, values, false)))
}

/// `AbstractChecker.getStatistics()` (`:683-709`).
fn statistics(ev: &mut Ev<'_>, checker: &dyn crate::effects::MainChecker) -> Value {
    let values = vec![
        narrow_to_int(checker.queue() as i64),
        narrow_to_int(checker.distinct() as i64),
        narrow_to_int(checker.initial() as i64),
        narrow_to_int(checker.generated() as i64),
        narrow_to_int(i64::from(checker.diameter())),
        narrow_to_int(checker.duration_secs()),
        // `Thread.currentThread() instanceof IdThread ? IdThread.GetId() : 0`, and tlars
        // numbers the main thread 0 too.
        Value::Int(i32::from(ev.worker_id())),
    ];
    let interner = ev.tool().graph().interner();
    let names = STATS_FIELDS.iter().map(|n| interner.intern(n)).collect();
    Value::Record(Value::record_arc(names, values, false))
}

/// `AbstractChecker.createConfig()` (`:711-736`), which the checker builds once at startup.
/// `mode` is `"bfs"` for the model checker; the simulator's own record is M5.
fn config(ev: &mut Ev<'_>) -> Value {
    let globals = ev.tool().globals();
    let (deadlock, workers, seed) = (globals.check_deadlock, globals.workers, globals.seed);
    let install = globals.install_location.clone();
    let poly = ev.tool().fp64.irred_poly();
    let interner = ev.tool().graph().interner();
    let values = vec![
        Value::Str(interner.intern("bfs")),
        Value::Bool(deadlock),
        Value::Int(workers as i32),
        Value::Str(interner.intern(&seed.to_string())),
        Value::Str(interner.intern(&poly.to_string())),
        Value::Str(interner.intern(&install)),
    ];
    let names = CONFIG_FIELDS.iter().map(|n| interner.intern(n)).collect();
    Value::Record(Value::record_arc(names, values, false))
}

/// `TLCGetSet.TLCSet` (`TLCGetSet.java:463-535`).
pub(crate) fn tlc_set(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    ensure_class(ev);
    if let Value::Int(i) = a[0]
        && i >= 0
    {
        // `IdThread.setLocalValue` on a worker and `mainChecker.setAllValues` elsewhere; with
        // no checker the `else` dereferences the null `TLCGlobals.simulator`
        // (`TLCGetSet.java:495-503`). `processConstantDefns` swallows that
        // `NullPointerException`, and anywhere else it reaches the spec as a JVM failure
        // whose text tlars cannot reproduce, so nothing is written either way.
        if ev.tool().main_checker().is_none() {
            return Err(EvalError::unimplemented(
                "TLCSet on a numbered register before the checker exists (TLC raises a NullPointerException)".into(),
            ));
        }
        ev.tool().registers.lock().expect("registers").set(i as usize, a[1].clone());
        return Ok(Value::Bool(true));
    }
    if let Value::Str(name) = a[0] {
        let key = ev.tool().graph().interner().text(name).to_string();
        // Every string key answers TRUE; only a TRUE second argument does anything.
        let asked = matches!(a[1], Value::Bool(true));
        match key.as_str() {
            // `if (mainChecker != null) mainChecker.stop()`: with no checker it is a no-op
            // that still answers TRUE.
            "exit" => {
                if asked
                    && let Some(checker) = ev.tool().main_checker()
                {
                    checker.stop();
                }
                return Ok(Value::Bool(true));
            }
            // `synchronized (mc.theStateQueue) { println; System.in.read(); }`
            // (`TLCGetSet.java:518-535`): the prompt is a plain line, not an MP message, and
            // the run goes on after one byte. Both happen inside one critical section, so the
            // lock comes first and no second pause can print its prompt in between. TLC holds
            // the *state queue*, which also stops the other workers from dequeuing; with one
            // worker there is nothing else to stop, and M6 has to take the queue here.
            "pause" => {
                // `mainChecker instanceof ModelChecker`: with no model checker, nothing
                // prompts and nothing is read.
                if asked && ev.tool().main_checker().is_some() {
                    let mut stdin = ev.tool().stdin.lock().expect("stdin");
                    ev.out().raw("Press enter to resume model checking.\n");
                    let mut byte = [0u8; 1];
                    // `catch (IOException e) { throw new EvalException(EC.GENERAL, e.getMessage()) }`;
                    // end of file is `read() == -1`, not an exception, so the run resumes.
                    if let Err(e) = stdin.read(&mut byte) {
                        let message = e.to_string();
                        drop(stdin);
                        return Err(eval_exception(ev, Ec::GENERAL, &[&message]));
                    }
                }
                return Ok(Value::Bool(true));
            }
            // `setAllNamedValues` through the checker or the simulator, and with neither the
            // write is dropped while the operator still answers TRUE (`:536-546`).
            _ if key.starts_with(NAMED_REGISTER_PREFIX) => {
                if ev.tool().main_checker().is_some() {
                    ev.tool().registers.lock().expect("registers").set_named(name, a[1].clone());
                }
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
