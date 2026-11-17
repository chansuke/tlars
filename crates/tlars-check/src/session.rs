//! Per-run replacement for every Java static TLC depends on.

use crate::cli::Options;
use crate::{Env, Invocation};
use std::path::PathBuf;
use std::sync::Arc;
use tlars_msg::{Ec, ExitStatus, Reporter};
use tlars_sany::Interner;

pub(crate) struct Session<'a> {
    pub(crate) reporter: Reporter<'a>,
    /// Created before the cfg is parsed: cfg strings take the first tokens.
    pub(crate) interner: Arc<Interner>,
    pub(crate) options: Options,
    pub(crate) env: Env,
    pub(crate) cwd: PathBuf,
}

impl<'a> Session<'a> {
    pub(crate) fn new(inv: Invocation<'a>, options: Options, system_properties: Vec<(String, String)>) -> Self {
        todo!()
    }

    /// `EC.ExitStatus.errorConstantToExitStatus`, except `-continue` exits 0.
    pub(crate) fn exit_status(&self, ec: Ec) -> ExitStatus {
        if self.options.continue_after_violation { ExitStatus::Success } else { ec.exit_status() }
    }
}
