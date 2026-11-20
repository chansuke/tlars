//! `tlc2.tool.Worker`. The bodies here are real code, not pseudocode, because their line
//! order *is* TLC's observable behaviour (`Worker.java:92-140, 418-470`).

use super::{is_java_error, ErrState, ModelChecker};
use crate::fpset::{FpSet, Seen};
use crate::liveness::SetOfStates;
use crate::queue::Queued;
use std::sync::atomic::Ordering::Relaxed;
use tlars_tool::{ActionId, Ev, Flow, Halt, NextSink, PartialState, State, WorkerLocals};

pub(super) struct Worker<'a> {
    id: u16,
    mc: &'a ModelChecker<'a>,
    locals: WorkerLocals,
    sink: SinkState,
    /// Which `stats` slot this worker reports into. The call-stack replay runs on a worker
    /// TLC never registers (`new Worker(4223, ...)`), so its counters stay out of the
    /// statistics the summary prints.
    stats: Option<usize>,
}

/// The part of the worker that `addElement` mutates.
struct SinkState {
    generated: u64,
    /// `Worker.unseenSuccessorStates`, reset after each expanded state.
    unseen: u32,
    /// Liveness successor set of the state being expanded, sized `16 * multiplier`.
    live_successors: Option<SetOfStates>,
    /// Grows whenever a successor set outgrew its initial capacity (`Worker.java:235`);
    /// per worker, so liveness slot order depends on which worker expanded a state.
    live_multiplier: u32,
}

impl<'a> Worker<'a> {
    pub(super) fn new(id: u16, mc: &'a ModelChecker<'a>) -> Self {
        let locals = WorkerLocals::new(id, mc.seed);
        Worker { id, mc, locals, sink: SinkState { generated: 0, unseen: 0, live_successors: None, live_multiplier: 1 }, stats: Some(id as usize) }
    }

    /// A worker whose evaluation records the nested-expression stack, for the replay TLC
    /// runs with a `CallStackTool`.
    pub(super) fn recording(id: u16, mc: &'a ModelChecker<'a>) -> Self {
        let mut w = Worker::new(id, mc);
        w.locals.record_call_stack();
        w.stats = None;
        w
    }

    pub(super) fn take_call_stack(&mut self) -> Option<tlars_tool::CallStack> {
        self.locals.take_call_stack()
    }

    /// `Worker.run`.
    pub(super) fn run(&mut self) {
        let mc = self.mc;
        while let Some(cur) = mc.queue.dequeue() {
            let _ = self.expand(&cur);
        }
        mc.worker_done(self.id);
    }

    /// One iteration of `Worker.run`: expand `cur`, check for a deadlock, and record the
    /// counters. The `Flow` is what the next-state generation returned, which the call-stack
    /// replay inspects.
    pub(super) fn expand(&mut self, cur: &Queued) -> Flow {
        let mc = self.mc;
        let cur = cur.clone();
        let outcome;
        {
            if mc.live.is_some() {
                self.sink.live_successors = Some(SetOfStates::with_capacity(16 * self.sink.live_multiplier as usize));
            }
            self.locals.set_level(cur.level);
            let generated_before = self.sink.generated;
            let mut ev = mc.tool.ev(&mut self.locals, mc.out);
            let mut sink = WorkerSink { mc, worker: self.id, cur: &cur, st: &mut self.sink, seen: SharedPut(&*mc.fpset) };
            outcome = ev.next_states(&cur.state, cur.fp, &mut sink);
            if let Err(Halt::Eval(e)) = &outcome {
                mc.next_failed(&cur, None, e.clone());
            }
            // Deadlock = zero addElement calls, even if every successor was out of model.
            // `doNextFailed` does not rethrow, so the check also runs after the next-state
            // relation failed to evaluate; the error is already recorded then, which leaves
            // `Worker.doNextSetErr` with nothing to print but still runs the postconditions.
            if mc.check_deadlock && generated_before == self.sink.generated {
                let first = mc.set_err(ErrState::Deadlock { state: cur.clone() });
                let mut ev = mc.tool.ev(&mut self.locals, mc.out);
                if first {
                    mc.out.error(tlars_msg::Ec::TLC_DEADLOCK_REACHED, &[]);
                    mc.print_trace_and_stop(&mut ev, &cur, None);
                }
                mc.post_condition_trace(&mut ev, &cur, None, false);
            }
            if let (Some(live), Some(succs)) = (mc.live.as_ref(), self.sink.live_successors.take()) {
                let mut ev = mc.tool.ev(&mut self.locals, mc.out);
                if let Err(e) = live.add_next_state(&mut ev, &cur, succs) {
                    mc.next_failed(&cur, None, e);
                }
            }
            if let Some(slot) = self.stats {
                mc.stats[slot].generated.store(self.sink.generated, Relaxed);
                // `outDegree.addSample(unseenSuccessorStates)`, with 32 buckets and the last
                // one holding the overflow.
                let bucket = (self.sink.unseen as usize).min(31);
                mc.stats[slot].out_degree.lock().expect("out degree")[bucket] += 1;
            }
            self.sink.unseen = 0;
        }
        outcome
    }
}

/// How `addElement` learns whether a fingerprint is new (grafted from candidate 3).
///
/// v1 has one implementation, TLC's: insert into the shared FPSet and use the answer.
/// The M6 in-order commit scheduler adds a speculative probe whose "new" answers are
/// re-validated at commit. Both run the same `examine` below, so the per-successor policy
/// (counting, check timing, enqueue rule) has exactly one implementation.
pub(crate) trait SeenOracle {
    fn seen(&mut self, fp: tlars_tool::Fp) -> Seen;
}

/// TLC's `FPSet.put` on the shared set.
struct SharedPut<'s>(&'s dyn FpSet);

impl SeenOracle for SharedPut<'_> {
    fn seen(&mut self, fp: tlars_tool::Fp) -> Seen {
        self.0.put(fp)
    }
}

/// `INextStateFunctor` for one expanded state.
struct WorkerSink<'s, O: SeenOracle> {
    mc: &'s ModelChecker<'s>,
    worker: u16,
    cur: &'s Queued,
    st: &'s mut SinkState,
    seen: O,
}

impl<O: SeenOracle> NextSink for WorkerSink<'_, O> {
    /// `Worker.addElement`.
    fn add_next(&mut self, ev: &mut Ev<'_>, s0: &State, action: ActionId, s1: &PartialState) -> Flow {
        self.st.generated += 1; // every call: duplicates and out-of-model successors too
        let Some(succ) = ev.complete(s1) else {
            return Err(self.mc.incomplete(self.cur, action, s1));
        };
        match self.examine(ev, s0, &succ) {
            Err(Halt::Eval(e)) => {
                // `addElement`'s `catch (Exception e) { throw new WrappingRuntimeException(e,
                // succState); }` (`Worker.java:483-490`) carries the successor up to
                // `Worker.run`. A `java.lang.Error` is not caught there, so it arrives
                // without one and the counterexample ends at the predecessor.
                let carried = (!is_java_error(&e)).then_some(&succ);
                self.mc.next_failed(self.cur, carried, e.clone());
                Err(Halt::Eval(e))
            }
            other => other,
        }
    }
}

impl<O: SeenOracle> WorkerSink<'_, O> {
    fn examine(&mut self, ev: &mut Ev<'_>, s0: &State, succ: &State) -> Flow {
        let mc = self.mc;
        let in_model = ev.is_in_model(succ)? && ev.is_in_actions(s0, succ)?;
        // `unseen` defaults to true: out-of-model successors are invariant-checked every time.
        let mut unseen = true;
        let mut new_ptr = None;
        let mut new_fp = None;
        if in_model {
            let fp = ev.fingerprint(succ)?;
            unseen = self.seen.seen(fp) == Seen::New;
            if unseen {
                // Written before invariants run, so a violation's trace can include it.
                new_ptr = Some(mc.traces.append(self.worker, Some(self.cur.ptr), fp));
                new_fp = Some(fp);
                mc.traces.record_level(self.worker, self.cur.level + 1);
                self.st.unseen += 1;
            }
            if let Some(live) = self.st.live_successors.as_mut() {
                live.put(ev, fp, succ.clone()); // even if seen
            }
        }
        if unseen {
            mc.check_invariants(ev, self.cur, succ, new_ptr.is_some())?;
        }
        // `Worker.writeState` sets the successor's worker id, which is what tells
        // `doPostCondition` whether the state has a trace record of its own.
        mc.check_implied_actions(ev, self.cur, succ, new_ptr.is_some())?; // always, even for seen states
        if let Some(ptr) = new_ptr {
            // in_model && unseen && no violation: other workers may now explore it.
            mc.queue.enqueue(Queued { state: succ.clone(), fp: new_fp.expect("set with ptr"), ptr, level: self.cur.level + 1 });
        }
        Ok(())
    }
}
