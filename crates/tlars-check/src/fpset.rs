//! Shared fingerprint set (`FPSet`). TLC's `put` returns "was already present"; here it
//! returns [`Seen`] so the inverted boolean cannot be misread. Sets store `FpKey`, whose only
//! constructor applies the 63-bit mask.

use std::sync::Mutex;
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

/// The keys in sorted order, which is what `DiskFPSet.checkFPs` reads back from its sorted
/// file. One lock: with the corpus's single worker there is nothing to contend, and the
/// disk-backed, sharded implementations arrive behind this trait in M6.
pub(crate) struct MemFpSet {
    keys: Mutex<std::collections::BTreeSet<u64>>,
}

impl MemFpSet {
    pub(crate) fn new() -> MemFpSet {
        MemFpSet { keys: Mutex::new(std::collections::BTreeSet::new()) }
    }
}

impl FpSet for MemFpSet {
    fn put(&self, key: FpKey) -> Seen {
        if self.keys.lock().expect("fpset").insert(key.raw()) { Seen::New } else { Seen::Old }
    }
    fn contains(&self, key: FpKey) -> bool {
        self.keys.lock().expect("fpset").contains(&key.raw())
    }
    fn size(&self) -> u64 {
        self.keys.lock().expect("fpset").len() as u64
    }
    /// `DiskFPSet.checkFPs` (`DiskFPSet.java:624-649`): the smallest gap between adjacent
    /// fingerprints of the sorted file. Fewer than two leaves `dis` at `Long.MAX_VALUE`.
    fn min_distance(&self) -> Option<u64> {
        let keys = self.keys.lock().expect("fpset");
        keys.iter().zip(keys.iter().skip(1)).map(|(x, y)| y - x).min()
    }
}

/// `FPSetFactory`. A `DiskFpSet` (MSBDiskFPSet) and `MultiFPSet` (`-fpbits`) arrive later
/// behind the same trait; both hold the same keys, so the search is unaffected.
pub(crate) fn from_options(_fp_bits: Option<u32>, _fp_mem: Option<f64>) -> Box<dyn FpSet> {
    Box::new(MemFpSet::new())
}
