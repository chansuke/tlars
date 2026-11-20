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
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicU32, AtomicU64};
use tlars_msg::{Ec, Report};
use tlars_tool::{ActionId, CheckId, Ev, EvalError, Flow, Halt, ImpliedAction, Invariant, PartialState, State, Tool, WorkerLocals};

/// `ModelChecker.modelCheck` (`ModelChecker.java:133-352`), with `TLC.process`'s `RunMode`
/// branch above it. The caller already printed the banner.
pub(crate) fn model_check(tool: &Tool, session: &crate::session::Session<'_>) -> Ec {
    let out = &session.reporter;
    let fpset = crate::fpset::from_options(session.options.fp_bits, None);
    let mut mc = ModelChecker::new(tool, out, fpset, session);
    mc.model_check()
}

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

/// `AbstractChecker.setErrState(curState, succState, keepCallStack, errorCode)`: the cause,
/// the error code it reports, and whether TLC re-runs the step to print the nested
/// expressions.
pub(crate) struct Recorded {
    state: ErrState,
    ec: Ec,
    keep_call_stack: bool,
}

/// `AbstractChecker.setErrState`: only the first error is kept (unless `-continue`).
pub(crate) struct ErrSlot {
    first: Mutex<Option<Recorded>>,
}

impl ErrSlot {
    /// `true` if this call recorded the first error.
    pub(crate) fn set(&self, state: ErrState, ec: Ec, keep_call_stack: bool) -> bool {
        let mut slot = self.first.lock().expect("error slot");
        if slot.is_some() {
            return false;
        }
        *slot = Some(Recorded { state, ec, keep_call_stack });
        true
    }
}

/// Per-worker counters (`Worker.statesGenerated`, `maxLevel`), summed by the reader.
pub(crate) struct WorkerStats {
    pub(crate) generated: AtomicU64,
    pub(crate) max_level: AtomicU32,
    /// `Worker.outDegree`: a `FixedSizedBucketStatistics` of 32 buckets over the number of
    /// *unseen* successors per expanded state, the last bucket holding the overflow.
    pub(crate) out_degree: Mutex<[u64; 32]>,
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
    /// `ModelChecker.numberOfInitialStates`.
    initial_states: AtomicU64,
    /// `TLC.startTime` and the run's clock, for the throughput in the progress messages.
    start_millis: i64,
    clock: crate::Clock,
    /// `ModelChecker.oldNumOfGenStates` / `oldFPSetSize`.
    old_generated: AtomicU64,
    old_distinct: AtomicU64,
}

impl<'a> ModelChecker<'a> {
    pub(crate) fn new(
        tool: &'a Tool,
        out: &'a dyn Report,
        fpset: Box<dyn FpSet>,
        session: &crate::session::Session<'_>,
    ) -> ModelChecker<'a> {
        let workers = tool.globals().workers.max(1);
        ModelChecker {
            tool,
            out,
            fpset,
            queue: StateQueue::new(workers),
            traces: TraceStores::new(workers),
            stats: (0..workers)
                .map(|_| WorkerStats { generated: AtomicU64::new(0), max_level: AtomicU32::new(0), out_degree: Mutex::new([0; 32]) })
                .collect(),
            err: ErrSlot { first: Mutex::new(None) },
            live: None,
            check_deadlock: tool.globals().check_deadlock,
            continue_after_violation: tool.globals().continue_after_violation,
            workers,
            worker_stack_bytes: session.env.worker_stack_bytes,
            seed: session.seed,
            initial_states: AtomicU64::new(0),
            start_millis: session.start_millis,
            clock: session.env.clock.clone(),
            old_generated: AtomicU64::new(0),
            old_distinct: AtomicU64::new(0),
        }
    }

    /// `AbstractChecker.modelCheck` (`AbstractChecker.java:618-621`): the recorded error code
    /// stands in whenever `modelCheckImpl` itself came back clean, which is how an invariant
    /// that failed to evaluate still ends the run with its own status.
    pub(crate) fn model_check(&mut self) -> Ec {
        let result = self.model_check_impl();
        if result != Ec::NO_ERROR { result } else { self.error_code() }
    }

    /// `ModelChecker.modelCheckImpl`: assumptions, 2189, `do_init`, 2190/2191, `run_tlc`,
    /// final liveness check, success report, summary (2199, 2194, 2268 on success).
    fn model_check_impl(&mut self) -> Ec {
        let mut locals = WorkerLocals::new(0, self.seed);
        let assumptions = self.tool.ev(&mut locals, self.out).check_assumptions();
        if assumptions != Ec::NO_ERROR {
            return assumptions;
        }
        if self.tool.has_liveness() {
            return self.out.error(Ec::GENERAL, &["tlars does not implement liveness checking yet (M4)"]);
        }
        self.out.message(Ec::TLC_COMPUTING_INIT, &[]);
        let mut result = match self.do_init() {
            Ok(()) => Ec::NO_ERROR,
            // `modelCheckImpl` returns an error code the functor recorded without printing
            // the summary, and prints it only on the exception path
            // (`ModelChecker.java:155-172` against `:219-221`).
            Err((ec, summary)) => {
                if summary {
                    self.print_summary(false);
                }
                return ec;
            }
        };
        let generated = self.states_generated();
        let plural = if generated == 1 { "" } else { "s" };
        let distinct = self.fpset.size();
        if generated == distinct {
            self.out.message(Ec::TLC_INIT_GENERATED1, &[&generated.to_string(), plural]);
        } else {
            self.out.message(Ec::TLC_INIT_GENERATED2, &[&generated.to_string(), plural, &distinct.to_string()]);
        }
        // Finished if there is no next-state predicate.
        if self.tool.actions().is_empty() {
            if self.queue.is_empty() {
                self.report_success();
                self.print_summary(true);
            } else {
                result = self.out.error(Ec::TLC_STATES_AND_NO_NEXT_ACTION, &[]);
            }
            return result;
        }
        self.run_tlc();
        // `runTLC` returns the recorded code unless a call-stack replay was asked for.
        let replay = self.call_stack_replay();
        result = match &replay {
            Some(_) => Ec::NO_ERROR,
            None => self.error_code(),
        };
        if result != Ec::NO_ERROR {
            self.print_summary(false);
            return result;
        }
        let errored = self.err.first.lock().expect("error slot").is_some();
        if !errored {
            self.report_success();
        } else if let Some(pred) = replay {
            result = self.replay_next_call_stack(&pred);
        }
        self.print_summary(result == Ec::NO_ERROR);
        result
    }

    /// `doInit` + `DoInitFunctor`: for each initial state in generation order:
    /// isGoodState; if CONSTRAINT holds: fp, FPSet.put, if new: trace record, enqueue,
    /// liveCheck.addInitState; if new *or* out of model: invariants, then implied inits.
    fn do_init(&mut self) -> Result<(), (Ec, bool)> {
        let mut locals = WorkerLocals::new(0, self.seed);
        let mut sink = InitSink { mc: self, result: Ec::NO_ERROR, stopped: false, force_checks: false, failed_state: None, kept: None };
        let flow = {
            let mut ev = self.tool.ev(&mut locals, self.out);
            ev.init_states(&mut sink)
        };
        let (result, errored) = (sink.result, sink.failed_state);
        // `doInit`: an exception the functor kept is rethrown once generation has finished.
        let flow = match (flow, sink.kept) {
            (Ok(()) | Err(Halt::Stop), Some(e)) => Err(Halt::Eval(e)),
            (flow, _) => flow,
        };
        match flow {
            Ok(()) | Err(Halt::Stop) => {}
            Err(Halt::Eval(e)) => {
                // `modelCheckImpl`'s `catch (Throwable e)` around `doInit`: the state the
                // functor was working on, if any, is printed with the message.
                let ec = eval_error_code(&e);
                // `getMessage()` of a `StackOverflowError` is null, so TLC substitutes its
                // own explanation (`ModelChecker.java:182-190`).
                let message = match e.kind {
                    tlars_tool::EvalErrorKind::StackOverflow => STACK_OVERFLOW_MESSAGE.to_string(),
                    _ => e.message.clone(),
                };
                // `doInit` carries the functor's state to the checker only for the
                // exceptions it catches by name (`ModelChecker.java:390-402`): an
                // `EvalException` passes straight through and leaves `errState` null, so the
                // message is the general one and names no state.
                let errored = errored.filter(|_| !matches!(e.kind, tlars_tool::EvalErrorKind::Eval));
                match errored {
                    Some(state) => {
                        let shown = {
                            let mut ev = self.tool.ev(&mut locals, self.out);
                            ev.print_state(&state, None)
                        };
                        self.out.error(Ec::TLC_INITIAL_STATE, &[&message, &shown]);
                    }
                    None => {
                        self.out.error(Ec::GENERAL, &[&message]);
                    }
                }
                return Err((self.replay_init_call_stack(ec), true));
            }
        }
        if result != Ec::NO_ERROR {
            return Err((result, false));
        }
        Ok(())
    }

    /// `doInit(cTool, true)`: TLC re-runs the whole initial-state generation under a
    /// `CallStackTool`, with the invariant checks forced on states already seen. If the
    /// re-run fails again, `TLC_NESTED_EXPRESSION` (2103) becomes the result.
    fn replay_init_call_stack(&mut self, ec: Ec) -> Ec {
        self.initial_states.store(0, Relaxed);
        let mut locals = WorkerLocals::new(0, self.seed);
        locals.record_call_stack();
        let mut sink = InitSink { mc: self, result: Ec::NO_ERROR, stopped: false, force_checks: true, failed_state: None, kept: None };
        let flow = {
            let mut ev = self.tool.ev(&mut locals, self.out);
            ev.init_states(&mut sink)
        };
        // As in `doInit`, an exception the functor kept surfaces after generation.
        let flow = match (flow, sink.kept) {
            (Ok(()) | Err(Halt::Stop), Some(e)) => Err(Halt::Eval(e)),
            (flow, _) => flow,
        };
        let stack = locals.take_call_stack().expect("recording was switched on");
        let text = stack.to_tlc_string(self.tool);
        match flow {
            Err(Halt::Eval(e)) if matches!(e.kind, tlars_tool::EvalErrorKind::Fingerprint { .. }) => {
                self.out.error(Ec::TLC_FINGERPRINT_EXCEPTION, &[&text, &e.message])
            }
            Err(Halt::Eval(_)) => self.out.error(Ec::TLC_NESTED_EXPRESSION, &[&text]),
            _ => ec,
        }
    }

    /// `runTLC`: start workers on large-stack scoped threads, then the main-thread loop
    /// (first progress after 3 s, then every 60 s; periodic liveness when the graph grew
    /// > 10% and liveness used < 20% of runtime, unless `-lncheck final|seqfinal|off`).
    fn run_tlc(&self) {
        std::thread::scope(|scope| {
            for id in 0..self.workers {
                std::thread::Builder::new()
                    .name(format!("TLCWorkerThread-{id:03}"))
                    .stack_size(self.worker_stack_bytes)
                    .spawn_scoped(scope, move || worker::Worker::new(id as u16, self).run())
                    .expect("spawn TLC worker");
            }
            self.periodic_loop();
        });
    }

    /// `AbstractChecker.runTLC`'s main-thread loop (`:446-478`): the first progress report
    /// after 3 s, then one every `progressInterval`. A run that finishes inside the first
    /// wait prints none, which is why the recordings hold no mid-run progress line.
    /// `doPeriodicWork` has nothing to do without liveness or checkpointing.
    fn periodic_loop(&self) {
        if self.queue.wait_finished(std::time::Duration::from_millis(3000)) {
            return;
        }
        while !self.queue.is_finished() {
            self.print_progress();
            if self.queue.wait_finished(std::time::Duration::from_millis(PROGRESS_INTERVAL_MILLIS)) {
                return;
            }
        }
    }

    /// `printProgresStats(-1, false)` (`ModelChecker.java:1009-1035`): the rates are over one
    /// progress interval, and the counters it compares against move on.
    fn print_progress(&self) {
        let generated = self.states_generated();
        let distinct = self.fpset.size();
        let factor = PROGRESS_INTERVAL_MILLIS as f64 / 60_000.0;
        let spm = rate(generated, self.old_generated.swap(generated, Relaxed), factor);
        let dspm = rate(distinct, self.old_distinct.swap(distinct, Relaxed), factor);
        let fmt = tlars_java::decimal_format_grouped;
        self.out.message(
            Ec::TLC_PROGRESS_STATS,
            &[
                &self.traces.level().to_string(),
                &fmt(generated as i64),
                &fmt(distinct as i64),
                &fmt(self.queue.len() as i64),
                &fmt(spm),
                &fmt(dspm),
            ],
        );
    }

    /// `setErrState` with the error code and the call-stack flag the caller chose.
    pub(crate) fn set_err_with(&self, e: ErrState, ec: Ec, keep_call_stack: bool) -> bool {
        self.err.set(e, ec, keep_call_stack)
    }

    /// `setErrState(cur, succ, false, ec)`: a violation, which needs no replay.
    pub(crate) fn set_err(&self, e: ErrState) -> bool {
        let ec = match &e {
            ErrState::InvariantInitial { .. } => Ec::TLC_INVARIANT_VIOLATED_INITIAL,
            ErrState::Invariant { .. } => Ec::TLC_INVARIANT_VIOLATED_BEHAVIOR,
            ErrState::ImpliedAction { .. } => Ec::TLC_ACTION_PROPERTY_VIOLATED_BEHAVIOR,
            ErrState::Deadlock { .. } => Ec::TLC_DEADLOCK_REACHED,
            ErrState::Incomplete { .. } => Ec::TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT,
            ErrState::Eval { error, .. } => eval_error_code(error),
            ErrState::Liveness => Ec::NO_ERROR,
        };
        self.err.set(e, ec, false)
    }

    /// Invariants on a new (or out-of-model) successor, in cfg order. Violation: record,
    /// print trace later, and stop generation unless `-continue`. An evaluation failure is
    /// `TLC_INVARIANT_EVALUATION_FAILED` (exit 76), not a next-state failure.
    fn check_invariants(&self, ev: &mut Ev<'_>, cur: &Queued, succ: &State, written: bool) -> Flow {
        for id in self.tool.invariants() {
            let name = self.tool.invariant_name(id);
            let name = ev.tool().graph().interner().text(name).to_string();
            match ev.check_invariant(id, succ) {
                Ok(true) => {}
                Ok(false) => {
                    if self.continue_after_violation {
                        self.out.error(Ec::TLC_INVARIANT_VIOLATED_BEHAVIOR, &[&name]);
                        self.print_trace(ev, cur, Some(Succ::Complete(succ)));
                        continue;
                    }
                    if self.set_err(ErrState::Invariant { id, pred: cur.clone(), state: succ.clone() }) {
                        self.out.error(Ec::TLC_INVARIANT_VIOLATED_BEHAVIOR, &[&name]);
                        self.print_trace_and_stop(ev, cur, Some(succ));
                        self.post_condition_trace(ev, cur, Some(Succ::Complete(succ)), written);
                    }
                    return Err(Halt::Stop);
                }
                Err(e) if is_java_error(&e) => return Err(Halt::Eval(e)),
                Err(e) => {
                    // `doNextEvalFailed`: keeps the call stack and reports the invariant's code.
                    let recorded = ErrState::Eval { pred: cur.clone(), succ: Some(succ.clone()), error: e.clone() };
                    if self.set_err_with(recorded, Ec::TLC_INVARIANT_EVALUATION_FAILED, true) {
                        self.out.error(Ec::TLC_INVARIANT_EVALUATION_FAILED, &[&name, &e.message]);
                        self.print_trace_and_stop(ev, cur, Some(succ));
                    }
                    return Err(Halt::Eval(e));
                }
            }
        }
        Ok(())
    }

    /// Implied actions `[][A]_v`, for *every* successor, seen or not.
    fn check_implied_actions(&self, ev: &mut Ev<'_>, cur: &Queued, succ: &State, written: bool) -> Flow {
        for id in self.tool.implied_actions() {
            let name = self.tool.implied_action_name(id);
            let name = ev.tool().graph().interner().text(name).to_string();
            match ev.check_implied_action(id, &cur.state, succ) {
                Ok(true) => {}
                Ok(false) => {
                    if self.continue_after_violation {
                        self.out.error(Ec::TLC_ACTION_PROPERTY_VIOLATED_BEHAVIOR, &[&name]);
                        self.print_trace(ev, cur, Some(Succ::Complete(succ)));
                        continue;
                    }
                    if self.set_err(ErrState::ImpliedAction { id, pred: cur.clone(), state: succ.clone() }) {
                        self.out.error(Ec::TLC_ACTION_PROPERTY_VIOLATED_BEHAVIOR, &[&name]);
                        self.print_trace_and_stop(ev, cur, Some(succ));
                        self.post_condition_trace(ev, cur, Some(Succ::Complete(succ)), written);
                    }
                    return Err(Halt::Stop);
                }
                Err(e) if is_java_error(&e) => return Err(Halt::Eval(e)),
                Err(e) => {
                    let recorded = ErrState::Eval { pred: cur.clone(), succ: Some(succ.clone()), error: e.clone() };
                    if self.set_err_with(recorded, Ec::TLC_ACTION_PROPERTY_EVALUATION_FAILED, true) {
                        self.out.error(Ec::TLC_ACTION_PROPERTY_EVALUATION_FAILED, &[&name, &e.message]);
                        self.print_trace_and_stop(ev, cur, Some(succ));
                    }
                    return Err(Halt::Eval(e));
                }
            }
        }
        Ok(())
    }

    /// `doNextFailed`: maps the error to its EC (stack overflow, TLC bug, eval code, general),
    /// records it, and stops all workers.
    pub(crate) fn next_failed(&self, cur: &Queued, succ: Option<&State>, e: EvalError) {
        let ec = eval_error_code(&e);
        // `doNextFailed`: a stack overflow, an out-of-memory or an assertion error keeps no
        // call stack, because its replay would not reproduce the same frames.
        let keep = !matches!(e.kind, tlars_tool::EvalErrorKind::StackOverflow | tlars_tool::EvalErrorKind::Assertion);
        if self.set_err_with(ErrState::Eval { pred: cur.clone(), succ: succ.cloned(), error: e.clone() }, ec, keep) {
            if !(ec == Ec::GENERAL && e.message.is_empty()) {
                // `MP.printError(ec, e)` with the exception itself: under `EC.GENERAL` that
                // is `ECGeneralMsg("", throwable)`, not the bare message.
                if ec == Ec::GENERAL {
                    let body = tlars_msg::unexpected_exception(java_class(&e), &e.message);
                    self.out.error(Ec::GENERAL, &[&body]);
                } else if e.params.is_empty() {
                    // `MP.printError(ec, e)`: the exception's own message as the parameter.
                    self.out.error(ec, &[&e.message]);
                } else {
                    // `MP.printError(ec, ((EvalException) e).getParameters(), e)`: the
                    // template is rendered from the parameters, not from the pretty-printed
                    // message the exception already built.
                    let params: Vec<&str> = e.params.iter().map(String::as_str).collect();
                    self.out.error(ec, &params);
                }
            }
            let mut locals = WorkerLocals::new(0, self.seed);
            let mut ev = self.tool.ev(&mut locals, self.out);
            self.print_trace_and_stop(&mut ev, cur, succ);
        }
    }

    /// `TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT` for `action` from `cur`.
    fn incomplete(&self, cur: &Queued, action: ActionId, s1: &PartialState) -> tlars_tool::Halt {
        if self.set_err(ErrState::Incomplete { pred: cur.clone(), action }) {
            let mut locals = WorkerLocals::new(0, self.seed);
            let mut ev = self.tool.ev(&mut locals, self.out);
            let unassigned = ev.unassigned(s1);
            let plural = if unassigned.len() > 1 { "s are" } else { " is" };
            let names = unassigned.join(", ");
            if self.tool.actions().len() == 1 {
                self.out.error(Ec::TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT, &[plural, &names]);
            } else {
                let label = ev.action_name(action);
                self.out.error(Ec::TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT, &[&label, plural, &names]);
            }
            self.print_trace(&mut ev, cur, Some(Succ::Partial(s1)));
            self.queue.finish_all();
            self.post_condition_trace(&mut ev, cur, Some(Succ::Partial(s1)), false);
        }
        Halt::Stop
    }

    /// Worker `id` is out of work for good (`setDone` + notify).
    fn worker_done(&self, _id: u16) {
        self.queue.finish_all();
    }

    // ---- reporting ----

    /// `getStatesGenerated`: the initial states plus every worker's count.
    fn states_generated(&self) -> u64 {
        self.initial_states.load(Relaxed) + self.stats.iter().map(|s| s.generated.load(Relaxed)).sum::<u64>()
    }

    /// `reportSuccess(theFPSet, getStatesGenerated())` (`ModelChecker.java:960-971`).
    fn report_success(&self) {
        let distinct = self.fpset.size();
        let generated = self.states_generated();
        let optimistic = optimistic_probability(distinct, generated);
        if optimistic < 1e-10 {
            self.out.message(Ec::TLC_SUCCESS, &[&format!("val = {}", probability_to_string(optimistic, 2))]);
            return;
        }
        if distinct == 0 && generated == 0 {
            self.out.message(Ec::TLC_SUCCESS, &["val = 0.0", "val = 0.0"]);
            return;
        }
        let distance = self.fpset.min_distance().unwrap_or(u64::MAX);
        let actual = tlars_java::reciprocal_two_digits(distance as i64);
        self.out.message(
            Ec::TLC_SUCCESS,
            &[&format!("val = {}", probability_to_string(optimistic, 2)), &format!("val = {}", probability_to_string(actual, 2))],
        );
    }

    /// `printSummary` (`ModelChecker.java:893-932`).
    fn print_summary(&self, success: bool) {
        let generated = self.states_generated();
        let distinct = self.fpset.size();
        let queued = self.queue.len();
        if self.tool.globals().tool_mode {
            // `printProgresStats(startTime, true)`: the rates are over the whole run, and the
            // counters it compares against are reset to zero first.
            let factor = (self.clock.now_millis() - self.start_millis) as f64 / 60_000.0;
            let spm = rate(generated, 0, factor);
            let dspm = rate(distinct, 0, factor);
            let fmt = tlars_java::decimal_format_grouped;
            self.out.message(
                Ec::TLC_PROGRESS_STATS,
                &[
                    &self.traces.level().to_string(),
                    &fmt(generated as i64),
                    &fmt(distinct as i64),
                    &fmt(queued as i64),
                    &fmt(spm),
                    &fmt(dspm),
                ],
            );
        }
        self.out.message(Ec::TLC_STATS, &[&generated.to_string(), &distinct.to_string(), &queued.to_string()]);
        let depth = if generated == 0 { 0 } else { self.traces.level() };
        self.out.message(Ec::TLC_SEARCH_DEPTH, &[&depth.to_string()]);
        if success {
            let mut buckets = [0u64; 32];
            for s in self.stats.iter() {
                for (i, n) in s.out_degree.lock().expect("out degree").iter().enumerate() {
                    buckets[i] += n;
                }
            }
            if let Some(stats) = OutDegree::of(&buckets) {
                self.out.message(
                    Ec::TLC_STATE_GRAPH_OUTDEGREE,
                    &[&stats.min.to_string(), &stats.mean.to_string(), &stats.p95.to_string(), &stats.max.to_string()],
                );
            }
        }
    }

    /// `AbstractChecker.errorCode`, as `setErrState` recorded it.
    fn error_code(&self) -> Ec {
        self.err.first.lock().expect("error slot").as_ref().map_or(Ec::NO_ERROR, |r| r.ec)
    }

    /// The step to re-run under a `CallStackTool`, and whether TLC asks for that re-run.
    fn call_stack_replay(&self) -> Option<Queued> {
        let slot = self.err.first.lock().expect("error slot");
        let recorded = slot.as_ref()?;
        if !recorded.keep_call_stack {
            return None;
        }
        match &recorded.state {
            ErrState::Eval { pred, .. } => Some(pred.clone()),
            _ => None,
        }
    }

    /// `modelCheckImpl`'s `keepCallStack` branch (`ModelChecker.java:299-318`): re-run the
    /// failing step with the call stack recorded and print `TLC_NESTED_EXPRESSION`.
    fn replay_next_call_stack(&self, pred: &Queued) -> Ec {
        let mut locals = WorkerLocals::new(0, self.seed);
        locals.record_call_stack();
        let flow = self.replay_do_next(&mut locals, pred);
        let stack = locals.take_call_stack().expect("recording was switched on");
        let text = stack.to_tlc_string(self.tool);
        match flow {
            Err(Halt::Eval(e)) if matches!(e.kind, tlars_tool::EvalErrorKind::Fingerprint { .. }) => {
                self.out.error(Ec::TLC_FINGERPRINT_EXCEPTION, &[&text, &e.message])
            }
            // An `EvalException` keeps its own code; anything else reports 2103.
            Err(Halt::Eval(e)) if matches!(e.kind, tlars_tool::EvalErrorKind::Eval) => {
                self.out.error(Ec::TLC_NESTED_EXPRESSION, &[&text]);
                e.ec
            }
            Err(Halt::Eval(_)) => self.out.error(Ec::TLC_NESTED_EXPRESSION, &[&text]),
            _ => Ec::NO_ERROR,
        }
    }

    /// `printTraceAndStop`: the trace, then every worker is told to stop.
    fn print_trace_and_stop(&self, ev: &mut Ev<'_>, cur: &Queued, succ: Option<&State>) {
        self.print_trace(ev, cur, succ.map(Succ::Complete));
        self.queue.finish_all();
    }

    /// `Worker.doPostCondition` (`Worker.java:636-680`), which every `Worker.doNextSetErr`
    /// runs after the counterexample has been printed. It rebuilds the trace even when the
    /// spec has no POSTCONDITION, because only the final
    /// `checkPostConditionWithCounterExample` is a no-op without one (`Tool.java:3671-3679`).
    /// The rebuild regenerates states, so anything the initial predicate or the next-state
    /// relation prints is printed again.
    pub(crate) fn post_condition_trace(&self, ev: &mut Ev<'_>, cur: &Queued, succ: Option<Succ<'_>>, written: bool) {
        let fps = self.traces.fingerprints_to(cur.ptr);
        let prefix_of_cur = |ev: &mut Ev<'_>| {
            // `tlc.getTraceInfo(curState)` followed by `tool.getState(curState, last)`.
            if let Ok(Some(steps)) = crate::trace::reconstruct(ev, &fps[..fps.len() - 1])
                && let Some(last) = steps.last()
            {
                let (pred, pred_fp) = (last.state.clone(), fps[fps.len() - 2]);
                let _ = ev.successor_from_fp(&pred, pred_fp, cur.fp);
            }
        };
        match succ {
            // `curState.isInitial() && succState == null`: the initial state alone.
            None if fps.len() <= 1 => {
                let _ = ev.state_from_fp(cur.fp);
            }
            None => prefix_of_cur(ev),
            // An incomplete successor has no trace record, and `getTraceInfo` fails on it
            // before anything is regenerated.
            Some(Succ::Partial(_)) => {}
            Some(Succ::Complete(state)) => {
                let Ok(fp) = ev.fingerprint(state) else { return };
                if written {
                    // `getTraceInfo(succState)` is the trace up to and including `cur`.
                    let _ = crate::trace::reconstruct(ev, &fps);
                } else if fps.len() <= 1 {
                    // Fully assigned but never written, from an initial state.
                    let _ = ev.state_from_fp(cur.fp);
                } else {
                    prefix_of_cur(ev);
                }
                let _ = ev.successor_from_fp(&cur.state, cur.fp, fp);
            }
        }
    }

    /// `ModelChecker.doNext` (`ModelChecker.java:416-480`), which the replay runs instead of
    /// `Worker.run`. It generates every successor of an action into a vector first and then
    /// runs the checks, so a failure in a CONSTRAINT or an invariant happens with no
    /// next-state generator frame on the nested-expression stack.
    fn replay_do_next(&self, locals: &mut WorkerLocals, cur: &Queued) -> Flow {
        let count = self.tool.actions().len();
        for i in 0..count {
            let id = self.tool.actions()[i].id();
            let mut succs = Vec::new();
            let mut ev = self.tool.ev(locals, self.out);
            ev.next_states_of(&cur.state, id, &mut Collect { states: &mut succs })?;
            for succ in succs.iter() {
                let in_model = ev.is_in_model(succ)? && ev.is_in_actions(&cur.state, succ)?;
                let mut unseen = true;
                if in_model {
                    let fp = ev.fingerprint(succ)?;
                    unseen = self.fpset.put(fp) == crate::fpset::Seen::New;
                }
                if unseen {
                    self.check_invariants(&mut ev, cur, succ, unseen)?;
                }
                self.check_implied_actions(&mut ev, cur, succ, unseen)?;
            }
        }
        Ok(())
    }

    /// `ConcurrentTLCTrace.printTrace(s1, s2)` (`TLCTrace.java:349-443`): 2121, then the
    /// reconstructed prefix, `s1` and `s2`, each printed as 2217.
    fn print_trace(&self, ev: &mut Ev<'_>, cur: &Queued, succ: Option<Succ<'_>>) {
        let fps = self.traces.fingerprints_to(cur.ptr);
        // An initial `s1` is printed as it stands, with no regeneration at all
        // (`TLCTrace.java:350-370`).
        if fps.len() <= 1 {
            self.out.error(Ec::TLC_BEHAVIOR_UP_TO_THIS_POINT, &[]);
            let step = tlars_tool::StateInfo { state: cur.state.clone(), action: None };
            self.print_state(ev, &step, None, 1);
            if let Some(succ) = succ {
                self.print_last(ev, &cur.state, cur.fp, succ, Some(&cur.state), 2);
            }
            return;
        }
        // `getTrace(s1)` is the *argument* of the three-parameter `printTrace`
        // (`ConcurrentTLCTrace.java:208-214`), so the prefix is reconstructed before 2121 is
        // printed and everything those steps print comes first.
        let reconstructed = crate::trace::reconstruct(ev, &fps[..fps.len() - 1]);
        self.out.error(Ec::TLC_BEHAVIOR_UP_TO_THIS_POINT, &[]);
        let prefix = match reconstructed {
            Ok(Some(steps)) => steps,
            Ok(None) => {
                self.out.error(Ec::TLC_FAILED_TO_RECOVER_INIT, &[]);
                return;
            }
            Err(e) => {
                self.out.error(Ec::GENERAL, &[&e.message]);
                return;
            }
        };
        let mut printed: Option<State> = None;
        let mut number = 0;
        for step in prefix.iter() {
            number += 1;
            self.print_state(ev, step, printed.as_ref(), number);
            printed = Some(step.state.clone());
        }
        // `s1` itself is regenerated from the last prefix state only *after* that state has
        // been printed, which is why a `Print` in the next-state relation shows up between
        // the two.
        let pred = prefix.last().expect("a prefix of at least one state").state.clone();
        let pred_fp = fps[fps.len() - 2];
        number += 1;
        if !self.print_successor(ev, &pred, pred_fp, Succ::Complete(&cur.state), printed.as_ref(), number) {
            return;
        }
        if let Some(succ) = succ {
            self.print_last(ev, &cur.state, cur.fp, succ, Some(&cur.state), number + 1);
        }
    }

    /// `getState(fp, pred)` followed by the state's own line: the action label is the first
    /// match, as during the search.
    /// `getState(s2, s1)` (`TLCTrace.java:426-440`): the *last* state of a counterexample is
    /// found by value equality, not by fingerprint, so two successors that share a
    /// fingerprint cannot be swapped. An incomplete successor is matched the same way and
    /// printed with its unassigned variables as `null`.
    fn print_last(
        &self,
        ev: &mut Ev<'_>,
        pred: &State,
        pred_fp: tlars_tool::Fp,
        succ: Succ<'_>,
        printed: Option<&State>,
        number: usize,
    ) -> bool {
        let target = match succ {
            Succ::Complete(state) => tlars_tool::PartialState::from_state(state),
            Succ::Partial(s1) => s1.copy_out(),
        };
        match ev.successor_matching(pred, pred_fp, &target) {
            Ok(Some((state, action))) => {
                match ev.complete(&state) {
                    Some(complete) => {
                        let step = tlars_tool::StateInfo { state: complete, action: Some(action) };
                        self.print_state(ev, &step, printed, number);
                    }
                    None => {
                        // An incomplete state has no fingerprint, so TLC prints `-1`.
                        let label = ev.action_label(action);
                        let shown = ev.print_partial_state(&state);
                        self.out.state(Ec::TLC_STATE_PRINT2, &[&number.to_string(), &label, &shown, "-1"]);
                    }
                }
                true
            }
            Ok(None) => {
                self.out.error(Ec::TLC_FAILED_TO_RECOVER_INIT, &[]);
                false
            }
            Err(e) => {
                self.out.error(Ec::GENERAL, &[&e.message]);
                false
            }
        }
    }

    fn print_successor(
        &self,
        ev: &mut Ev<'_>,
        pred: &State,
        pred_fp: tlars_tool::Fp,
        succ: Succ<'_>,
        printed: Option<&State>,
        number: usize,
    ) -> bool {
        let Succ::Complete(state) = succ else { unreachable!("only the last state may be incomplete") };
        let fp = match ev.fingerprint(state) {
            Ok(fp) => fp,
            Err(e) => {
                self.out.error(Ec::GENERAL, &[&e.message]);
                return false;
            }
        };
        match ev.successor_from_fp(pred, pred_fp, fp) {
            Ok(Some(step)) => {
                self.print_state(ev, &step, printed, number);
                true
            }
            Ok(None) => {
                self.out.error(Ec::TLC_FAILED_TO_RECOVER_INIT, &[]);
                false
            }
            Err(e) => {
                self.out.error(Ec::GENERAL, &[&e.message]);
                false
            }
        }
    }

    /// `StatePrinter.printInvariantViolationStateTraceState`: 2217 with the state number, the
    /// action label, the state and its fingerprint.
    fn print_state(&self, ev: &mut Ev<'_>, step: &tlars_tool::StateInfo, prev: Option<&State>, number: usize) {
        let difftrace = self.tool.globals().difftrace;
        let label = ev.state_label(step);
        let shown = ev.print_state(&step.state, if difftrace { prev } else { None });
        let fp = ev.fingerprint(&step.state).map(|fp| fp.raw() as i64).unwrap_or(-1);
        self.out.state(Ec::TLC_STATE_PRINT2, &[&number.to_string(), &label, &shown, &fp.to_string()]);
    }
}

/// `DoInitFunctor.addElement` (`ModelChecker.java:1136-1250`).
struct InitSink<'s, 'a> {
    mc: &'s ModelChecker<'a>,
    result: Ec,
    stopped: bool,
    /// `DoInitFunctor.forceChecks`: on the call-stack re-run the invariants are checked even
    /// for states the FPSet already holds.
    force_checks: bool,
    /// `DoInitFunctor.errState`: the state the functor was working on when it threw.
    failed_state: Option<State>,
    /// `DoInitFunctor.e`: an exception the functor kept instead of rethrowing, so that
    /// generation runs to the end (`ModelChecker.java:1241-1257`).
    kept: Option<EvalError>,
}

impl tlars_tool::InitSink for InitSink<'_, '_> {
    /// `DoInitFunctor.addElement`. Only the exceptions TLC names in its first `catch` leave
    /// the functor; every other `Throwable` is recorded and generation carries on, so the
    /// remaining initial states are still generated and counted.
    fn add_init(&mut self, ev: &mut Ev<'_>, s: &PartialState) -> Flow {
        match self.add_init_impl(ev, s) {
            Err(Halt::Eval(e)) if !matches!(e.kind, tlars_tool::EvalErrorKind::Runtime | tlars_tool::EvalErrorKind::Eval) => {
                self.stopped = true;
                self.kept = Some(e);
                Ok(())
            }
            other => other,
        }
    }
}

impl InitSink<'_, '_> {
    fn add_init_impl(&mut self, ev: &mut Ev<'_>, s: &PartialState) -> Flow {
        let mc = self.mc;
        let n = mc.initial_states.fetch_add(1, Relaxed);
        if n.count_ones() == 1 && n > 1 {
            mc.out.message(Ec::TLC_COMPUTING_INIT_PROGRESS, &[&n.to_string()]);
        }
        // Generation is not aborted once a violation is recorded; later states are ignored.
        if self.stopped {
            if self.result == Ec::NO_ERROR {
                self.result = Ec::TLC_INITIAL_STATE;
            }
            return Ok(());
        }
        let Some(state) = ev.complete(s) else {
            let shown = ev.print_partial_state(s);
            mc.out.error(Ec::TLC_INITIAL_STATE, &["current state is not a legal state", &shown]);
            self.result = Ec::TLC_INITIAL_STATE;
            self.stopped = true;
            return Err(Halt::Stop);
        };
        // `DoInitFunctor`'s catch records the state it was working on, whatever step failed.
        self.failed_state = Some(state.clone());
        let in_model = ev.is_in_model(&state)?;
        let mut seen = false;
        if in_model {
            let fp = ev.fingerprint(&state)?;
            seen = mc.fpset.put(fp) == crate::fpset::Seen::Old;
            if !seen {
                let ptr = mc.traces.append(0, None, fp);
                mc.traces.record_level(0, 1);
                mc.queue.enqueue(Queued { state: state.clone(), fp, ptr, level: 1 });
            }
        }
        if seen && !self.force_checks {
            return Ok(());
        }
        for id in mc.tool.invariants() {
            let name = mc.tool.invariant_name(id);
            let name = ev.tool().graph().interner().text(name).to_string();
            if !ev.check_invariant(id, &state)? {
                let shown = ev.print_state(&state, None);
                mc.out.error(Ec::TLC_INVARIANT_VIOLATED_INITIAL, &[&name, &shown]);
                if !mc.continue_after_violation {
                    mc.set_err(ErrState::InvariantInitial { id, state: state.clone() });
                    self.result = Ec::TLC_INVARIANT_VIOLATED_INITIAL;
                    self.stopped = true;
                    return Err(Halt::Stop);
                }
            }
        }
        for id in mc.tool.implied_inits() {
            if !ev.check_implied_init(id, &state)? {
                let shown = ev.print_state(&state, None);
                mc.out.error(Ec::TLC_PROPERTY_VIOLATED_INITIAL, &["", &shown]);
                self.result = Ec::TLC_PROPERTY_VIOLATED_INITIAL;
                self.stopped = true;
                return Err(Halt::Stop);
            }
        }
        self.failed_state = None;
        Ok(())
    }
}

/// What `modelCheckImpl` prints in place of a `StackOverflowError`'s null message.
const STACK_OVERFLOW_MESSAGE: &str = "This was a Java StackOverflowError. It was probably the result\nof an incorrect recursive function definition that caused TLC to enter\nan infinite loop when trying to compute the function or its application\nto an element in its putative domain.";

/// A `java.lang.Error` rather than an `Exception`, which the `catch (Exception e)` around
/// the invariant and implied-action checks does not catch (`ModelChecker.java:520, 571`).
pub(crate) fn is_java_error(e: &EvalError) -> bool {
    matches!(e.kind, tlars_tool::EvalErrorKind::StackOverflow | tlars_tool::EvalErrorKind::Assertion)
}

/// The Java class `ECGeneralMsg` names. TLC disguises its own `Assert.TLCRuntimeException`
/// as `java.lang.RuntimeException` (`MP.java:1754-1761`).
fn java_class(e: &EvalError) -> &str {
    use tlars_tool::EvalErrorKind;
    match &e.kind {
        EvalErrorKind::Jvm { class } => class,
        EvalErrorKind::Eval => "tlc2.tool.EvalException",
        _ => "java.lang.RuntimeException",
    }
}

/// The state a trace ends on after `cur`: none, a complete successor, or the incomplete one
/// `TLC_STATE_NOT_COMPLETELY_SPECIFIED_NEXT` reports.
enum Succ<'a> {
    Complete(&'a State),
    Partial(&'a PartialState),
}

/// `TLCGlobals.progressInterval`.
const PROGRESS_INTERVAL_MILLIS: u64 = 60_000;

/// `(long) ((now - old) / factor)`: a `double` division, so a run whose elapsed time is zero
/// gives `Long.MAX_VALUE` as the Java cast does.
fn rate(now: u64, old: u64, factor: f64) -> i64 {
    let value = (now as f64 - old as f64) / factor;
    if value.is_nan() {
        0
    } else if value >= i64::MAX as f64 {
        i64::MAX
    } else if value <= i64::MIN as f64 {
        i64::MIN
    } else {
        value as i64
    }
}

/// `StateVec`: the successors `doNext` collects before it checks any of them.
struct Collect<'s> {
    states: &'s mut Vec<State>,
}

impl tlars_tool::NextSink for Collect<'_> {
    fn add_next(&mut self, ev: &mut Ev<'_>, _s0: &State, _action: ActionId, s1: &PartialState) -> Flow {
        if let Some(state) = ev.complete(s1) {
            self.states.push(state);
        }
        Ok(())
    }
}

/// `doNextFailed`'s error-code map (`ModelChecker.java:632-660`).
fn eval_error_code(e: &EvalError) -> Ec {
    use tlars_tool::EvalErrorKind;
    match &e.kind {
        EvalErrorKind::StackOverflow => Ec::SYSTEM_STACK_OVERFLOW,
        EvalErrorKind::Assertion => Ec::TLC_BUG,
        EvalErrorKind::Eval => e.ec,
        _ => Ec::GENERAL,
    }
}

/// `calculateOptimisticProbability` (`AbstractChecker.java:206-208`).
fn optimistic_probability(distinct: u64, generated: u64) -> f64 {
    distinct as f64 * ((generated as f64 - distinct as f64) / 2f64.powi(64))
}

/// `AbstractChecker.ProbabilityToString` (`:258-392`): the decimal form of a probability to
/// `digits` significant digits, built from `Double.toString`.
fn probability_to_string(val: f64, digits: usize) -> String {
    if val == 0.0 {
        return "0.0".to_string();
    }
    let s: Vec<char> = tlars_java::double_to_string(val).chars().collect();
    let mut result = String::new();
    let mut next = 0;
    let mut found = 0;
    while next < s.len() && s[next] == '0' {
        next += 1;
    }
    while next < s.len() && s[next].is_ascii_digit() {
        result.push(s[next]);
        found += 1;
        next += 1;
    }
    if next == s.len() {
        return result;
    }
    if s[next] != '.' {
        return s.into_iter().collect();
    }
    if found >= digits {
        next += 1;
        while next < s.len() && s[next].is_ascii_digit() {
            next += 1;
        }
    } else {
        next += 1;
        result.push('.');
        if found == 0 {
            while next < s.len() && s[next] == '0' {
                next += 1;
                result.push('0');
            }
        }
        while next < s.len() && s[next].is_ascii_digit() && found < digits {
            result.push(s[next]);
            next += 1;
            found += 1;
        }
        // Round the last digit kept, carrying backwards over the digits of `result`.
        if next < s.len() && s[next].is_ascii_digit() && s[next].to_digit(10).expect("a digit") >= 5 {
            let mut chars: Vec<char> = result.chars().collect();
            let mut prev = chars.len() as i64 - 1;
            loop {
                if prev < 0 {
                    chars.insert(0, '1');
                    break;
                }
                let c = chars[prev as usize];
                if let Some(d) = c.to_digit(10) {
                    if c == '9' {
                        chars[prev as usize] = '0';
                    } else {
                        chars[prev as usize] = char::from_digit(d + 1, 10).expect("a digit");
                        break;
                    }
                }
                prev -= 1;
            }
            result = chars.into_iter().collect();
        }
        while next < s.len() && s[next].is_ascii_digit() {
            next += 1;
        }
    }
    if next >= s.len() {
        return result;
    }
    if s[next] == 'E' {
        result.push('E');
        result.extend(&s[next + 1..]);
        return result;
    }
    s.into_iter().collect()
}

/// `BucketStatistics` over the out-degree buckets: what 2268 prints.
struct OutDegree {
    min: i32,
    mean: i64,
    p95: i64,
    max: i32,
}

impl OutDegree {
    fn of(buckets: &[u64; 32]) -> Option<OutDegree> {
        let observations: u64 = buckets.iter().sum();
        if observations == 0 {
            return None;
        }
        let min = buckets.iter().position(|n| *n > 0).expect("an observation") as i32;
        let max = buckets.iter().rposition(|n| *n > 0).expect("an observation") as i32;
        let sum: u64 = buckets.iter().enumerate().map(|(i, n)| i as u64 * n).sum();
        let mean = java_round(sum as f64 / observations as f64);
        // `getPercentile(.95)`: the bucket at position `(int) (observations * 0.95)`.
        let pos = (observations as f64 * 0.95) as u64;
        let mut cumulative = 0;
        let mut p95 = 0.95;
        for (i, n) in buckets.iter().enumerate() {
            if *n == 0 {
                continue;
            }
            cumulative += n;
            if cumulative > pos {
                p95 = i as f64;
                break;
            }
        }
        Some(OutDegree { min, mean, p95: java_round(p95), max })
    }
}

/// `Math.round(double)`: `floor(x + 0.5)`.
fn java_round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}
