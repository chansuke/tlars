//! `StatePrinter` + error-trace printing: 2121 "The behavior up to this point is:", then
//! 2217 `N: <label>\n<state>` per step (`Ev::print_state`, `Ev::action_label`), then 2218
//! Stuttering or 2122 Back to state for lassos. ALIAS is applied inside `print_state`.

use tlars_msg::Report;
use tlars_tool::{Ev, StateInfo};

pub(crate) enum TraceTail {
    None,
    Stuttering,
    BackToState { index: usize, action: tlars_tool::ActionId },
}

pub(crate) fn print_trace(ev: &mut Ev<'_>, out: &dyn Report, steps: &[StateInfo], tail: TraceTail) {
    todo!()
}
