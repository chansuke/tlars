//! `Simulator` + `SimulationWorker`: random behaviours up to `-depth`, seeded `JavaRandom`
//! per worker (`-seed`, `-aril`), `-simulate num=N,file=F,stats=...`, `-generate`.

use tlars_msg::Ec;
use tlars_tool::Tool;

pub(crate) struct SimulateOptions {
    pub(crate) num: Option<u64>,
    pub(crate) depth: u32,
    pub(crate) seed: i64,
    pub(crate) aril: i64,
    pub(crate) generate: bool,
}

pub(crate) fn simulate(tool: &Tool, out: &dyn tlars_msg::Report, opts: &SimulateOptions, workers: usize) -> Ec {
    todo!()
}
