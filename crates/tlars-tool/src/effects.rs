//! The one effect channel (grafted from candidate 3's `EvalCx`/`World`).
//!
//! Everything an evaluation can observe or change outside its arguments goes through
//! [`Effects`]: the RandomElement RNG, `TLCSet`/`TLCGet` registers, run counters read by
//! `TLCGet("distinct")` and friends, runtime string interning, the clock, and `Print`
//! notices. A native that touches the world must call a method here, so a new native
//! cannot silently break determinism, and the future in-order commit scheduler (DESIGN.md,
//! M6) has exactly one place to journal speculative effects.

use crate::fp64::Fp;
use crate::value::Value;
use tlars_java::JavaRandom;

/// Identity of the state whose successors are being generated (TLC compares the
/// `TLCState` object with `==`). One expansion of one parent = one identity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ExpansionId(pub(crate) u64);

/// `RandomEnumerableValues.TLCStateRandom` (`RandomEnumerableValues.java:127-165`).
///
/// - Init generation (`IdThread.getCurrentState() == null`): the stream seeded with the
///   run seed, never re-seeded.
/// - Next-state generation: on the first draw for a new parent, `setSeed(fp(parent) ^ seed)`;
///   later draws for the same parent continue the stream, so
///   `x' = RandomElement(0..2) /\ y' = RandomElement(0..2)` gives two draws.
/// - Trace replay (`Tool.getState(fp, s)` calls `IdThread.setCurrentState(s)`): the same
///   rule, so replay regenerates the same choices without any snapshot/restore.
pub(crate) struct StateRandom {
    run_seed: i64,
    rng: JavaRandom,
    seeded_for: Option<ExpansionId>,
}

impl StateRandom {
    pub(crate) fn new(run_seed: i64) -> Self {
        StateRandom { run_seed, rng: JavaRandom::new(run_seed), seeded_for: None }
    }

    /// `initialize()`: `parent == None` during init generation.
    pub(crate) fn for_draw(&mut self, parent: Option<(ExpansionId, Fp)>) -> &mut JavaRandom {
        if let Some((id, fp)) = parent
            && self.seeded_for != Some(id)
        {
            self.rng.set_seed(fp.raw() as i64 ^ self.run_seed);
            self.seeded_for = Some(id);
        }
        &mut self.rng
    }
}

/// Read-only run counters owned by the checker (`TLCGet("distinct")`, `"generated"`,
/// `"queue"`, `"diameter"`, `"duration"`, `"stats"`). The checker implements this; the
/// evaluator only reads through it.
pub trait RunCounters: Sync {
    fn distinct(&self) -> u64;
    fn generated(&self) -> u64;
    fn queue(&self) -> u64;
    fn diameter(&self) -> u32;
}

/// Per-worker effect state: what TLC keeps in `IdThread` locals and statics.
pub(crate) struct Effects {
    pub(crate) random: StateRandom,
    /// `TLCSet(i, v)` integer registers (per worker in TLC: `IdThread.localValues`).
    pub(crate) registers: Vec<Option<Value>>,
    /// The parent being expanded, set by `next_states` and by replay.
    pub(crate) current: Option<(ExpansionId, Fp)>,
}

impl Effects {
    pub(crate) fn new(run_seed: i64) -> Self {
        Effects { random: StateRandom::new(run_seed), registers: Vec::new(), current: None }
    }
    pub(crate) fn rng(&mut self) -> &mut JavaRandom {
        self.random.for_draw(self.current)
    }
    pub(crate) fn tlc_set(&mut self, index: usize, v: Value) {
        todo!("grow registers; TLC_MODULE_TLCSET_* errors live in error.rs")
    }
    pub(crate) fn tlc_get(&self, index: usize) -> Option<&Value> {
        todo!("None = TLC_MODULE_TLCGET_UNDEFINED")
    }
}
