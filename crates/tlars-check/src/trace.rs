//! Per-worker trace stores (`Worker` trace files + `ConcurrentTLCTrace`). Each worker
//! appends only to its own store (per-actor state); readers merge at reconstruction time.

use std::sync::Mutex;
use tlars_tool::{Ev, EvalResult, Fp, StateInfo};

/// TLC's `(workerId, uid)` pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TracePtr {
    worker: u16,
    index: u32,
}

/// `pred == None` marks an initial state (TLC's `ptr == 1` marker).
#[derive(Clone, Copy, Debug)]
pub(crate) struct TraceRecord {
    pred: Option<TracePtr>,
    fp: Fp,
}

pub(crate) struct TraceStores {
    per_worker: Box<[WorkerTrace]>,
}

struct WorkerTrace {
    // Uncontended: only its worker appends. Later: file-backed (`MC-N.st`) for -recover.
    records: Mutex<Vec<TraceRecord>>,
    max_level: std::sync::atomic::AtomicU32,
}

impl TraceStores {
    pub(crate) fn new(workers: usize) -> Self {
        todo!()
    }
    /// Initial states are written by the main thread into store 0.
    pub(crate) fn append(&self, worker: u16, pred: Option<TracePtr>, fp: Fp) -> TracePtr {
        todo!()
    }
    /// Follow `pred` links back to the initial record; returns fingerprints init-first.
    pub(crate) fn fingerprints_to(&self, end: TracePtr) -> Vec<Fp> {
        todo!()
    }
    /// `ConcurrentTLCTrace.getLevel`: max over workers, minimum 1.
    pub(crate) fn level(&self) -> u32 {
        todo!()
    }
}

/// `ConcurrentTLCTrace.getTrace(..)`: replay forward taking the first fingerprint match at
/// every step. RandomElement repeats its choices because each replayed step re-seeds from
/// its predecessor's fingerprint, as in the search. `Ok(None)` =
/// `TLC_FAILED_TO_RECOVER_INIT` / "failed to recover state".
pub(crate) fn reconstruct(ev: &mut Ev<'_>, fps: &[Fp]) -> EvalResult<Option<Vec<StateInfo>>> {
    ev.reset_init_random();
    let Some((&first, rest)) = fps.split_first() else { return Ok(Some(Vec::new())) };
    let Some(init) = ev.state_from_fp(first)? else { return Ok(None) };
    let mut steps = vec![init];
    let mut pred_fp = first;
    for &fp in rest {
        let pred = &steps.last().expect("non-empty").state;
        let Some(next) = ev.successor_from_fp(pred, pred_fp, fp)? else { return Ok(None) };
        steps.push(next);
        pred_fp = fp;
    }
    Ok(Some(steps))
}
