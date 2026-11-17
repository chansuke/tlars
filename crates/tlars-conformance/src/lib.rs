//! Runs tla-ts's MIT conformance corpus (`packages/conformance/test`, 61,542 TLC cases)
//! against tlars, in-process, in parallel, hermetically.
//!
//! Ports of `test-file.ts` (YAML header + embedded `(*--- recorded ... ---*)` block, with
//! `correct:` overrides and `stack:` metadata) and `transcript.ts` (drop volatile codes
//! 2185/2186/2187/2188/2189/2200/2219/2220/2262/2271/2401, plus 2190/2192/2193/2194/2199/
//! 2267/2268 in expression contexts; `<date>`/`<time>`/`<id>` normalisation; strip paths,
//! "Parsing file" lines and Java frames). Verdict order: exit code, then message-code
//! sequence, then exact text.
#![allow(dead_code, unused_variables)]

use std::path::{Path, PathBuf};
use tlars_check::{Env, ExitStatus, Invocation};

/// `hosts.ts` DETERMINISTIC_ARGUMENTS.
pub const DETERMINISTIC_ARGS: &[&str] =
    &["-workers", "1", "-fp", "0", "-seed", "4", "-lncheck", "final", "-noGenerateSpecTE"];

/// Expression tests run in five contexts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Assume,
    Init,
    Next,
    Invariant,
    Property,
}

pub struct Metadata {
    pub description: String,
    pub outcome: String,
    pub config: Option<String>,
    pub args: Vec<String>,
    pub flags: Vec<String>,
    pub contexts: Vec<Context>,
    /// `-Xss` for this case; maps to `Env::worker_stack_bytes`.
    pub stack: Option<String>,
    /// TLC-bug overrides: tlars must match this instead of the recording.
    pub correct: Vec<(String, Recording)>,
}

pub struct Recording {
    pub exit: i32,
    pub transcript: Option<String>,
}

pub struct Case {
    pub id: String,
    pub suite: String,
    pub path: PathBuf,
    pub metadata: Metadata,
    pub recording: Option<Recording>,
}

pub enum Verdict {
    Pass,
    ExitMismatch { expected: i32, actual: i32 },
    CodeSequenceMismatch { expected: Vec<u16>, actual: Vec<u16> },
    TextMismatch { first_difference: usize },
    Unsupported { reason: &'static str },
}

pub struct Corpus {
    root: PathBuf,
}

impl Corpus {
    pub fn open(root: &Path) -> std::io::Result<Self> {
        todo!()
    }
    /// Cases in a stable order (suite, then path).
    pub fn cases(&self) -> std::io::Result<Vec<Case>> {
        todo!("std-only YAML-subset reader for the test header")
    }
}

impl Case {
    /// Materialise the case (spec, cfg, includes) into `dir` and build its invocation.
    pub fn invocation<'a>(&self, dir: &Path, stdout: &'a mut Vec<u8>, context: Option<Context>) -> Invocation<'a> {
        Invocation {
            args: self.tlc_args(context),
            system_properties: vec![("tlc2.output.MP.noTimestamps".into(), "true".into())],
            cwd: dir.to_path_buf(),
            stdout: Box::new(stdout),
            stderr: Box::new(std::io::sink()),
            env: Env::hermetic(),
        }
    }
    fn tlc_args(&self, context: Option<Context>) -> Vec<String> {
        todo!()
    }
}

pub mod transcript {
    /// `normalizeTranscript`: framed `-tool` output → `code:severity body` lines.
    pub fn normalize(raw: &str, expression_context: bool) -> String {
        todo!()
    }
}

/// Run one case (every context for expression tests) and compare.
pub fn run_case(case: &Case, scratch: &Path) -> Vec<Verdict> {
    // TODO: for each context: let mut out = Vec::new();
    //   let status: ExitStatus = tlars_check::run(case.invocation(scratch, &mut out, ctx));
    //   compare(expected (correct: override first, else recording), status, &out)
    todo!()
}

pub fn compare(expected: &Recording, status: ExitStatus, stdout: &[u8], expression_context: bool) -> Verdict {
    todo!()
}
