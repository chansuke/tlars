//! `enabled*` (`Tool.java:2953-3600`): the next-state generator run over a `FunState`
//! (TLCStateFun) with a branched `Ctx`, answering whether *some* successor exists.

use super::next::Acts;
use super::Ev;
use crate::ctx::Ctx;
use crate::error::{EvalError, EvalResult};
use crate::state::{FunState, StateRef};
use tlars_sany::graph::ExprId;

impl<'t> Ev<'t> {
    pub(crate) fn enabled(&mut self, pred: ExprId, _acts: Acts<'_>, _c: &Ctx, _s0: StateRef<'_>, _s1: &FunState) -> EvalResult<bool> {
        let _ = pred;
        Err(EvalError::unimplemented("ENABLED (M3 unit 5)".into()))
    }
}
