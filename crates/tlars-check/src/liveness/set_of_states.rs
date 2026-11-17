//! `tlc2.util.SetOfStates`, exact: slot = `((int) fp & 0x7FFFFFFF) % length`, linear probing,
//! grow to `2 * length + 1` when `count >= length / 2`, re-inserting in old slot order.
//! Iteration (slot order) is the order behaviour-graph edges are appended in.
//!
//! Membership is *state equality*, not fingerprint equality (`put0` compares with
//! `TLCState.equals`), and an equality that throws (incomparable values, e.g. typed model
//! values) is swallowed and treated as "different" (`put0`'s `catch`), which can add an
//! edge TLC also adds. The parent itself is put last, as the stutter step (`Worker.java:172`).

use tlars_tool::{Ev, Fp, State};

pub(crate) struct SetOfStates {
    slots: Box<[Option<(Fp, State)>]>,
    count: usize,
    thresh: usize,
}

impl SetOfStates {
    pub(crate) fn with_capacity(length: usize) -> Self {
        todo!()
    }
    /// `true` if newly added. Probes compare states with `Ev::states_equal_or_false`.
    pub(crate) fn put(&mut self, ev: &mut Ev<'_>, fp: Fp, s: State) -> bool {
        todo!()
    }
    /// Whether this set grew past its initial capacity (bumps the worker's multiplier).
    pub(crate) fn grew(&self) -> bool {
        todo!()
    }
    pub(crate) fn iter_slot_order(&self) -> impl Iterator<Item = &(Fp, State)> + '_ {
        self.slots.iter().flatten()
    }
}
