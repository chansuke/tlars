//! Shared fingerprint set (`FPSet`). TLC's `put` returns "was already present"; here it
//! returns [`Seen`] so the inverted boolean cannot be misread. Sets store `FpKey`, whose only
//! constructor applies the 63-bit mask.

use std::sync::atomic::AtomicU64;
use tlars_tool::FpKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    New,
    Old,
}

pub(crate) trait FpSet: Send + Sync {
    fn put(&self, key: FpKey) -> Seen;
    fn contains(&self, key: FpKey) -> bool;
    fn size(&self) -> u64;
    /// `checkFPs()`: minimum distance between stored fingerprints, for the "based on the
    /// actual fingerprints" probability. Called once, after the search.
    fn min_distance(&self) -> Option<u64>;
}

/// Lock-free open addressing over `AtomicU64` slots (0 = empty; key 0 tracked separately).
/// Grows by stop-the-world rehash under `StateQueue::suspend_all`.
pub(crate) struct MemFpSet {
    slots: Box<[AtomicU64]>,
    zero_present: std::sync::atomic::AtomicBool,
    len: AtomicU64,
}

/// `-fpbits n`: `MultiFPSet` sharded on `fp >>> (64 - n)`.
pub(crate) struct MultiFpSet {
    shards: Box<[MemFpSet]>,
    bits: u32,
}

impl FpSet for MemFpSet {
    fn put(&self, key: FpKey) -> Seen {
        todo!()
    }
    fn contains(&self, key: FpKey) -> bool {
        todo!()
    }
    fn size(&self) -> u64 {
        todo!()
    }
    fn min_distance(&self) -> Option<u64> {
        todo!()
    }
}

impl FpSet for MultiFpSet {
    fn put(&self, key: FpKey) -> Seen {
        todo!()
    }
    fn contains(&self, key: FpKey) -> bool {
        todo!()
    }
    fn size(&self) -> u64 {
        todo!()
    }
    fn min_distance(&self) -> Option<u64> {
        todo!()
    }
}

/// `FPSetFactory`. A `DiskFpSet` (MSBDiskFPSet) arrives later behind the same trait.
pub(crate) fn from_options(fp_bits: Option<u32>, fp_mem: Option<f64>) -> Box<dyn FpSet> {
    todo!()
}
