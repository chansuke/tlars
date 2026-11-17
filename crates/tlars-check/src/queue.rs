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
        todo!()
    }
    pub(crate) fn enqueue(&self, s: Queued) {
        todo!()
    }
    /// `sDequeue`: blocks; `None` once the queue is empty and every worker waits
    /// (`StateQueue.isAvail`), or after `finish_all`.
    pub(crate) fn dequeue(&self) -> Option<Queued> {
        todo!()
    }
    /// `suspendAll`: park workers for periodic work (liveness, checkpoint, FPSet growth).
    pub(crate) fn suspend_all(&self) -> bool {
        todo!()
    }
    pub(crate) fn resume_all(&self) {
        todo!()
    }
    pub(crate) fn finish_all(&self) {
        todo!()
    }
    pub(crate) fn len(&self) -> u64 {
        todo!()
    }
}
