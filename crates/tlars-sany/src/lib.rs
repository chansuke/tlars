//! SANY, mirrored: module loading, lexing, parsing, semantic analysis and level checking,
//! producing one frozen [`graph::SemanticGraph`]. Also the `.cfg` parser, because TLC's
//! `ModelConfig` reuses SANY's token manager (lexical state 2) and must intern first.
//!
//! Public surface: [`load`], [`cfg::parse`], the read-only [`graph`] API, and
//! [`Atom`]/[`Interner`]. Lexer, parser, generator and level checker are private: an import
//! of them from another crate fails to compile, so there is exactly one way to obtain a graph.
//!
//! There is no second AST. TLC consumes SANY's graph directly, and so does `tlars-tool`.
#![allow(dead_code, unused_variables)]

pub mod cfg;
#[rustfmt::skip]
mod error_codes;
mod fidelity;
pub mod graph;
mod intern;
mod parser;
mod pcal;
mod semantic;
#[rustfmt::skip]
mod standard_modules;

pub use intern::{Atom, Interner, JavaClass};

/// `ErrorCode.ErrorLevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorLevel {
    Warning,
    Error,
}

/// One of SANY's standardized error codes (`tla2sany.semantic.ErrorCode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SanyErrorCode {
    pub name: &'static str,
    pub value: i32,
    pub level: ErrorLevel,
}

impl SanyErrorCode {
    /// `ErrorCode.fromStandardValue`; `None` where Java throws.
    pub fn from_standard_value(value: i32) -> Option<SanyErrorCode> {
        error_codes::SANY_ERROR_CODES.iter().copied().find(|c| c.value == value)
    }
}

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// SANY's console output, line by line, in the order TLC prints it. A line may contain
/// newlines (one `println`). TLC sends the two kinds of `ToolIO.out` line to different
/// streams when it holds SANY's log back (`DelayedPrintStream`), so they stay apart here.
pub trait SanyOut {
    /// What TLC's `SimpleSanyOutput` logs ("Parsing file ...", "Semantic processing of
    /// module X", warnings and errors).
    fn log_line(&mut self, text: &str);
    /// What PlusCal and the module resolver print to `ToolIO.out` directly.
    fn line(&mut self, text: &str);
    /// `ToolIO.err`.
    fn err_line(&mut self, text: &str);
}

pub struct LoadRequest<'a> {
    /// TLC's `mainFile`: the root module's file name as given, `.tla` optional.
    pub root_file: &'a str,
    /// `ToolIO.setUserDir`: TLC sets it to the spec's directory when `mainFile` is an
    /// absolute path; otherwise modules are found from the working directory.
    pub user_dir: Option<&'a Path>,
    /// `-DTLA-Library` paths, searched after the user directory, as given: a relative one
    /// is printed as written and opened relative to `working_dir`.
    pub library_paths: &'a [PathBuf],
    /// The JVM's `user.dir`: the user directory when none is set, and the base of relative
    /// paths.
    pub working_dir: &'a Path,
    /// Session interner. The cfg was already parsed into it (cfg strings intern first).
    pub interner: Arc<Interner>,
    /// `ParameterizedSpecObj`: modules that `-inv`, `-postCondition`, ... force into the
    /// root's EXTENDS.
    pub extra_root_extends: &'a [&'a str],
    /// `MP.getSanySuppressedCodes()` / `MP.getSanyMessagesAsErrorCodes()`.
    pub suppressed_codes: &'a [SanyErrorCode],
    pub messages_as_errors: &'a [SanyErrorCode],
}

/// SANY failed; its text has already gone to [`SanyOut`]. What `SpecProcessor.processSpec`
/// makes of it: `FrontEnd` is `TLC_PARSING_FAILED2`, the others `TLC_PARSING_FAILED` (with
/// the error strings where SANY recorded errors).
#[derive(Debug)]
pub enum SanyFailed {
    /// `SANY.parse` threw `FrontEndException`.
    FrontEnd,
    /// `specObj.parseErrors.getErrors()`.
    Parse { errors: Vec<String> },
    /// `specObj.semanticErrors.getErrors()`.
    Semantic { errors: Vec<String> },
    /// A non-zero `SanyExitCode` with no recorded errors (a warning elevated to an error).
    Exit(i32),
}

/// `SANY.parse` as `SpecProcessor.processSpec` runs it (`SanySettings.forExternalCaller`:
/// strict exit codes, semantic analysis, level checking, linting and PlusCal validation),
/// then the semantic graph. Runs on its own thread with a large stack: the translated SANY
/// recurses per nesting level, and its statics start fresh as in a new JVM.
pub fn load(req: LoadRequest<'_>, out: &mut dyn SanyOut) -> Result<graph::SemanticGraph, SanyFailed> {
    assert!(req.extra_root_extends.is_empty(), "ParameterizedSpecObj (extra root EXTENDS) is not implemented yet");
    let job = parser::LoadJob {
        root_file: req.root_file.to_string(),
        user_dir: req.user_dir.map(Path::to_path_buf),
        library_paths: req.library_paths.to_vec(),
        working_dir: req.working_dir.to_path_buf(),
        interner: req.interner.clone(),
        suppressed_codes: req.suppressed_codes.to_vec(),
        messages_as_errors: req.messages_as_errors.to_vec(),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("sany".into())
        .stack_size(1 << 28)
        .spawn(move || parser::load_on_this_thread(job, tx))
        .expect("spawning the SANY thread");
    for line in rx {
        match line {
            parser::ConsoleLine::Log(s) => out.log_line(&s),
            parser::ConsoleLine::Out(s) => out.line(&s),
            parser::ConsoleLine::Err(s) => out.err_line(&s),
        }
    }
    worker.join().unwrap_or_else(|e| std::panic::resume_unwind(e))
}
