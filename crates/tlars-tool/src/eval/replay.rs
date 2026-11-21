//! Trace reconstruction support (`Tool.getState(fp)`, `getState(fp, pred)`): regenerate and
//! take the **first** state whose (VIEW/SYMMETRY-canonical) fingerprint matches, in action
//! split order. That choice decides the printed action label and, under VIEW/SYMMETRY, the
//! printed state. The engine never re-derives labels itself.

use super::{Ev, InitSink, NextSink, StateInfo};
use crate::actions::ActionId;
use crate::error::{EvalResult, Flow, Halt};
use crate::fp64::Fp;
use crate::state::{PartialState, State};

impl<'t> Ev<'t> {
    /// `getState(fp)` (`Tool.java:3719-3762`): the first initial state with this fingerprint.
    /// Generation runs to the end; later matches do not replace the first.
    pub fn state_from_fp(&mut self, fp: Fp) -> EvalResult<Option<StateInfo>> {
        let mut sink = InitMatch { want: fp, found: None };
        match self.init_states(&mut sink) {
            Ok(()) | Err(Halt::Stop) => {}
            Err(Halt::Eval(e)) => return Err(e),
        }
        Ok(sink.found.map(|state| StateInfo { state, action: None, fp: Some(fp) }))
    }

    /// `getState(fp, pred)` starts with `IdThread.setCurrentState(pred)` (`Tool.java:3786`),
    /// so RandomElement re-seeds with `fp(pred) ^ seed` exactly as during the search: replay
    /// needs no RNG snapshot. Only the init step uses the unseeded run stream, which
    /// `RandomEnumerableValues.reset()` rewinds (`reset_init_random`).
    pub fn successor_from_fp(&mut self, pred: &State, pred_fp: Fp, fp: Fp) -> EvalResult<Option<StateInfo>> {
        self.begin_expansion(pred_fp);
        let count = self.tool.actions.len();
        for i in 0..count {
            let id = self.tool.actions[i].id();
            let mut sink = NextMatch { want: fp, found: None };
            match self.next_states_of(pred, id, &mut sink) {
                Ok(()) | Err(Halt::Stop) => {}
                Err(Halt::Eval(e)) => return Err(e),
            }
            if let Some((state, action)) = sink.found {
                return Ok(Some(StateInfo { state, action: Some(action), fp: None }));
            }
        }
        Ok(None)
    }

    /// `getState(TLCState s1, TLCState s)` (`Tool.java:3803-3833`): the first successor that
    /// *equals* `target`, in action split order. TLC matches the last state of a
    /// counterexample this way rather than by fingerprint, so two successors that share a
    /// fingerprint cannot be confused, and a candidate whose comparison throws is skipped.
    pub fn successor_matching(&mut self, pred: &State, pred_fp: Fp, target: &PartialState) -> EvalResult<Option<(PartialState, ActionId)>> {
        self.begin_expansion(pred_fp);
        let count = self.tool.actions.len();
        for i in 0..count {
            let id = self.tool.actions[i].id();
            let mut sink = PartialMatch { want: target, found: None };
            match self.next_states_of(pred, id, &mut sink) {
                Ok(()) | Err(Halt::Stop) => {}
                Err(Halt::Eval(e)) => return Err(e),
            }
            if let Some(state) = sink.found.take() {
                return Ok(Some((state, id)));
            }
        }
        Ok(None)
    }

    /// `RandomEnumerableValues.reset()`: rewind the init-generation stream before replay.
    pub fn reset_init_random(&mut self) {
        self.locals.effects.reset_random(self.tool.globals.seed);
    }
}

/// `InitStateSelectorFunctor`: keeps the first match and lets generation finish.
struct InitMatch {
    want: Fp,
    found: Option<State>,
}

impl InitSink for InitMatch {
    fn add_init(&mut self, ev: &mut Ev<'_>, s: &PartialState) -> Flow {
        let Some(state) = ev.complete(s) else { return Ok(()) };
        if self.found.is_none() && ev.fingerprint(&state)? == self.want {
            self.found = Some(state);
        }
        Ok(())
    }
}

/// `TLCStateMut.equals` over the successors of one action. The state is kept as the
/// generator built it, so an incomplete successor matches too.
struct PartialMatch<'a> {
    want: &'a PartialState,
    found: Option<PartialState>,
}

impl NextSink for PartialMatch<'_> {
    fn add_next(&mut self, ev: &mut Ev<'_>, _s0: &State, _action: ActionId, s1: &PartialState) -> Flow {
        if self.found.is_none() && ev.states_match(s1, self.want) {
            self.found = Some(s1.copy());
        }
        Ok(())
    }
}

/// `getState(fp, s)`'s inner loop: the first successor of one action whose fingerprint
/// matches, after which the remaining successors of that action are not generated.
struct NextMatch {
    want: Fp,
    found: Option<(State, ActionId)>,
}

impl NextSink for NextMatch {
    fn add_next(&mut self, ev: &mut Ev<'_>, _s0: &State, action: ActionId, s1: &PartialState) -> Flow {
        let Some(state) = ev.complete(s1) else { return Ok(()) };
        // TLC generates every successor of the action and then picks the first match, so
        // the RandomElement draws of the later successors still happen.
        if self.found.is_none() && ev.fingerprint(&state)? == self.want {
            self.found = Some((state, action));
        }
        Ok(())
    }
}
