//! `tlc2.util.SetOfStates`: slot = `((int) fp & 0x7FFFFFFF) % length`, linear probing,
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
        SetOfStates { slots: (0..length).map(|_| None).collect(), count: 0, thresh: length / 2 }
    }

    /// `put(fp, state, tool)`: grow first when half full, then `put0`.
    pub(crate) fn put(&mut self, ev: &mut Ev<'_>, fp: Fp, s: State) {
        if self.count >= self.thresh {
            self.grow(ev);
        }
        self.put0(ev, fp, s);
    }

    /// `put0`: `true` when an equal state is already in the set.
    fn put0(&mut self, ev: &mut Ev<'_>, fp: Fp, s: State) -> bool {
        let length = self.slots.len();
        let mut loc = fp.slot_hash() as usize % length;
        loop {
            match &self.slots[loc] {
                None => {
                    self.slots[loc] = Some((fp, s));
                    self.count += 1;
                    return false;
                }
                Some((_, ent)) => {
                    if ev.states_equal_or_false(&s, ent) {
                        return true;
                    }
                }
            }
            loc = (loc + 1) % length;
        }
    }

    /// `grow(tool)`: the old slots in order, each put again under its fingerprint.
    fn grow(&mut self, ev: &mut Ev<'_>) {
        let length = (2 * self.slots.len() + 1).min(i32::MAX as usize);
        let old = std::mem::replace(&mut self.slots, (0..length).map(|_| None).collect());
        self.count = 0;
        self.thresh = length / 2;
        for (fp, s) in old.into_vec().into_iter().flatten() {
            self.put(ev, fp, s);
        }
    }

    /// `capacity()`, which tells the worker whether the set outgrew its initial size.
    pub(crate) fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// `next()` until `size()` states: the occupied slots in slot order.
    pub(crate) fn iter_slot_order(&self) -> impl Iterator<Item = &(Fp, State)> + '_ {
        self.slots.iter().flatten()
    }
}
