//! `ModelChecker` + `AbstractChecker`: BFS over a shared FPSet and a shared FIFO queue,
//! with per-worker trace stores and per-worker counters merged when read.
//!
//! Shared-state map (who writes what):
//! - `fpset`, `queue`: shared by design (TLC), internally synchronised.
//! - `traces[i]`, `stats[i]`: written only by worker `i`; read (merged) by the main thread.
//! - `err`: first writer wins (`setErrState`), everyone else observes.
//! - `live`: behaviour graph, appended under its own lock (`synchronized(oos)` in TLC).

mod worker;

use crate::fpset::FpSet;
use crate::liveness::LiveCheck;
use crate::queue::{Queued, StateQueue};
use crate::trace::TraceStores;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64};
use tlars_msg::{Ec, Report};
use tlars_tool::{ActionId, CheckId, Ev, EvalError, Flow, ImpliedAction, Invariant, PartialState, State, Tool};

/// What went wrong first. One variant per `setErrState` cause.
pub(crate) enum ErrState {
    InvariantInitial { id: CheckId<Invariant>, state: State },
    Invariant { id: CheckId<Invariant>, pred: Queued, state: State },
    ImpliedAction { id: CheckId<ImpliedAction>, pred: Queued, state: State },
    Deadlock { state: Queued },
    Incomplete { pred: Queued, action: ActionId },
    Eval { pred: Queued, succ: Option<State>, error: EvalError },
    Liveness,
}

/// `AbstractChecker.setErrState`: only the first error is kept (unless `-continue`).
pub(crate) struct ErrSlot {
    first: Mutex<Option<ErrState>>,
}

impl ErrSlot {
    /// `true` if this call recorded the first error.
    pub(crate) fn set(&self, e: ErrState) -> bool {
        todo!()
    }
}

/// Per-worker counters (`Worker.statesGenerated`, `maxLevel`), summed by the reader.
pub(crate) struct WorkerStats {
    pub(crate) generated: AtomicU64,
    pub(crate) max_level: AtomicU32,
}

pub(crate) struct ModelChecker<'a> {
    tool: &'a Tool,
    out: &'a dyn Report,
    fpset: Box<dyn FpSet>,
    queue: StateQueue,
    traces: TraceStores,
    stats: Box<[WorkerStats]>,
    err: ErrSlot,
    live: Option<LiveCheck>,
    check_deadlock: bool,
    continue_after_violation: bool,
    workers: usize,
    worker_stack_bytes: usize,
    seed: i64,
}

impl<'a> ModelChecker<'a> {
    pub(crate) fn new(tool: &'a Tool, out: &'a dyn Report, fpset: Box<dyn FpSet>, workers: usize, worker_stack_bytes: usize) -> Self {
        todo!()
    }

    /// `ModelChecker.modelCheck`: assumptions, 2189, `do_init`, 2190, `run_tlc`, final
    /// liveness check, POSTCONDITION, success report, summary (2199, 2194, 2268 on success).
    /// On an evaluation error the failing step is re-run with `WorkerLocals::record_call_stack`
    /// to print `TLC_NESTED_EXPRESSION` (TLC's `CallStackTool` replay).
    pub(crate) fn model_check(&mut self) -> Ec {
        todo!()
    }

    /// `doInit` + `DoInitFunctor`: for each initial state in generation order:
    /// isGoodState; if CONSTRAINT holds: fp, FPSet.put, if new: trace record, enqueue,
    /// liveCheck.addInitState; if new *or* out of model: invariants, then implied inits.
    fn do_init(&mut self) -> Result<(), Ec> {
        todo!()
    }

    /// `runTLC`: start workers on large-stack scoped threads, then the main-thread loop
    /// (first progress after 3 s, then every 60 s; periodic liveness when the graph grew
    /// > 10% and liveness used < 20% of runtime, unless `-lncheck final|seqfinal|off`).
    fn run_tlc(&self) -> Ec {
        std::thread::scope(|scope| {
            for id in 0..self.workers {
                std::thread::Builder::new()
                    .name(format!("TLCWorkerThread-{id:03}"))
                    .stack_size(self.worker_stack_bytes)
                    .spawn_scoped(scope, move || worker::Worker::new(id as u16, self).run())
                    .expect("spawn TLC worker");
            }
            self.periodic_loop()
        })
    }

    fn periodic_loop(&self) -> Ec {
        todo!()
    }

    pub(crate) fn set_err(&self, e: ErrState) -> bool {
        self.err.set(e)
    }

    /// Invariants on a new (or out-of-model) successor, in cfg order. Violation: record,
    /// print trace later, and stop generation unless `-continue`. An evaluation failure is
    /// `TLC_INVARIANT_EVALUATION_FAILED` (exit 76), not a next-state failure.
    fn check_invariants(&self, ev: &mut Ev<'_>, cur: &Queued, succ: &State) -> Flow {
        todo!()
    }

    /// Implied actions `[][A]_v`, for *every* successor, seen or not.
    fn check_implied_actions(&self, ev: &mut Ev<'_>, cur: &Queued, succ: &State) -> Flow {
        todo!()
    }

    /// `doNextFailed`: maps the error to its EC (stack overflow, TLC bug, eval code, general),
    /// records it, and stops all workers.
    pub(crate) fn next_failed(&self, cur: &Queued, succ: Option<&State>, e: EvalError) {
        todo!()
    }

    /// `TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT` for `action` from `cur`.
    fn incomplete(&self, cur: &Queued, action: ActionId, s1: &PartialState) -> tlars_tool::Halt {
        todo!()
    }

    /// Worker `id` is out of work for good (`setDone` + notify).
    fn worker_done(&self, id: u16) {
        todo!()
    }
}
