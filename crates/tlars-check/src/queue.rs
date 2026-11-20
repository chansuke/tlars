//! Shared FIFO state queue (`StateQueue` + `DiskStateQueue`). With one worker, FIFO order is
//! what makes the whole run deterministic.

use crate::trace::TracePtr;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
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
    avail: Condvar,
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
        StateQueue { inner: Mutex::new(inner), avail: Condvar::new() }
    }
    pub(crate) fn enqueue(&self, s: Queued) {
        let mut inner = self.inner.lock().expect("queue");
        inner.fifo.push_back(s);
        self.avail.notify_all();
    }
    /// `sDequeue`: blocks; `None` once the queue is empty and every worker waits
    /// (`StateQueue.isAvail`), or after `finish_all`.
    pub(crate) fn dequeue(&self) -> Option<Queued> {
        let mut inner = self.inner.lock().expect("queue");
        loop {
            if inner.finished {
                return None;
            }
            if !inner.suspend_requested {
                if let Some(s) = inner.fifo.pop_front() {
                    return Some(s);
                }
            }
            // Every worker idle with an empty queue means the search is over.
            inner.waiting += 1;
            if !inner.suspend_requested && inner.waiting == inner.workers {
                inner.finished = true;
                inner.waiting -= 1;
                self.avail.notify_all();
                return None;
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
            inner = self.avail.wait(inner).expect("queue");
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
    }
    pub(crate) fn len(&self) -> u64 {
        self.inner.lock().expect("queue").fifo.len() as u64
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
