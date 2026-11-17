//! `TLC.process()` (`TLC.java:1337-1474`) + `Spec`/`SpecProcessor` sequencing.

use crate::session::Session;
use tlars_msg::Ec;

/// Order (each step's messages are TLC's):
///  1. `TLC2 Version` (2262), mode banner (2187 / 2188 / 2271) with the ordered parameter map.
///  2. cfg: `tlars_sany::cfg::parse` into the session interner (before SANY).
///  3. 2220, `tlars_sany::load` with SANY's own lines, 2219; failure → TLC_PARSING_FAILED (150).
///  4. `tlars_tool::build_tool` (TLC_STARTING, bindings, folding, splitting); config errors → 151.
///  5. Mode: `bfs::ModelChecker` / `dfid` / `simulate`, which run assumptions (exit 10) first.
///  6. `finally`: 2186 `Finished in ...`, trace-exploration spec (unless -tool/-continue/TE).
pub(crate) fn process(session: &Session<'_>) -> Ec {
    todo!()
}
