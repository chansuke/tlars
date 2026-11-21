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

/// `TLCGlobals.mainChecker`, as `TLCGet`/`TLCSet` use it: the live counters of the run plus
/// the one write, `stop()`. The checker implements it; the evaluator only calls through it.
/// Every counter is read live, so it answers what it would have answered at that point of the
/// search.
///
/// Nothing attached means `TLCGlobals.mainChecker == null`, which in TLC is the NPE the
/// module turns into `TLC_MODULE_TLCGET_UNDEFINED`.
pub trait MainChecker: Send + Sync {
    /// `getDistinctStatesGenerated`.
    fn distinct(&self) -> u64;
    /// `getStatesGenerated`.
    fn generated(&self) -> u64;
    /// `getInitialStatesGenerated`.
    fn initial(&self) -> u64;
    /// `getStateQueueSize`.
    fn queue(&self) -> u64;
    /// `getProgress`, the level `TLCTrace` reports.
    fn diameter(&self) -> u32;
    /// `(System.currentTimeMillis() - startTime) / 1000`.
    fn duration_secs(&self) -> i64;
    /// `ModelChecker.stop()`: `theStateQueue.finishAll()`, so the workers stop dequeuing and
    /// the run ends where it stands. What `TLCSet("exit", TRUE)` does.
    fn stop(&self);
}

/// `IdThread.localValues` and `IdThread.namedRegisters`: what `TLCSet` writes and `TLCGet`
/// reads back. They belong to the run, not to one [`Effects`], because TLC keeps them in the
/// worker *threads*, which outlive any single generation step: a register an ASSUME sets is
/// read back by the initial predicate, and one an action sets by the invariant.
///
/// One bank, which is exactly TLC's single-worker behaviour: the main thread writes every
/// worker's bank (`AbstractChecker.setAllValues`) and reads worker 0's, and worker 0 reads
/// and writes its own. Per-worker banks, and with them the per-worker tuples of
/// `TLCGet("all")`, arrive with `-workers` (M6).
#[derive(Default)]
pub(crate) struct Registers {
    /// `IdThread.localValues`, a `ConcurrentHashMap<Integer, IValue>`: keyed by register
    /// number, so `TLCSet(1000000000, v)` costs one entry and not an array of a billion.
    /// Ordered, because `getLocalValues()` lays the entries out by index.
    indexed: std::collections::BTreeMap<usize, Value>,
    /// The `"s:"`-prefixed named registers, in the order they were first set.
    named: Vec<(tlars_sany::Atom, Value)>,
}

impl Registers {
    /// `IdThread.setLocalValue(idx, val)`.
    pub(crate) fn set(&mut self, index: usize, v: Value) {
        self.indexed.insert(index, v);
    }
    /// `IdThread.getLocalValue(idx)`; `None` = `TLC_MODULE_TLCGET_UNDEFINED`.
    pub(crate) fn get(&self, index: usize) -> Option<Value> {
        self.indexed.get(&index).cloned()
    }
    /// `IdThread.setNamedRegister(name, val)`.
    pub(crate) fn set_named(&mut self, name: tlars_sany::Atom, v: Value) {
        match self.named.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = v,
            None => self.named.push((name, v)),
        }
    }
    /// `IdThread.getNamedRegister(name)`.
    pub(crate) fn get_named(&self, name: tlars_sany::Atom) -> Option<Value> {
        self.named.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone())
    }
    /// `IdThread.getLocalValues()`' written entries, which `getAllValues` walks in index
    /// order to fill its map.
    pub(crate) fn indexed(&self) -> impl Iterator<Item = (usize, &Value)> {
        self.indexed.iter().map(|(&i, v)| (i, v))
    }
    /// The length of the array `getLocalValues()` returns: one past the largest index
    /// written, which is the capacity `getAllValues` gives its `HashMap`.
    pub(crate) fn indexed_len(&self) -> usize {
        self.indexed.keys().next_back().map_or(0, |&i| i + 1)
    }
    /// `IdThread.getNamedRegisters()`, whose key set `getAllNamedRegisterValues` walks.
    pub(crate) fn named(&self) -> &[(tlars_sany::Atom, Value)] {
        &self.named
    }
}

/// Per-worker effect state: what TLC keeps in `IdThread` locals and statics.
pub(crate) struct Effects {
    pub(crate) random: StateRandom,
    /// The parent being expanded, set by `next_states` and by replay.
    pub(crate) current: Option<(ExpansionId, Fp)>,
}

impl Effects {
    pub(crate) fn new(run_seed: i64) -> Self {
        Effects { random: StateRandom::new(run_seed), current: None }
    }
    /// `RandomEnumerableValues.reset()`: a fresh stream seeded with the run seed.
    pub(crate) fn reset_random(&mut self, run_seed: i64) {
        self.random = StateRandom::new(run_seed);
    }
    pub(crate) fn rng(&mut self) -> &mut JavaRandom {
        self.random.for_draw(self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::Registers;
    use crate::value::Value;

    /// `localValues` is a map, so a large index costs one entry; the array
    /// `getLocalValues()` hands `getAllValues` is still as long as the largest index plus one.
    #[test]
    fn a_large_register_index_costs_one_entry() {
        let mut registers = Registers::default();
        registers.set(1_000_000_000, Value::Int(7));
        assert!(matches!(registers.get(1_000_000_000), Some(Value::Int(7))));
        assert!(registers.get(0).is_none(), "nothing was written at 0");
        assert_eq!(registers.indexed().count(), 1, "one entry, not a billion");
        assert_eq!(registers.indexed_len(), 1_000_000_001, "getLocalValues()' array length");
    }

    /// `getAllValues` walks the array by index, so the entries come out in index order
    /// whatever order they were written in.
    #[test]
    fn written_registers_come_out_in_index_order() {
        let mut registers = Registers::default();
        for index in [3, 0, 17, 4] {
            registers.set(index, Value::Int(index as i32));
        }
        assert_eq!(registers.indexed().map(|(i, _)| i).collect::<Vec<_>>(), vec![0, 3, 4, 17]);
        assert_eq!(registers.indexed_len(), 18);
    }
}
