//! `tlc2.tool.impl.ModelConfig`, mirrored. Parsed *before* SANY so its strings and model
//! values take the lowest intern tokens (`Spec.java:87-113`).
//!
//! Lexing is SANY's translated token manager in lexical state 2, over the file with `\n`
//! appended so a trailing `\*` comment terminates (`ModelConfig.java:187-193`). A list
//! keyword ends at the next token that is itself a keyword. Names (`INIT Init`, constant
//! left-hand sides, overrides) stay plain strings, as in TLC, which interns only string
//! values and model values.

use crate::{Atom, Interner};
use tlars_java::Hashtable;
use tlars_msg::Ec;

/// A cfg right-hand side. `{...}` stays as written (unnormalised); any other bare token that
/// is not a number, string or TRUE/FALSE is a model value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfgValue {
    Int(i32),
    Str(Atom),
    Bool(bool),
    ModelValue(Atom),
    Set(Vec<CfgValue>),
}

/// `id = v`, `id(args) = v`, `a!b!c = v` (the left-hand side as TLC joins it).
#[derive(Clone, Debug)]
pub struct CfgConstant {
    pub lhs: String,
    pub args: Vec<CfgValue>,
    pub value: CfgValue,
}

/// The parsed cfg. Hashtables are TLC's, so later passes see TLC's enumeration order.
#[derive(Default)]
pub struct ModelConfig {
    /// `CONSTANT` assignments without a module, in file order.
    pub constants: Vec<CfgConstant>,
    /// `id = [Mod] v`, by module.
    pub mod_constants: Hashtable<String, Vec<CfgConstant>>,
    /// `id <- op`.
    pub overrides: Hashtable<String, String>,
    /// `op` -> `id` for each override.
    pub overrides_reverse: Hashtable<String, String>,
    /// `id <- [Mod] op`, by module.
    pub mod_overrides: Hashtable<String, Hashtable<String, String>>,
    pub constraints: Vec<String>,
    pub action_constraints: Vec<String>,
    pub invariants: Vec<String>,
    pub properties: Vec<String>,
    pub postconditions: Vec<String>,
    pub possible: Vec<String>,
    pub init: Option<String>,
    pub next: Option<String>,
    pub specification: Option<String>,
    pub view: Option<String>,
    pub symmetry: Option<String>,
    pub alias: Option<String>,
    pub periodic: Option<String>,
    pub rl_reward: Option<String>,
    pub check_deadlock: Option<bool>,
    /// `ModelValue.make` first-creation order: the model values' indices.
    pub model_values: Vec<Atom>,
    /// Each `CONSTANT` section's tokens, kept for the trace-exploration spec.
    pub raw_constants: Vec<String>,
}

/// Why the cfg was rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum CfgError {
    /// `ConfigFileException(code, params)`; TLC reports it through the caller.
    ConfigFile { code: Ec, params: Vec<String> },
    /// `Integer.parseInt` threw on a number literal beyond 32 bits; TLC does not catch it.
    JavaNumberFormat { image: String },
}

/// Parses cfg file contents (TLC reads them as UTF-8), interning string and model values
/// into `interner` in the order TLC does.
pub fn parse(text: &[u8], interner: &std::sync::Arc<Interner>) -> Result<ModelConfig, CfgError> {
    crate::parser::parse_cfg(text, interner)
}
