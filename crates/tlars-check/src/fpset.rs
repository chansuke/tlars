//! Shared fingerprint set (`FPSet`). TLC's `put` returns "was already present"; here it
//! returns [`Seen`] so the inverted boolean cannot be misread. Sets store `FpKey`, whose only
//! constructor applies the 63-bit mask.

use std::sync::Mutex;
use tlars_tool::{Fp, FpKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    New,
    Old,
}

pub(crate) trait FpSet: Send + Sync {
    fn put(&self, fp: Fp) -> Seen;
    fn contains(&self, fp: Fp) -> bool;
    fn size(&self) -> u64;
    /// `checkFPs()`: minimum distance between stored fingerprints, for the "based on the
    /// actual fingerprints" probability. Called once, after the search.
    fn min_distance(&self) -> Option<u64>;
}

/// `MultiFPSet` over `2^bits` sets, each holding `FpKey`s. The default configuration has
/// `fpBits == 1` because `FPSetConfiguration.getFpBits` raises 0 to 1 for every `DiskFPSet`
/// (`FPSetConfiguration.java:52-58`), and the default implementation is `MSBDiskFPSet`. So
/// the top bit picks the set (`MultiFPSet.java:103-109`) and only the remaining 63 are the
/// key inside it (`HeapBasedDiskFPSet.java:251-253`): two fingerprints that differ in bit 63
/// alone are two states, not one.
pub(crate) struct MemFpSet {
    /// Indexed by `fp >>> (64 - bits)`.
    shards: Box<[Shard]>,
    bits: u32,
}

/// The keys of one set in sorted order, which is what `DiskFPSet.checkFPs` reads back from
/// its sorted file. One lock: with the corpus's single worker there is nothing to contend,
/// and the disk-backed implementation arrives behind this trait in M6.
struct Shard {
    keys: Mutex<std::collections::BTreeSet<u64>>,
}

impl MemFpSet {
    /// `new MultiFPSet(config)` (`MultiFPSet.java:50-66`).
    pub(crate) fn new(bits: u32) -> MemFpSet {
        let shards = (0..1usize << bits).map(|_| Shard { keys: Mutex::new(Default::default()) }).collect();
        MemFpSet { shards, bits }
    }

    /// `getFPSet(fp)`: the unsigned shift leaves the `bits` most significant bits.
    fn shard(&self, fp: Fp) -> &Shard {
        &self.shards[(fp.raw() >> (64 - self.bits)) as usize]
    }
}

impl FpSet for MemFpSet {
    fn put(&self, fp: Fp) -> Seen {
        let key = FpKey::from(fp).raw();
        if self.shard(fp).keys.lock().expect("fpset").insert(key) { Seen::New } else { Seen::Old }
    }
    fn contains(&self, fp: Fp) -> bool {
        self.shard(fp).keys.lock().expect("fpset").contains(&FpKey::from(fp).raw())
    }
    /// `MultiFPSet.size()`: the sum over the sets.
    fn size(&self) -> u64 {
        self.shards.iter().map(|s| s.keys.lock().expect("fpset").len() as u64).sum()
    }
    /// `MultiFPSet.checkFPs()`: the minimum over the sets of `DiskFPSet.checkFPs`, which is
    /// the smallest gap between adjacent fingerprints of one sorted file
    /// (`DiskFPSet.java:624-649`). Fewer than two leaves `dis` at `Long.MAX_VALUE`.
    fn min_distance(&self) -> Option<u64> {
        self.shards
            .iter()
            .filter_map(|s| {
                let keys = s.keys.lock().expect("fpset");
                keys.iter().zip(keys.iter().skip(1)).map(|(x, y)| y - x).min()
            })
            .min()
    }
}

/// `FPSetFactory.getFPSetInitialized`: `-fpbits n` sets the address bits, and a `DiskFPSet`
/// raises 0 to 1, which is the default.
pub(crate) fn from_options(fp_bits: Option<u32>, _fp_mem: Option<f64>) -> Box<dyn FpSet> {
    Box::new(MemFpSet::new(fp_bits.unwrap_or(1).max(1)))
}
