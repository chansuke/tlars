//! The worker behind [`crate::load`]: `SpecProcessor.processSpec`'s call of `SANY.parse` and
//! its checks of the result, then [`super::freeze`].

use super::driver::{self, SanyExitCode, SanySettings};
use super::files::FilenameToStream;
use super::generated::SpecObj;
use super::java::*;
use super::support::{self, ErrorCode};
use crate::{Interner, SanyErrorCode, SanyFailed, graph};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;

/// What a load needs, owned, to move to the worker thread.
pub(crate) struct LoadJob {
    pub(crate) root_file: String,
    pub(crate) user_dir: Option<PathBuf>,
    pub(crate) library_paths: Vec<PathBuf>,
    pub(crate) working_dir: PathBuf,
    pub(crate) interner: Arc<Interner>,
    pub(crate) suppressed_codes: Vec<SanyErrorCode>,
    pub(crate) messages_as_errors: Vec<SanyErrorCode>,
}

/// One `println` by `SimpleSanyOutput`, or one to TLC's `ToolIO.out` or `ToolIO.err`.
pub(crate) enum ConsoleLine {
    Log(String),
    Out(String),
    Err(String),
}

thread_local! {
    /// Where this thread's console lines go during a load.
    static CONSOLE: RefCell<Option<Sender<ConsoleLine>>> = const { RefCell::new(None) };
}

/// Sends a console line, if a load is listening on this thread.
pub(crate) fn console(line: ConsoleLine) {
    CONSOLE.with(|c| {
        if let Some(tx) = &*c.borrow() {
            let _ = tx.send(line);
        }
    });
}

pub(crate) fn load_on_this_thread(job: LoadJob, tx: Sender<ConsoleLine>) -> Result<graph::SemanticGraph, SanyFailed> {
    CONSOLE.with(|c| *c.borrow_mut() = Some(tx));
    support::enter(&job.interner);
    let result = run(&job);
    CONSOLE.with(|c| *c.borrow_mut() = None);
    match result {
        Ok(r) => r,
        // Exceptions SANY itself does not catch escape `SANY.parse` as they would the JVM.
        Err(e) => panic!("uncaught exception in SANY: {e:?}"),
    }
}

fn error_strings(errors: JArr<JStr>) -> Vec<String> {
    errors.jget().iter().map(JToString::java_string).collect()
}

fn run(job: &LoadJob) -> JResult<Result<graph::SemanticGraph, SanyFailed>> {
    let resolver = FilenameToStream::new(job.user_dir.clone(), job.library_paths.clone(), job.working_dir.clone());
    let spec = SpecObj::new_2(JStr::of(&job.root_file), resolver)?;
    let out = support::SanyOutput::new_0();
    // `SanySettings.forExternalCaller(suppressed, elevated)`.
    let settings = SanySettings {
        strict_error_codes: true,
        semantic_analysis: true,
        level_checking: true,
        linting: true,
        validate_pcal_translation: true,
        suppressed_codes: job.suppressed_codes.iter().map(|c| ErrorCode::of(c.name)).collect(),
        messages_as_error_codes: job.messages_as_errors.iter().map(|c| ErrorCode::of(c.name)).collect(),
    };
    let exit = match driver::parse(&spec, &out, &settings) {
        Ok(code) => code,
        Err(_) => return Ok(Err(SanyFailed::FrontEnd)),
    };
    let parse_errors = spec.getParseErrors()?;
    if !parse_errors.isSuccess()? {
        return Ok(Err(SanyFailed::Parse { errors: error_strings(parse_errors.getErrors()?) }));
    }
    let semantic_errors = spec.getSemanticErrors()?;
    if !semantic_errors.isSuccess()? {
        return Ok(Err(SanyFailed::Semantic { errors: error_strings(semantic_errors.getErrors()?) }));
    }
    if exit != SanyExitCode::Ok {
        return Ok(Err(SanyFailed::Exit(exit.code())));
    }
    Ok(Ok(super::freeze::freeze(&spec.getExternalModuleTable()?, job.interner.clone())?))
}
