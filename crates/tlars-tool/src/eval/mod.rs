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
mod expr;
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

/// `CallStackTool`'s frames.
#[derive(Default)]
pub struct CallStack {
    frames: Vec<(ExprId, u32)>,
}

impl CallStack {
    /// The indented position list printed under `TLC_NESTED_EXPRESSION`.
    pub fn to_tlc_string(&self, tool: &Tool) -> String {
        todo!()
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
        todo!()
    }
    /// `isInActions`: all ACTION_CONSTRAINTs.
    pub fn is_in_actions(&mut self, s0: &State, s1: &State) -> EvalResult<bool> {
        todo!()
    }
    /// `TLCStateMut.fingerPrint`: deepNormalize, then SYMMETRY (min permuted state compared
    /// in `vars[]` order, early exit on cmp > 0) or VIEW, then FP64 over `vars[]` order.
    pub fn fingerprint(&mut self, s: &State) -> EvalResult<Fp> {
        todo!()
    }
    /// One INVARIANT, `Ok(false)` = violated.
    pub fn check_invariant(&mut self, id: CheckId<Invariant>, s: &State) -> EvalResult<bool> {
        todo!()
    }
    /// One implied action `[][A]_v`, checked for every successor, seen or not.
    pub fn check_implied_action(&mut self, id: CheckId<ImpliedAction>, s0: &State, s1: &State) -> EvalResult<bool> {
        todo!()
    }
    pub fn check_implied_init(&mut self, id: CheckId<ImpliedInit>, s: &State) -> EvalResult<bool> {
        todo!()
    }
    pub fn check_postconditions(&mut self) -> Result<(), (tlars_msg::Ec, Vec<String>)> {
        todo!()
    }

    /// `TLCState.equals` as `SetOfStates.put0` uses it: an equality that throws is `false`.
    pub fn states_equal_or_false(&mut self, a: &State, b: &State) -> bool {
        todo!()
    }

    // ---- printing ----

    /// `TLCStateMut.toString` / ALIAS `PrintTLCState`: `x = v` for one variable, else
    /// `/\ x = v` per variable in `vars[]` order (ALIAS: record fields in token order,
    /// `_format`, `_ALIASEvalError`); `prev` given = `-difftrace`.
    pub fn print_state(&mut self, s: &State, prev: Option<&State>) -> String {
        todo!()
    }
    /// `Action.getLocation()`: `<Name(p) line L, col C to line L2, col C2 of module M>`.
    pub fn action_label(&mut self, a: ActionId) -> String {
        todo!()
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
            cs.frames.push((expr, self.depth));
        }
        Ok(())
    }
    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }
    /// What the JVM's `StackOverflowError` becomes. It is an `Error`, so the `catch
    /// (Exception)` blocks TLC evaluates under (assumptions, invariants) let it through.
    fn stack_overflow(&self) -> EvalError {
        EvalError { ec: Ec::SYSTEM_STACK_OVERFLOW, message: String::new(), loc: None, kind: crate::error::EvalErrorKind::StackOverflow }
    }
}
