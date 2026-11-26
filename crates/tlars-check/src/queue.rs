//! Shared FIFO state queue (`StateQueue` + `DiskStateQueue`). With one worker, FIFO order is
//! what makes the whole run deterministic.

use crate::trace::TracePtr;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use tlars_java::time::{Deadline, Remaining};
use tlars_tool::{Fp, State};

/// A state waiting for expansion: TLC's `TLCState` with `uid`/`workerId`/`level`.
#[derive(Clone)]
pub(crate) struct Queued {
    pub(crate) state: State,
    /// `TLCState.fingerPrint()`, cached: seeds RandomElement when this state is expanded.
    pub(crate) fp: Fp,
    pub(crate) ptr: TracePtr,
    pub(crate) level: u32,
}

pub(crate) struct StateQueue {
    inner: Mutex<QueueInner>,
    /// Workers wait here for a state.
    avail: Condvar,
    /// The main thread waits here for the search to end or the workers to park. Kept apart
    /// from `avail` so that enqueueing a state does not wake it.
    idle: Condvar,
}

struct QueueInner {
    fifo: VecDeque<Queued>,
    workers: usize,
    waiting: usize,
    suspend_requested: bool,
    finished: bool,
    // later: disk spill segments (8192-entry buffers) behind this same type
}

impl StateQueue {
    pub(crate) fn new(workers: usize) -> Self {
        let inner = QueueInner { fifo: VecDeque::new(), workers, waiting: 0, suspend_requested: false, finished: false };
        StateQueue { inner: Mutex::new(inner), avail: Condvar::new(), idle: Condvar::new() }
    }
    pub(crate) fn enqueue(&self, s: Queued) {
        let mut inner = self.inner.lock().expect("queue");
        inner.fifo.push_back(s);
        // A notify is a syscall even with nobody waiting, and a worker only waits after
        // counting itself in `waiting` under this lock.
        if inner.waiting > 0 {
            self.avail.notify_one();
        }
    }
    /// `sDequeue`: blocks; `None` once the queue is empty and every worker waits
    /// (`StateQueue.isAvail`), or after `finish_all`.
    pub(crate) fn dequeue(&self) -> Option<Queued> {
        let mut inner = self.inner.lock().expect("queue");
        loop {
            if inner.finished {
                return None;
            }
            if !inner.suspend_requested
                && let Some(s) = inner.fifo.pop_front()
            {
                return Some(s);
            }
            // Every worker idle with an empty queue means the search is over.
            inner.waiting += 1;
            if !inner.suspend_requested && inner.waiting == inner.workers {
                inner.finished = true;
                inner.waiting -= 1;
                self.avail.notify_all();
                self.idle.notify_all();
                return None;
            }
            if inner.suspend_requested {
                self.idle.notify_all();
            }
            inner = self.avail.wait(inner).expect("queue");
            inner.waiting -= 1;
        }
    }
    /// `suspendAll`: park workers for periodic work (liveness, checkpoint, FPSet growth).
    pub(crate) fn suspend_all(&self) -> bool {
        let mut inner = self.inner.lock().expect("queue");
        if inner.finished {
            return false;
        }
        inner.suspend_requested = true;
        self.avail.notify_all();
        while inner.waiting < inner.workers && !inner.finished {
            inner = self.idle.wait(inner).expect("queue");
        }
        !inner.finished
    }
    pub(crate) fn resume_all(&self) {
        let mut inner = self.inner.lock().expect("queue");
        inner.suspend_requested = false;
        self.avail.notify_all();
    }
    pub(crate) fn finish_all(&self) {
        let mut inner = self.inner.lock().expect("queue");
        inner.finished = true;
        self.avail.notify_all();
        self.idle.notify_all();
    }
    pub(crate) fn len(&self) -> u64 {
        self.inner.lock().expect("queue").fifo.len() as u64
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// `AbstractChecker.done`, which the main thread waits on (`this.wait(ms)`). `true` once
    /// the search is over; `false` when the wait timed out first.
    pub(crate) fn wait_finished(&self, deadline: &Deadline) -> bool {
        let mut inner = self.inner.lock().expect("queue");
        // A wakeup can be spurious or for parked workers, so the wait is resumed until the
        // search is over or the deadline passes.
        while !inner.finished {
            inner = match deadline.remaining() {
                Remaining::Passed => return false,
                Remaining::Unbounded => self.idle.wait(inner).expect("queue"),
                Remaining::For(left) => self.idle.wait_timeout(inner, left).expect("queue").0,
            };
        }
        true
    }
    pub(crate) fn is_finished(&self) -> bool {
        self.inner.lock().expect("queue").finished
    }
}

#[cfg(test)]
mod tests {
    use super::StateQueue;
    use crate::Clock;
    use std::time::Duration;

    /// A hermetic run's output must not depend on how long it took, so under a fixed clock
    /// the main thread's progress wait lasts until the search ends, however short it is.
    #[test]
    fn a_fixed_clock_wait_lasts_until_the_search_ends() {
        let queue = StateQueue::new(1);
        let clock = Clock::Fixed { epoch_millis: 0, utc_offset_minutes: 0 };
        std::thread::scope(|scope| {
            scope.spawn(|| queue.finish_all());
            assert!(queue.wait_finished(&clock.deadline(Duration::ZERO)));
        });
    }

    #[test]
    fn a_system_clock_wait_gives_up_at_its_deadline() {
        let queue = StateQueue::new(1);
        assert!(!queue.wait_finished(&Clock::System.deadline(Duration::ZERO)));
    }
}
