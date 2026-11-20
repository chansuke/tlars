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

mod enabled;
pub(crate) mod expr;
mod init;
mod next;
mod replay;

use crate::actions::ActionId;
use crate::effects::Effects;
use crate::error::{EvalError, EvalResult, Flow};
use crate::fp64::Fp;
use crate::state::{PartialState, State};
use crate::tool::{CheckId, ImpliedAction, ImpliedInit, Invariant, Tool};
use tlars_msg::{Ec, Report};
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
    /// RNG, registers, current parent: the one effect channel (`effects.rs`).
    pub(crate) effects: Effects,
    /// `TLCGet("level")` of the state being expanded.
    pub(crate) level: u32,
    /// `Some` while re-running a failure under `CallStackTool` semantics.
    pub(crate) call_stack: Option<CallStack>,
}

impl WorkerLocals {
    pub fn new(worker_id: u16, seed: i64) -> Self {
        Self { worker_id, effects: Effects::new(seed), level: 0, call_stack: None }
    }
    pub fn set_level(&mut self, level: u32) {
        self.level = level;
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
        Ok(true)
    }
    /// `TLCStateMut.fingerPrint` (`TLCStateMut.java:171-237`): with no SYMMETRY and no VIEW,
    /// the values' own fingerprints in `vars[]` order. Each value normalises itself on the
    /// way, which is why fingerprinting a state also leaves it shareable.
    pub fn fingerprint(&mut self, s: &State) -> EvalResult<Fp> {
        if self.tool.has_symmetry() || self.tool.view.is_some() {
            return Err(crate::error::EvalError::unimplemented("SYMMETRY and VIEW fingerprints (M3 unit 5)".into()));
        }
        let mut fp = self.tool.fp64.new_fp();
        for i in 0..s.len() {
            let v = s.get(self.tool.vars.slot(i)).clone();
            fp = v.fingerprint(self, fp)?;
        }
        Ok(fp)
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
    pub fn check_postconditions(&mut self) -> Result<(), (tlars_msg::Ec, Vec<String>)> {
        debug_assert!(self.tool.postconditions.is_empty(), "POSTCONDITION is not processed yet");
        Ok(())
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
                Err(EvalError { ec: Ec::TLC_EXPECTED_VALUE, message, loc: Some(loc), kind: crate::error::EvalErrorKind::Runtime })
            }
        }
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
    pub fn print_state(&mut self, s: &State, prev: Option<&State>) -> String {
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

    /// `IdThread.setCurrentState(s0)`: the next RandomElement draw re-seeds with
    /// `fp(s0) ^ seed`. Called by `next_states` and by trace replay, never by init.
    pub(crate) fn begin_expansion(&mut self, s0_fp: Fp) {
        self.expansions += 1;
        self.locals.effects.current = Some((crate::effects::ExpansionId(self.expansions), s0_fp));
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
        match self.eval(expr, &crate::ctx::Ctx::EMPTY, crate::state::StateRef::Empty, crate::state::StateRef::Empty, EvalControl::CONST)? {
            crate::value::Value::Bool(b) => Ok(b),
            _ => {
                // `Assert.fail(TLC_EXPECTED_VALUE, {"boolean", expr.toString()}, expr)`: the
                // message is `MP.getMessage(code, params)`.
                let graph = &self.tool.graph;
                let loc = graph.loc(expr);
                let message = self.out.text(Ec::TLC_EXPECTED_VALUE, &["boolean", &graph.expr_tlc_text(expr)]);
                Err(EvalError { ec: Ec::TLC_EXPECTED_VALUE, message, loc: Some(loc), kind: crate::error::EvalErrorKind::Runtime })
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
    pub(crate) fn enter_generator(&mut self, expr: ExprId) {
        if let Some(cs) = self.locals.call_stack.as_mut() {
            cs.push(expr);
        }
    }
    pub(crate) fn leave_generator(&mut self, failure: Option<&crate::error::Halt>) {
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
        EvalError { ec: Ec::SYSTEM_STACK_OVERFLOW, message: String::new(), loc: None, kind: crate::error::EvalErrorKind::StackOverflow }
    }
}
