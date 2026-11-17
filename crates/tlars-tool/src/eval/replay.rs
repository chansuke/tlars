//! Trace reconstruction support (`Tool.getState(fp)`, `getState(fp, pred)`): regenerate and
//! take the **first** state whose (VIEW/SYMMETRY-canonical) fingerprint matches, in action
//! split order. That choice decides the printed action label and, under VIEW/SYMMETRY, the
//! printed state. The engine never re-derives labels itself.

use super::{Ev, StateInfo};
use crate::error::EvalResult;
use crate::fp64::Fp;
use crate::state::State;

impl<'t> Ev<'t> {
    pub fn state_from_fp(&mut self, fp: Fp) -> EvalResult<Option<StateInfo>> {
        todo!("init_states with a sink that stops at the first fingerprint match")
    }

    /// `getState(fp, pred)` starts with `IdThread.setCurrentState(pred)` (`Tool.java:3786`),
    /// so RandomElement re-seeds with `fp(pred) ^ seed` exactly as during the search: replay
    /// needs no RNG snapshot. Only the init step uses the unseeded run stream, which
    /// `RandomEnumerableValues.reset()` rewinds (`reset_init_random`).
    pub fn successor_from_fp(&mut self, pred: &State, pred_fp: Fp, fp: Fp) -> EvalResult<Option<StateInfo>> {
        self.begin_expansion(pred_fp);
        todo!("per action in order: next_states_of with a first-match sink")
    }

    /// `RandomEnumerableValues.reset()`: rewind the init-generation stream before replay.
    pub fn reset_init_random(&mut self) {
        todo!()
    }
}
