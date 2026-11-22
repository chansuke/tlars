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

impl TracePtr {
    /// `TLCState.uid`, which TLC sets to the record's file offset. tlars has one store per
    /// worker, so the pair is packed instead.
    pub(crate) fn to_uid(self) -> tlars_tool::TraceUid {
        (u64::from(self.worker) << 32) | u64::from(self.index)
    }
    fn from_uid(uid: tlars_tool::TraceUid) -> TracePtr {
        TracePtr { worker: (uid >> 32) as u16, index: uid as u32 }
    }
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
        let per_worker =
            (0..workers.max(1)).map(|_| WorkerTrace { records: Mutex::new(Vec::new()), max_level: Default::default() }).collect();
        TraceStores { per_worker }
    }
    /// Initial states are written by the main thread into store 0.
    pub(crate) fn append(&self, worker: u16, pred: Option<TracePtr>, fp: Fp) -> TracePtr {
        let store = &self.per_worker[worker as usize];
        let mut records = store.records.lock().expect("trace store");
        records.push(TraceRecord { pred, fp });
        TracePtr { worker, index: (records.len() - 1) as u32 }
    }
    /// `Worker.setMaxLevel`: the level of the deepest state this worker wrote.
    pub(crate) fn record_level(&self, worker: u16, level: u32) {
        let max = &self.per_worker[worker as usize].max_level;
        max.fetch_max(level, std::sync::atomic::Ordering::Relaxed);
    }
    /// Follow `pred` links back to the initial record; returns fingerprints init-first.
    pub(crate) fn fingerprints_to(&self, end: TracePtr) -> Vec<Fp> {
        let mut fps = Vec::new();
        let mut cur = Some(end);
        while let Some(ptr) = cur {
            let record = self.record(ptr);
            fps.push(record.fp);
            cur = record.pred;
        }
        fps.reverse();
        fps
    }
    /// `fingerprints_to` for a state's `TLCState.uid`.
    pub(crate) fn fingerprints_to_uid(&self, uid: tlars_tool::TraceUid) -> Vec<Fp> {
        self.fingerprints_to(TracePtr::from_uid(uid))
    }
    fn record(&self, ptr: TracePtr) -> TraceRecord {
        self.per_worker[ptr.worker as usize].records.lock().expect("trace store")[ptr.index as usize]
    }
    /// `ConcurrentTLCTrace.getLevel`: max over workers, minimum 1.
    pub(crate) fn level(&self) -> u32 {
        self.per_worker.iter().map(|w| w.max_level.load(std::sync::atomic::Ordering::Relaxed)).max().unwrap_or(1).max(1)
    }
}

/// `ConcurrentTLCTrace.getTrace(..)`: replay forward taking the first fingerprint match at
/// every step. `Ok(None)` = `TLC_FAILED_TO_RECOVER_INIT` / "failed to recover state".
pub(crate) fn reconstruct(ev: &mut Ev<'_>, fps: &[Fp]) -> EvalResult<Option<Vec<StateInfo>>> {
    ev.reconstruct_states(fps)
}
