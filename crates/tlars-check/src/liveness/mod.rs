//! The graph side of `tlc2.tool.liveness`: `LiveCheck` (behaviour graph, optionally crossed
//! with a tableau) and `LiveWorker` (iterative Tarjan per PEM, lasso construction). The
//! formula side (`Liveness.processLiveness`, PEMs, tableau) is `tlars_tool::LiveSpec`; this
//! module sees only check indices and bit vectors.
//! In memory first; `DiskGraph` arrives behind `BehaviorGraph` later.

mod set_of_states;

pub(crate) use set_of_states::SetOfStates;

use crate::queue::Queued;
use std::sync::Mutex;
use tlars_tool::{Ev, EvalResult, Fp, LiveSpec};

/// Node key `(fp, tableau index)`; edges appended in `SetOfStates` slot order; per-edge
/// check bits pre-evaluated outside the lock (`LiveCheck.addNextState`).
pub(crate) struct BehaviorGraph {}

pub(crate) struct LiveCheck {
    spec: LiveSpec,
    graphs: Box<[Mutex<BehaviorGraph>]>,
}

impl LiveCheck {
    /// One graph per solution; `Ev::live_spec` already printed the branch count.
    pub(crate) fn new(spec: LiveSpec) -> Self {
        todo!()
    }
    pub(crate) fn add_init_state(&self, ev: &mut Ev<'_>, s: &Queued, fp: Fp) -> EvalResult<()> {
        todo!()
    }
    /// Add `cur`'s successors (in slot order), incl. late expansion of already-done states
    /// that reach a new tableau node (`LiveCheck.java:879-951`).
    pub(crate) fn add_next_state(&self, ev: &mut Ev<'_>, cur: &Queued, succs: SetOfStates) -> EvalResult<()> {
        todo!()
    }
    /// `check0` / `finalCheck`: per OOS, per PEM, `LiveWorker.checkSccs` (iterative Tarjan,
    /// EA-action pruning, trivial-SCC skip), then lasso: BFS prefix (`DiskGraph.getPath`),
    /// greedy DFS (`dfsPostFix`) + BFS back (`bfsPostFix`), "Stuttering" / "Back to state",
    /// then `findViolatedProperties`.
    pub(crate) fn check(&self, ev: &mut Ev<'_>, final_check: bool) -> EvalResult<Option<Lasso>> {
        todo!()
    }
}

pub(crate) struct Lasso {
    pub(crate) prefix: Vec<Fp>,
    pub(crate) cycle: Vec<Fp>,
    pub(crate) stuttering: bool,
}
