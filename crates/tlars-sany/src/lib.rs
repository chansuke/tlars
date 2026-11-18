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
mod error_codes;
pub mod graph;
mod fidelity;
mod intern;
mod modules;
mod parser;
mod pcal;
mod semantic;

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

/// Where SANY finds `EXTENDS`/`INSTANCE` modules. Search order mirrors
/// `SimpleFilenameToStream`: spec dir (or cwd), `-DTLA-Library` paths, embedded
/// `StandardModules/*.tla`. User files shadow standard modules.
pub trait ModuleResolver {
    fn resolve(&self, module_name: &str) -> Option<ModuleSource>;
}

pub struct ModuleSource {
    pub text: Arc<str>,
    /// Absolute path printed in `Parsing file <abs> [(<uri>)]`.
    pub path: PathBuf,
    pub library_uri: Option<String>,
    /// Found via the library locator: such modules are not linted.
    pub is_standard: bool,
}

/// The default resolver; the embedded standard modules are the pinned
/// `tla2sany/StandardModules/*.tla` sources (see `fidelity`).
pub struct DefaultResolver {
    pub spec_dir: PathBuf,
    pub library_paths: Vec<PathBuf>,
}

impl ModuleResolver for DefaultResolver {
    fn resolve(&self, module_name: &str) -> Option<ModuleSource> {
        todo!()
    }
}

/// SANY's console lines ("Parsing file ...", "Semantic processing of module X",
/// "Linting of module X", warnings and errors), in order. The caller decides framing and
/// delayed printing in `-tool` mode.
pub trait SanyOut {
    fn line(&mut self, text: &str);
}

pub struct LoadRequest<'a> {
    pub root_file: &'a Path,
    pub resolver: &'a dyn ModuleResolver,
    /// Session interner. The cfg was already parsed into it (cfg strings intern first).
    pub interner: Arc<Interner>,
    /// `ParameterizedSpecObj`: modules that `-inv`, `-postCondition`, ... force into the
    /// root's EXTENDS.
    pub extra_root_extends: &'a [&'a str],
}

/// SANY failed. Its error text has already been written to [`SanyOut`]; TLC then prints
/// `TLC_PARSING_FAILED` and exits 150.
#[derive(Debug)]
pub enum SanyFailed {
    Parse,
    Semantic,
    ModuleNotFound,
    Circular,
}

/// `SANY.parse` (`SANY.java:182`): `frontEndParse` (SpecObj.loadSpec, dependency order),
/// `frontEndSemanticAnalysis` (Generator + levelCheck per module), `frontEndLinting`.
pub fn load(req: LoadRequest<'_>, out: &mut dyn SanyOut) -> Result<graph::SemanticGraph, SanyFailed> {
    // TODO(pseudocode):
    //   interner.ensure(JavaClass::ContextReInit)           // Context.reInit(): built-ins
    //   units = modules::load_spec(root, resolver, interner, out)?   // prints "Parsing file"
    //   for m in units.in_dependency_order():
    //       out.line("Semantic processing of module " + m)
    //       semantic::generate(&mut builder, m.tree, &mut errors)
    //       if errors.none() { semantic::level_check(&mut builder, m) }
    //   print warnings / errors exactly as SANY.java:403-545
    //   for m in non-standard modules: out.line("Linting of module " + m); linter
    //   Ok(builder.freeze())
    todo!()
}
