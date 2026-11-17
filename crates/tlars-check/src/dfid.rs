//! `DFIDModelChecker` + `DFIDWorker`: iterative deepening over a fingerprint → (status,
//! level) table. Where TLC uses an unseeded RNG, tlars picks the first candidate (policy,
//! same as tla-ts).

use tlars_msg::Ec;
use tlars_tool::Tool;

pub(crate) fn model_check(tool: &Tool, out: &dyn tlars_msg::Report, max_depth: u32, workers: usize) -> Ec {
    todo!()
}
