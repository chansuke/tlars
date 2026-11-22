//! `Tool.java`, mirrored. [`Ev`] is "the Tool as seen by one worker": the shared, frozen
//! [`Tool`] plus that worker's mutable locals. TLC's `FastTool`/`CallStackTool`/`DebugTool`
//! subclasses become one switch: `WorkerLocals::call_stack` (the debugger would hook here).
//!
//! Module map (each mirrors a region of `Tool.java`):
//! - `init`    `getInitStates*`           (473-890)
//! - `next`    `getNextStates*`, `processUnchanged` (897-1600)
//! - `expr`    `eval*`, `evalAppl*`, `lookup`, `getOpContext`, `contexts` (1795-2858, 3989-4120)
//! - `enabled` `enabled*`                 (2953-3600)
//! - `replay`  `getState(fp)`, `getState(fp, pred)` (3719-3805)
//! - `alias`   `evalAlias` + `RecordValue.PrintTLCState` (1680-1766)

mod alias;
mod enabled;
pub(crate) mod expr;
mod init;
mod next;
mod replay;

pub use alias::Aliased;

use crate::actions::ActionId;
use crate::effects::Effects;
use crate::error::{EvalError, EvalResult, Flow};
use crate::ctx::Ctx;
use crate::fp64::Fp;
use crate::state::{PartialState, State};
use crate::tool::{CheckId, CounterExampleDef, ImpliedAction, ImpliedInit, Invariant, Tool};
use crate::value::Value;
use tlars_msg::{Ec, Report};
use tlars_sany::Atom;
use tlars_sany::graph::ExprId;

/// `EvalControl` bit flags (`Clear`, `KeepLazy`, `Primed`, `Enabled`, `Init`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EvalControl(u8);

impl EvalControl {
    pub const CLEAR: Self = Self(0);
    pub const KEEP_LAZY: Self = Self(1);
    pub const PRIMED: Self = Self(2);
    pub const ENABLED: Self = Self(4);
    pub const INIT: Self = Self(8);
    pub const CONST: Self = Self(16);

    pub fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub fn has(self, flag: Self) -> bool {
        self.0 & flag.0 != 0
    }
    /// `EvalControl.setPrimedIfEnabled`: primed only inside ENABLED, where the context
    /// lookup has to stop at the branch.
    pub(crate) fn set_primed_if_enabled(self) -> Self {
        if self.has(Self::ENABLED) { self.with(Self::PRIMED) } else { self }
    }
    /// `EvalControl.semanticallyEquivalent` (LazyValue cache guard).
    pub(crate) fn semantically_equivalent(self, other: Self) -> bool {
        // `KeepLazy`, `Init` and `Const` do not change what an expression means
        // (`EvalControl.java:108-130`).
        let ignored = Self::KEEP_LAZY.0 | Self::INIT.0 | Self::CONST.0;
        self.0 & !ignored == other.0 & !ignored
    }
}

/// Per-worker mutable evaluator state: what TLC keeps in `IdThread` locals and statics.
pub struct WorkerLocals {
    pub(crate) worker_id: u16,
    /// RNG and the current parent: the one effect channel (`effects.rs`).
    pub(crate) effects: Effects,
    /// `Some` while re-running a failure under `CallStackTool` semantics.
    pub(crate) call_stack: Option<CallStack>,
}

impl WorkerLocals {
    pub fn new(worker_id: u16, seed: i64) -> Self {
        Self { worker_id, effects: Effects::new(seed), call_stack: None }
    }
    /// Start recording the nested-expression stack (TLC re-runs the failing step with a
    /// `CallStackTool` to print `TLC_NESTED_EXPRESSION`).
    pub fn record_call_stack(&mut self) {
        self.call_stack = Some(CallStack::default());
    }
    pub fn take_call_stack(&mut self) -> Option<CallStack> {
        self.call_stack.take()
    }
}

/// The `catch` clauses of a `CallStackTool` override: a `TLCRuntimeException` or an
/// `EvalException` freezes the stack, and a `FingerprintException` also appends the sources
/// of the values it travelled through (`CallStack.freeze(FingerprintException)`).
fn freeze_for(cs: &mut CallStack, failure: Option<&crate::error::EvalErrorKind>) {
    match failure {
        Some(crate::error::EvalErrorKind::Fingerprint { sources }) => cs.freeze_with(sources),
        Some(crate::error::EvalErrorKind::Runtime | crate::error::EvalErrorKind::Eval) => cs.freeze(),
        _ => {}
    }
}

/// `CallStackTool`'s frames (`tlc2.tool.CallStack`). A failure freezes the stack, so the
/// `pop`s that run while the Rust stack unwinds leave it as it was at the throw.
#[derive(Default)]
pub struct CallStack {
    frames: Vec<ExprId>,
    frozen: bool,
}

impl CallStack {
    fn push(&mut self, expr: ExprId) {
        self.frames.push(expr);
    }
    fn pop(&mut self) {
        if !self.frozen {
            self.frames.pop();
        }
    }
    fn freeze(&mut self) {
        self.frozen = true;
    }
    /// `freeze(FingerprintException)`: the first freeze also pushes the exception's trace.
    fn freeze_with(&mut self, sources: &[ExprId]) {
        if self.frozen {
            return;
        }
        self.frozen = true;
        self.frames.extend_from_slice(sources);
    }
    /// `CallStack.toString` (`CallStack.java:53-88`): one numbered line per frame, counted
    /// from zero, with consecutive repeats of the same node collapsed.
    pub fn to_tlc_string(&self, tool: &Tool) -> String {
        if self.frames.is_empty() {
            return "    The error call stack is empty.\n".to_string();
        }
        let graph = tool.graph();
        let mut out = String::new();
        let mut depth = 0;
        let mut last = None;
        for &expr in self.frames.iter() {
            if last == Some(expr) {
                continue;
            }
            last = Some(expr);
            let loc = graph.loc(expr);
            let module = loc.module.map_or_else(|| "null".to_string(), |m| graph.interner().text(m).to_string());
            out.push_str(&format!(
                "{depth}. Line {}, column {} to line {}, column {} in {module}\n",
                loc.begin_line, loc.begin_col, loc.end_line, loc.end_col
            ));
            depth += 1;
        }
        out.push('\n');
        out
    }
}

/// A reconstructed trace step: the state plus the action whose re-run produced it first
/// (`None` = `<Initial predicate>`).
pub struct StateInfo {
    pub state: State,
    pub action: Option<ActionId>,
    /// `TLCStateInfo.fp`, the fingerprint the step was looked up by. `fingerPrint()` answers
    /// with it when it is there and computes one from the state otherwise, and
    /// `AliasTLCStateInfo` copies it, so a step that carries one prints the fingerprint of
    /// the original state even where an ALIAS replaced the variables. Only `getState(fp)`
    /// sets it (`Tool.java:3759`); the successor lookups leave it empty.
    pub fp: Option<Fp>,
}

/// `IStateFunctor`: receives every initial state generated (`DoInitFunctor`).
pub trait InitSink {
    fn add_init(&mut self, ev: &mut Ev<'_>, s: &PartialState) -> Flow;
}

/// `INextStateFunctor`: receives every successor (`Worker.addElement`).
pub trait NextSink {
    fn add_next(&mut self, ev: &mut Ev<'_>, s0: &State, action: ActionId, s1: &PartialState) -> Flow;
    /// `addUnsatisfiedState` (only `-dump ... constrained` and the debugger care).
    fn add_unsatisfied(&mut self, ev: &mut Ev<'_>, s0: &State, action: ActionId, s1: &PartialState) -> Flow {
        Ok(())
    }
}

/// Maximum evaluation depth before raising TLC's StackOverflowError message instead of
/// overflowing the native stack. Evaluation runs on threads of `Env::worker_stack_bytes`
/// (64 MiB): 2^15 levels of operator recursion measured to fit, 2^16 not; 2^14 leaves room
/// for expressions with larger frames. TLC's own limit follows the JVM's `-Xss`, so the depth
/// at which the two overflow differs.
pub(crate) const MAX_EVAL_DEPTH: u32 = 1 << 14;

pub struct Ev<'t> {
    tool: &'t Tool,
    locals: &'t mut WorkerLocals,
    out: &'t dyn Report,
    depth: u32,
    /// Monotone per-worker counter giving each parent expansion its identity.
    expansions: u64,
}

impl<'t> Ev<'t> {
    pub(crate) fn new(tool: &'t Tool, locals: &'t mut WorkerLocals, out: &'t dyn Report) -> Self {
        Ev { tool, locals, out, depth: 0, expansions: 0 }
    }

    pub fn tool(&self) -> &'t Tool {
        self.tool
    }
    pub fn out(&self) -> &'t dyn Report {
        self.out
    }
    /// `IdThread.GetId()`, which is 0 on the main thread (`getStatistics`'s `worker` field).
    pub(crate) fn worker_id(&self) -> u16 {
        self.locals.worker_id
    }

    // ---- checks the engine runs around each successor (ITool) ----

    /// `isGoodState` + freeze. `None` = some variable unassigned.
    pub fn complete(&self, s1: &PartialState) -> Option<State> {
        s1.freeze()
    }
    /// `isInModel`: all CONSTRAINTs.
    pub fn is_in_model(&mut self, s: &State) -> EvalResult<bool> {
        let checks = self.tool.model_constraints.len();
        for i in 0..checks {
            let (expr, ctx) = {
                let check = &self.tool.model_constraints[i];
                (check.expr, check.ctx.clone())
            };
            if !self.is_valid_in(expr, &ctx, crate::state::StateRef::Full(s), crate::state::StateRef::Empty)? {
                return Ok(false);
            }
        }
        self.track_possible(false, crate::state::StateRef::Full(s), crate::state::StateRef::Empty)?;
        Ok(true)
    }
    /// `isInActions`: all ACTION_CONSTRAINTs.
    pub fn is_in_actions(&mut self, s0: &State, s1: &State) -> EvalResult<bool> {
        let checks = self.tool.action_constraints.len();
        for i in 0..checks {
            let (expr, ctx) = {
                let check = &self.tool.action_constraints[i];
                (check.expr, check.ctx.clone())
            };
            if !self.is_valid_in(expr, &ctx, crate::state::StateRef::Full(s0), crate::state::StateRef::Full(s1))? {
                return Ok(false);
            }
        }
        self.track_possible(true, crate::state::StateRef::Full(s0), crate::state::StateRef::Full(s1))?;
        Ok(true)
    }
    /// `TLCStateMut.fingerPrint` (`TLCStateMut.java:171-237`): with no SYMMETRY and no VIEW,
    /// the values' own fingerprints in `vars[]` order. Each value normalises itself on the
    /// way, which is why fingerprinting a state also leaves it shareable.
    pub fn fingerprint(&mut self, s: &State) -> EvalResult<Fp> {
        // `minVals`: `None` is TLC's `minVals == this.values`, the state as it stands.
        let min_vals = self.symmetry_min(s)?;
        let mut fp = self.tool.fp64.new_fp();
        match &self.tool.view {
            // The values' own fingerprints in `vars[]` order. Each value normalises itself
            // on the way, which is why fingerprinting a state also leaves it shareable --
            // and why a state replaced by its representative is normalised separately.
            None => {
                for i in 0..s.len() {
                    let v = match &min_vals {
                        Some(vals) => vals[i].clone(),
                        None => s.get(self.tool.vars.slot(i)).clone(),
                    };
                    fp = v.fingerprint(self, fp)?;
                }
                if min_vals.is_some() {
                    for i in 0..s.len() {
                        let v = s.get(self.tool.vars.slot(i)).clone();
                        v.deep_normalize(self)?;
                    }
                }
                Ok(fp)
            }
            // With a VIEW the state's own values are normalised explicitly, because only
            // the view's value is fingerprinted, and the view is evaluated against the
            // symmetry representative (`TLCStateMut.java:261-281`).
            Some(view) => {
                let (expr, ctx) = (view.expr, view.ctx.clone());
                for i in 0..s.len() {
                    let v = s.get(self.tool.vars.slot(i)).clone();
                    v.deep_normalize(self)?;
                }
                let state = match min_vals {
                    Some(vals) => crate::state::State::of_values(vals),
                    None => s.clone(),
                };
                self.with_fast_tool(|ev| {
                    let val =
                        ev.eval(expr, &ctx, crate::state::StateRef::Full(&state), crate::state::StateRef::Empty, EvalControl::CLEAR)?;
                    val.fingerprint(ev, fp)
                })
            }
        }
    }

    /// The `perms != null` loop of `fingerPrint` (`TLCStateMut.java:201-247`): apply every
    /// permutation of the group and keep the lexicographically smallest state, comparing
    /// variable by variable in `vars[]` order. A permutation whose first differing value is
    /// larger is abandoned, because `perms` is the cross product of the symmetry sets.
    /// `None` means no permutation beat the state itself.
    fn symmetry_min(&mut self, s: &State) -> EvalResult<Option<Vec<crate::value::Value>>> {
        if self.tool.symmetry.is_empty() {
            return Ok(None);
        }
        let sz = s.len();
        // The group is frozen in the shared tool, whose borrow outlives `self`.
        let tool: &'t Tool = self.tool;
        let mut min_vals: Option<Vec<crate::value::Value>> = None;
        for i in 0..tool.symmetry.len() {
            let mut cmp = std::cmp::Ordering::Equal;
            let mut vals: Vec<crate::value::Value> = Vec::with_capacity(sz);
            let mut abandoned = false;
            for j in 0..sz {
                let own = s.get(self.tool.vars.slot(j)).clone();
                let permuted = own.permute(self, &tool.symmetry[i])?.unwrap_or_else(|| own.clone());
                vals.push(permuted);
                if cmp == std::cmp::Ordering::Equal {
                    let current = match &min_vals {
                        Some(m) => m[j].clone(),
                        None => own,
                    };
                    cmp = vals[j].clone().tlc_compare(self, &current)?;
                    if cmp == std::cmp::Ordering::Greater {
                        abandoned = true;
                        break;
                    }
                }
            }
            if abandoned {
                continue;
            }
            if cmp == std::cmp::Ordering::Less {
                min_vals = Some(vals);
            }
        }
        Ok(min_vals)
    }
    /// One INVARIANT, `Ok(false)` = violated.
    pub fn check_invariant(&mut self, id: CheckId<Invariant>, s: &State) -> EvalResult<bool> {
        let (expr, ctx) = {
            let check = &self.tool.invariants[id.index()];
            (check.expr, check.ctx.clone())
        };
        self.is_valid_in(expr, &ctx, crate::state::StateRef::Full(s), crate::state::StateRef::Empty)
    }
    /// One implied action `[][A]_v`, checked for every successor, seen or not.
    pub fn check_implied_action(&mut self, id: CheckId<ImpliedAction>, s0: &State, s1: &State) -> EvalResult<bool> {
        let (expr, ctx) = {
            let check = &self.tool.implied_actions[id.index()];
            (check.expr, check.ctx.clone())
        };
        self.is_valid_in(expr, &ctx, crate::state::StateRef::Full(s0), crate::state::StateRef::Full(s1))
    }
    pub fn check_implied_init(&mut self, id: CheckId<ImpliedInit>, s: &State) -> EvalResult<bool> {
        let (expr, ctx) = {
            let check = &self.tool.implied_inits[id.index()];
            (check.expr, check.ctx.clone())
        };
        self.is_valid_in(expr, &ctx, crate::state::StateRef::Full(s), crate::state::StateRef::Empty)
    }
    /// `_Possible!_Track(expr, name)` for the constraints `processConfigPossible` wires in
    /// after the cfg's own (`_Possible.tla`): evaluate the predicate and record it in the
    /// `s:_possible` register, raising the count when it holds and registering a zero the
    /// first time it does not. It always answers TRUE, so a `_POSSIBLE` predicate never
    /// narrows the state space.
    fn track_possible(&mut self, is_action: bool, s0: crate::state::StateRef<'_>, s1: crate::state::StateRef<'_>) -> EvalResult<()> {
        for i in 0..self.tool.possible.len() {
            let (name, expr, wanted) = {
                let check = &self.tool.possible[i];
                (check.name, check.expr, check.is_action)
            };
            if wanted != is_action {
                continue;
            }
            let held = match self.eval(expr, &Ctx::EMPTY, s0, s1, EvalControl::CLEAR)? {
                Value::Bool(b) => b,
                // `IF b THEN .. ELSE ..` inside `_Track` on a non-boolean raises an
                // `EvalException` that names the generated module's IF, which tlars has no
                // syntax for.
                v => {
                    let what = format!("a _POSSIBLE predicate that is not a boolean but a {} (M3 unit 5)", v.kind_string());
                    return Err(crate::error::EvalError::unimplemented(what));
                }
            };
            let mut counts = self.possible_counts()?;
            match counts.iter_mut().find(|(n, _)| *n == name) {
                // `[old EXCEPT ![name] = @ + 1]`, which keeps the domain order.
                Some(slot) if held => slot.1 = slot.1.wrapping_add(1),
                // `old @@ (name :> 0)` on a name already there changes nothing: `@@` keeps
                // the left function's value, so a witnessed count never falls back to zero.
                Some(_) => {}
                // `old @@ (name :> 1)` / `old @@ (name :> 0)` appends.
                None => counts.push((name, i32::from(held))),
            }
            self.put_possible_counts(counts);
        }
        Ok(())
    }

    /// `TLCGetOrDefault("s:_possible", <<>>)` decoded: the names `_Track` has seen and their
    /// counts, in the order `@@` appended them. An unset register is the empty tuple, whose
    /// DOMAIN is empty.
    ///
    /// A spec can write the register itself. A non-empty tuple there gives `_Track` an
    /// integer interval as `DOMAIN old`, so `name \in DOMAIN old` fails; TLC's error carries
    /// a fingerprint stack trace through the expressions of `_Possible` and of the module it
    /// generated, which the shortcut has no nodes for. The run stops instead of guessing it.
    pub(crate) fn possible_counts(&mut self) -> EvalResult<Vec<(Atom, i32)>> {
        let stored = self.tool.registers.lock().expect("registers").get_named(self.possible_register());
        match stored {
            None => Ok(Vec::new()),
            Some(Value::Tuple(t)) if t.elems.is_empty() => Ok(Vec::new()),
            Some(Value::Tuple(_)) => Err(crate::error::EvalError::unimplemented(
                "a _POSSIBLE register a spec overwrote with a non-empty tuple, whose DOMAIN is an integer interval (M3 unit 5)".into(),
            )),
            Some(Value::FcnRcd(fcn)) => {
                let (domain, values) = fcn.snapshot();
                let Some(domain) = domain else {
                    return Err(crate::error::EvalError::unimplemented("a _POSSIBLE register holding a function over an interval (M3 unit 5)".into()));
                };
                let mut out = Vec::with_capacity(values.len());
                for (d, v) in domain.iter().zip(values.iter()) {
                    match (d, v) {
                        (Value::Str(n), Value::Int(c)) => out.push((*n, *c)),
                        _ => {
                            return Err(crate::error::EvalError::unimplemented(
                                "a _POSSIBLE register a spec overwrote with something other than a record of counts (M3 unit 5)".into(),
                            ));
                        }
                    }
                }
                Ok(out)
            }
            Some(_) => Err(crate::error::EvalError::unimplemented(
                "a _POSSIBLE register a spec overwrote with something other than a record of counts (M3 unit 5)".into(),
            )),
        }
    }

    /// `TLCSet("s:_possible", ..)`, which with one worker writes the single register bank.
    fn put_possible_counts(&mut self, counts: Vec<(Atom, i32)>) {
        let value = possible_record(counts);
        let key = self.possible_register();
        self.tool.registers.lock().expect("registers").set_named(key, value);
    }

    /// `_Possible.KEY`, the one named register `_Track` and `_Counts` share.
    fn possible_register(&self) -> Atom {
        self.tool.graph.interner().intern("s:_possible")
    }

    /// `_Possible!_CheckName(name)` for each `_POSSIBLE` predicate, which
    /// `getPostConditionSpecs()` puts ahead of the cfg's own POSTCONDITIONs. The first
    /// predicate the run never witnessed prints the whole count record and stops the loop,
    /// so only one is ever named.
    fn check_possible(&mut self) -> Result<(), (tlars_msg::Ec, Vec<String>)> {
        if self.tool.possible.is_empty() {
            return Ok(());
        }
        let graph = self.tool.graph.clone();
        for i in 0..self.tool.possible.len() {
            let (name, expr) = {
                let check = &self.tool.possible[i];
                (check.name, check.expr)
            };
            // `_Counts`, which merges every worker's register; with one worker it is that
            // register, rebuilt as `new FcnRcdValue(domain, values, false)`.
            let counts = match self.possible_counts() {
                Ok(counts) => counts,
                Err(e) => return Err((Ec::GENERAL, vec![e.message])),
            };
            if counts.iter().any(|(n, c)| *n == name && *c != 0) {
                continue;
            }
            // `PrintT(counts) /\ FALSE`.
            if let Err(e) = crate::modules::tlc::print_t(self, &[possible_record(counts)]) {
                return Err((Ec::GENERAL, vec![e.message]));
            }
            let shown = graph.expr_tlc_text(expr);
            let name = graph.interner().text(name).to_string();
            return Err((Ec::TLC_POSSIBLE_UNWITNESSED, vec![name, shown]));
        }
        Ok(())
    }

    /// `Tool.checkPostConditionWithContext` (`Tool.java:3681-3712`): each POSTCONDITION is a
    /// constant-level predicate, and the first one that is false or fails stops the loop.
    /// `Err` is what `MP.printError` is given; the caller decides whether that code becomes
    /// the run's.
    pub fn check_postconditions(&mut self, counter_example: Option<Value>) -> Result<(), (tlars_msg::Ec, Vec<String>)> {
        debug_assert!(self.postcondition_exception(false).is_none(), "the caller catches the lookup's exception first");
        self.check_possible()?;
        // `Context.Empty.cons(getCounterExampleDef(), value)`
        // (`Tool.checkPostConditionWithCounterExample`), which every POSTCONDITION is
        // evaluated under. With no counterexample, or with no `TLCExt!CounterExample` in the
        // spec, it is `checkPostCondition()`'s empty context.
        let bound = match (counter_example, self.tool.counter_example_def()) {
            (Some(v), Some(d)) => Ctx::EMPTY.cons(tlars_sany::graph::SymbolRef::OpDef(d), crate::ctx::Binding::Value(v)),
            _ => Ctx::EMPTY,
        };
        let graph = self.tool.graph.clone();
        for i in 0..self.tool.postconditions.len() {
            // `isValid(pc.pred, ctxt)` evaluates in `ctxt`, not in the Action's own context,
            // which `getPostConditionSpecs` leaves empty anyway.
            let (expr, ctx, name) = {
                let pc = &self.tool.postconditions[i];
                (pc.expr, bound.clone(), graph.interner().text(pc.name).to_string())
            };
            let shown = graph.expr_tlc_text(expr);
            match self.is_valid_const(expr, &ctx) {
                Ok(true) => {}
                Ok(false) => return Err((Ec::TLC_POSTCONDITION_FALSE, vec![name, shown])),
                // `catch (Exception e)`, which a `java.lang.Error` escapes.
                Err(e) if matches!(e.kind, crate::error::EvalErrorKind::StackOverflow | crate::error::EvalErrorKind::Assertion) => {
                    let what = "a java.lang.Error raised while a POSTCONDITION is evaluated (M3)";
                    return Err((Ec::GENERAL, vec![format!("tlars does not implement {what} yet")]));
                }
                Err(e) => return Err((Ec::TLC_POSTCONDITION_EVALUATION_ERROR, vec![name, shown, e.message])),
            }
        }
        Ok(())
    }

    /// The `Assert.fail` of the two lookups a postcondition check runs before it checks
    /// anything: `getCounterExampleDef()` (`Tool.java:3672`) and then `getPostConditionSpecs()`
    /// (`Tool.java:3683`). Neither is inside a `try` of `checkPostCondition*`, so this is an
    /// exception travelling out of the check, and every caller catches it where TLC does.
    /// `counter_example` is false for `checkPostCondition()`, which looks up no counterexample
    /// definition at all.
    pub fn postcondition_exception(&self, counter_example: bool) -> Option<EvalError> {
        // `Assert.fail(EC.GENERAL)`, which is given no parameters.
        if counter_example && self.tool.counter_example == CounterExampleDef::NotEvaluating {
            return Some(EvalError::runtime(Ec::GENERAL, self.out.text(Ec::GENERAL, &[])));
        }
        self.tool.postcondition_error.clone().filter(|e| matches!(e.kind, crate::error::EvalErrorKind::Runtime))
    }

    /// A name both lookups resolve for TLC, bound to a Java method tlars has not written.
    /// TLC runs it, so there is no exception to catch and nothing may swallow this: a run
    /// that agreed with TLC by staying quiet would be agreeing by accident.
    pub fn postcondition_unimplemented(&self, _counter_example: bool) -> Option<EvalError> {
        self.tool.postcondition_error.clone().filter(|e| matches!(e.kind, crate::error::EvalErrorKind::Unimplemented))
    }

    /// `Tool.isValid(action, s0, s1)`: the predicate must be a boolean.
    fn is_valid_in(
        &mut self,
        expr: ExprId,
        ctx: &crate::ctx::Ctx,
        s0: crate::state::StateRef<'_>,
        s1: crate::state::StateRef<'_>,
    ) -> EvalResult<bool> {
        match self.eval(expr, ctx, s0, s1, EvalControl::CLEAR)? {
            crate::value::Value::Bool(b) => Ok(b),
            _ => {
                let graph = &self.tool.graph;
                let loc = graph.loc(expr);
                let message = self.out.text(Ec::TLC_EXPECTED_VALUE, &["boolean", &graph.expr_tlc_text(expr)]);
                Err(EvalError { ec: Ec::TLC_EXPECTED_VALUE, message, loc: Some(loc), kind: crate::error::EvalErrorKind::Runtime, params: Vec::new() })
            }
        }
    }

    /// `TLCStateMut.equals` (`TLCStateMut.java:85-100`): slot by slot, an unassigned slot
    /// matching only another unassigned one. A comparison of incomparable values throws a
    /// `TLCRuntimeException`, which `getState(s1, s)` catches to skip that candidate
    /// (`Tool.java:3812-3828`), so it answers "not this one" rather than failing.
    pub fn states_match(&mut self, a: &PartialState, b: &PartialState) -> bool {
        for i in 0..self.tool.var_count() {
            let slot = self.tool.vars.slot(i);
            match (a.lookup(slot).cloned(), b.lookup(slot).cloned()) {
                (None, None) => {}
                (Some(x), Some(y)) if matches!(x.tlc_equals(self, &y), Ok(true)) => {}
                _ => return false,
            }
        }
        true
    }

    /// `TLCState.equals` as `SetOfStates.put0` uses it: an equality that throws is `false`.
    pub fn states_equal_or_false(&mut self, a: &State, b: &State) -> bool {
        (0..a.len()).all(|i| {
            let slot = self.tool.vars.slot(i);
            let (x, y) = (a.get(slot).clone(), b.get(slot).clone());
            matches!(x.tlc_equals(self, &y), Ok(true))
        })
    }

    // ---- printing ----

    /// `TLCStateMut.toString` (`TLCStateMut.java:327-396`): `x = v` for one variable, else
    /// `/\ x = v` per variable in `vars[]` order; `prev` given = `-difftrace`, which prints
    /// only the variables that changed.
    ///
    /// `-view` with a VIEW prints the view's value instead, but only on the no-argument
    /// `toString()`: `-difftrace` reaches `toString(lastState)`, which has no view branch.
    pub fn print_state(&mut self, s: &State, prev: Option<&State>) -> EvalResult<String> {
        if prev.is_none()
            && self.tool.globals.use_view
            && let Some(view) = &self.tool.view
        {
            let (expr, ctx) = (view.expr, view.ctx.clone());
            return self.with_fast_tool(|ev| {
                let val = ev.eval(expr, &ctx, crate::state::StateRef::Full(s), crate::state::StateRef::Empty, EvalControl::CLEAR)?;
                ev.view_string(expr, &val)
            });
        }
        Ok(self.print_variables(s, prev))
    }

    /// `SemanticNode.toString(IValue)` (`SemanticNode.java:495-497`). `OpApplNode` overrides
    /// it for a tuple whose size equals the node's `allParams`, printing one conjunct per
    /// parameter; the semantic graph does not carry `allParams`, so that branch is not
    /// decidable here.
    fn view_string(&mut self, expr: ExprId, val: &crate::value::Value) -> EvalResult<String> {
        if matches!(val, crate::value::Value::Tuple(_)) {
            let _ = expr;
            return Err(EvalError::unimplemented(
                "printing a tuple-valued VIEW under -view (OpApplNode.toString needs allParams) (M3)".into(),
            ));
        }
        val.ppr(self)
    }

    fn print_variables(&mut self, s: &State, prev: Option<&State>) -> String {
        let interner = self.tool.graph.interner().clone();
        let single = s.len() == 1;
        let mut out = String::new();
        for i in 0..s.len() {
            let slot = self.tool.vars.slot(i);
            let val = s.get(slot).clone();
            if let Some(prev) = prev {
                let old = prev.get(slot).clone();
                if matches!(old.tlc_equals(self, &val), Ok(true)) {
                    continue;
                }
            }
            let name = interner.text(self.tool.vars.names[i]);
            let shown = val.ppr(self).unwrap_or_default();
            if !single {
                out.push_str("/\\ ");
            }
            out.push_str(&format!("{name} = {shown}\n"));
        }
        out
    }
    /// `TLCStateMut.getUnassigned()`, named and sorted as 2109 prints them.
    pub fn unassigned(&self, s1: &PartialState) -> Vec<String> {
        s1.unassigned(&self.tool.vars, self.tool.graph.interner())
    }
    /// A partially assigned state as `toString` prints it: an unassigned variable is a Java
    /// `null`, which `Values.ppr` renders as `null`.
    pub fn print_partial_state(&mut self, s: &PartialState) -> String {
        let interner = self.tool.graph.interner().clone();
        let single = self.tool.var_count() == 1;
        let mut out = String::new();
        for i in 0..self.tool.var_count() {
            let slot = self.tool.vars.slot(i);
            let shown = match s.lookup(slot).cloned() {
                Some(v) => v.ppr(self).unwrap_or_default(),
                None => "null".to_string(),
            };
            let name = interner.text(self.tool.vars.names[i]);
            if !single {
                out.push_str("/\\ ");
            }
            out.push_str(&format!("{name} = {shown}\n"));
        }
        out
    }
    /// `TLCStateInfo.info`: the action's location, or `<Initial predicate>` for a state TLC
    /// recovered from an initial predicate it cannot name (`TLCStateInfo.java:32-63`).
    pub fn state_label(&mut self, step: &StateInfo) -> String {
        match step.action {
            None => "<Initial predicate>".to_string(),
            Some(a) => self.action_label(a),
        }
    }
    /// `Action.getName()`, which 2109 prints when the spec has more than one action.
    pub fn action_name(&mut self, a: ActionId) -> String {
        match self.tool.actions[a.index()].name() {
            Some(n) => self.tool.graph.interner().text(n).to_string(),
            None => "UnnamedAction".to_string(),
        }
    }
    /// `Action.getLocation()`: `<Name(p) line L, col C to line L2, col C2 of module M>`.
    pub fn action_label(&mut self, a: ActionId) -> String {
        let action = &self.tool.actions[a.index()];
        let (pred, op_def, ctx) = (action.pred, action.op_def, action.ctx.clone());
        let name = action.name.map_or_else(|| "Action".to_string(), |n| self.tool.graph.interner().text(n).to_string());
        self.labelled(&name, pred, op_def, &ctx)
    }
    /// `Action.getLocation(name)`: the action's parameters, as bound by splitting, then the
    /// predicate's location.
    pub(crate) fn labelled(&mut self, name: &str, pred: ExprId, op_def: Option<tlars_sany::graph::OpDefId>, ctx: &crate::ctx::Ctx) -> String {
        let graph = self.tool.graph.clone();
        let params = op_def.map(|d| graph.op_def(d).params.clone()).unwrap_or_default();
        let mut shown = Vec::new();
        for &p in params.iter() {
            if let Some(crate::ctx::Binding::Value(v)) = ctx.lookup(tlars_sany::graph::SymbolRef::Param(p)) {
                let v = v.clone();
                shown.push(v.to_tlc_string(self).unwrap_or_default());
            }
        }
        let args = if shown.is_empty() { String::new() } else { format!("({})", shown.join(",")) };
        format!("<{name}{args} {}>", graph.loc(pred).tlc_text(graph.interner()))
    }

    // ---- internal ----

    /// `TLCStateMut.mytool`: the static the `Tool` constructor set, which is always the
    /// `FastTool`. So the VIEW is evaluated by the plain tool even while the failing step is
    /// being replayed under a `CallStackTool`: its sub-expressions leave no frames on the
    /// nested-expression stack and its values carry no source.
    fn with_fast_tool<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = self.locals.call_stack.take();
        let r = f(self);
        self.locals.call_stack = saved;
        r
    }

    /// `IdThread.setCurrentState(s0)`: the next RandomElement draw re-seeds with
    /// `fp(s0) ^ seed`, and `TLCExt!Trace` reads `s0` back. Called by `next_states` and by
    /// trace replay, never by init.
    pub(crate) fn begin_expansion(&mut self, s0: &State, s0_fp: Fp) {
        self.expansions += 1;
        let id = crate::effects::ExpansionId(self.expansions);
        self.locals.effects.current = Some(crate::effects::CurrentState { id, fp: s0_fp, state: s0.clone() });
    }

    /// `IdThread.getCurrentState()`, which is `null` outside a worker's expansion.
    pub(crate) fn current_state(&self) -> Option<&State> {
        self.locals.effects.current.as_ref().map(|c| &c.state)
    }

    /// The `IdThread.setCurrentState(currentState)` that `TLCExt!Trace` runs after a nested
    /// reconstruction moved the slot (`TLCExt.java:274-277`).
    pub(crate) fn restore_expansion(&mut self, saved: Option<crate::effects::CurrentState>) {
        self.locals.effects.current = saved;
    }

    pub(crate) fn saved_expansion(&self) -> Option<crate::effects::CurrentState> {
        self.locals.effects.current.clone()
    }

    /// `Tool.checkAssumptions` (`Tool.java:3645-3663`): the root module's assumptions in
    /// order; the first false one is 2104, the first that fails to evaluate 2105.
    pub fn check_assumptions(&mut self) -> Ec {
        let graph = self.tool.graph.clone();
        for a in graph.module(graph.root()).assumptions.iter() {
            if a.is_axiom {
                continue;
            }
            let loc = graph.expr_tlc_text(a.expr);
            match self.is_valid(a.expr) {
                Ok(true) => {}
                Ok(false) => return self.out.error(Ec::TLC_ASSUMPTION_FALSE, &[&loc]),
                // Not an `Exception`: `TLC.process` reports it (`TLC.java:1414-1418`), and
                // nothing runs in between.
                Err(e) if matches!(e.kind, crate::error::EvalErrorKind::StackOverflow) => {
                    return self.out.error(Ec::SYSTEM_STACK_OVERFLOW, &[&e.message]);
                }
                Err(e) => return self.out.error(Ec::TLC_ASSUMPTION_EVALUATION_ERROR, &[&loc, &e.message]),
            }
        }
        Ec::NO_ERROR
    }

    /// `Tool.isValid(expr, Context.Empty)` (`:3626-3633`).
    fn is_valid(&mut self, expr: ExprId) -> EvalResult<bool> {
        self.is_valid_const(expr, &crate::ctx::Ctx::EMPTY)
    }

    /// `Tool.isValid(ExprNode, Context)` (`:3625-3632`), which evaluates with
    /// `EvalControl.Const` and both states empty.
    fn is_valid_const(&mut self, expr: ExprId, ctx: &crate::ctx::Ctx) -> EvalResult<bool> {
        match self.eval(expr, ctx, crate::state::StateRef::Empty, crate::state::StateRef::Empty, EvalControl::CONST)? {
            crate::value::Value::Bool(b) => Ok(b),
            _ => {
                // `Assert.fail(TLC_EXPECTED_VALUE, {"boolean", expr.toString()}, expr)`: the
                // message is `MP.getMessage(code, params)`.
                let graph = &self.tool.graph;
                let loc = graph.loc(expr);
                let message = self.out.text(Ec::TLC_EXPECTED_VALUE, &["boolean", &graph.expr_tlc_text(expr)]);
                Err(EvalError { ec: Ec::TLC_EXPECTED_VALUE, message, loc: Some(loc), kind: crate::error::EvalErrorKind::Runtime, params: Vec::new() })
            }
        }
    }

    pub(crate) fn enter(&mut self, expr: ExprId) -> EvalResult<()> {
        self.depth += 1;
        if self.depth > MAX_EVAL_DEPTH {
            return Err(self.stack_overflow());
        }
        if let Some(cs) = self.locals.call_stack.as_mut() {
            cs.push(expr);
        }
        Ok(())
    }
    /// The `catch { freeze; throw } finally { pop }` of every `CallStackTool` override. Only
    /// the exceptions TLC catches there freeze the stack; a cast error or a stack overflow
    /// unwinds through the `finally` and leaves nothing behind.
    pub(crate) fn leave(&mut self, failure: Option<&crate::error::EvalErrorKind>) {
        self.depth -= 1;
        let Some(cs) = self.locals.call_stack.as_mut() else { return };
        freeze_for(cs, failure);
        cs.pop();
    }
    /// `setSource(expr, value)` (`Tool.java:2859`): `FastTool` discards it and only
    /// `CallStackTool` records it, so a value carries its creating expression exactly while
    /// the nested-expression stack is being recorded.
    pub(crate) fn set_source(&self, expr: ExprId, value: crate::value::Value) -> crate::value::Value {
        if self.locals.call_stack.is_some() {
            value.set_source(expr);
        }
        value
    }

    /// `callStack.push(node)` around a generator step, released the same way as `leave`.
    /// The depth counter is the same one `enter` uses, because TLC's generators and `eval`
    /// recurse on one native stack: a `NEXT` that only recurses through the generator must
    /// still end in `SYSTEM_STACK_OVERFLOW` and a counterexample, not in a crash.
    pub(crate) fn enter_generator(&mut self, expr: ExprId) -> EvalResult<()> {
        self.depth += 1;
        if self.depth > MAX_EVAL_DEPTH {
            return Err(self.stack_overflow());
        }
        if let Some(cs) = self.locals.call_stack.as_mut() {
            cs.push(expr);
        }
        Ok(())
    }
    pub(crate) fn leave_generator(&mut self, failure: Option<&crate::error::Halt>) {
        self.depth -= 1;
        let Some(cs) = self.locals.call_stack.as_mut() else { return };
        match failure {
            Some(crate::error::Halt::Eval(e)) => freeze_for(cs, Some(&e.kind)),
            _ => freeze_for(cs, None),
        }
        cs.pop();
    }
    /// What the JVM's `StackOverflowError` becomes. It is an `Error`, so the `catch
    /// (Exception)` blocks TLC evaluates under (assumptions, invariants) let it through.
    fn stack_overflow(&self) -> EvalError {
        EvalError { ec: Ec::SYSTEM_STACK_OVERFLOW, message: String::new(), loc: None, kind: crate::error::EvalErrorKind::StackOverflow, params: Vec::new() }
    }
}

/// The `s:_possible` register's value and what `_Possible!_Counts` returns:
/// `new FcnRcdValue(domain, values, false)` over the tracked names.
pub(crate) fn possible_record(counts: Vec<(Atom, i32)>) -> Value {
    let (domain, values): (Vec<Value>, Vec<Value>) = counts.into_iter().map(|(n, c)| (Value::Str(n), Value::Int(c))).unzip();
    Value::FcnRcd(Value::fcn_rcd_arc(domain, values, false))
}
