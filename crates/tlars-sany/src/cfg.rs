//! `tlc2.tool.impl.ModelConfig`, mirrored. Parsed *before* SANY so its strings and model
//! values take the lowest intern tokens (`Spec.java:87-113`).
//!
//! Lexing reuses SANY's token manager in lexical state 2, with `\n` appended so a trailing
//! `\*` comment terminates (`ModelConfig.java:187-193`). A list keyword ends at the next
//! token that is itself a keyword (the keyword table is the terminator set).

use crate::{Atom, Interner};
use std::path::Path;

/// A cfg right-hand side. `{...}` stays unnormalised (order as written); any bare token that
/// is not a number, string or TRUE/FALSE is a model value.
#[derive(Clone, Debug)]
pub enum CfgValue {
    /// `Integer.parseInt`: values beyond 32 bits are `CfgError::JavaNumberFormat`.
    Int(i32),
    Str(Atom),
    Bool(bool),
    ModelValue(Atom),
    Set(Vec<CfgValue>),
}

/// `id = v`, `id(args) = v`, `id = [Mod] v`, `a!b!c = v`.
#[derive(Clone, Debug)]
pub struct CfgConstant {
    pub lhs: Vec<Atom>,
    pub args: Vec<CfgValue>,
    pub value: CfgValue,
    pub module: Option<Atom>,
    pub line: u32,
}

/// `id <- op`, `id <- [Mod] op`.
#[derive(Clone, Debug)]
pub struct CfgOverride {
    pub lhs: Vec<Atom>,
    pub rhs: Atom,
    pub module: Option<Atom>,
    pub line: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ModelConfig {
    pub constants: Vec<CfgConstant>,
    pub overrides: Vec<CfgOverride>,
    pub constraints: Vec<Atom>,
    pub action_constraints: Vec<Atom>,
    pub invariants: Vec<Atom>,
    pub properties: Vec<Atom>,
    pub postconditions: Vec<Atom>,
    pub init: Option<Atom>,
    pub next: Option<Atom>,
    pub specification: Option<Atom>,
    pub view: Option<Atom>,
    pub symmetry: Option<Atom>,
    pub alias: Option<Atom>,
    pub periodic: Option<Atom>,
    pub rl_reward: Option<Atom>,
    pub possible: Vec<Atom>,
    pub check_deadlock: Option<bool>,
    /// `ModelValue.make` call order during parsing. Single source for model-value indices
    /// (`ModelValue.index`, used by symmetry permutations).
    pub model_values_in_creation_order: Vec<Atom>,
    /// Raw constant text, kept for the trace-exploration spec.
    pub raw_constants: Vec<String>,
}

#[derive(Debug)]
pub enum CfgError {
    /// `CFG_ERROR_READING_FILE` (TLC calls `System.exit` directly).
    ReadingFile { path: String },
    /// `CFG_LEXICAL_ERROR`.
    Lexical { line: u32, column: u32, text: String },
    /// `CFG_TWICE_KEYWORD`.
    TwiceKeyword { keyword: &'static str, line: u32 },
    /// `CFG_EXPECT_ID` / `CFG_EXPECTED_SYMBOL` / ... with TLC's parameters.
    Syntax { code: u16, params: Vec<String> },
    /// TLC leaks an uncaught `NumberFormatException` for > 32-bit integers.
    JavaNumberFormat { image: String },
}

/// Parse a cfg file. Interns into `interner` in the order TLC's parser does.
pub fn parse(text: &str, file: &Path, interner: &Interner) -> Result<ModelConfig, CfgError> {
    // TODO: the translated `TLAplusParserTokenManager` in lexical state 2 over `text + "\n"`; keyword table from
    // ModelConfig.java:59-89; `configTbl.get(image) != null` terminates lists.
    todo!()
}
