//! `enabled*` (`Tool.java:2953-3600`): the next-state generator run over a `FunState`
//! (TLCStateFun) with a branched `Ctx`, answering whether *some* successor exists.

use super::{EvalControl, Ev};
use crate::ctx::Ctx;
use crate::error::EvalResult;
use crate::state::{FunState, State};
use tlars_sany::graph::ExprId;

impl<'t> Ev<'t> {
    pub(crate) fn enabled(&mut self, pred: ExprId, c: &Ctx, s0: &State, s1: &FunState, control: EvalControl) -> EvalResult<bool> {
        todo!()
    }
}
