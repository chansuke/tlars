//! `SpecProcessor` + the `Tool` constructor: bind the frozen SANY graph and the cfg into a
//! frozen [`Tool`]. This module is the only writer of [`Bindings`] (TLC's tool objects).
//!
//! Order of work mirrors `SpecProcessor.processSpec` (`:410` onwards): TLC_STARTING;
//! `vars[]` from `ModuleNode::variable_decls` (Hashtable order); `processConstants`
//! (numerals → Int, 2^31-1 check, decimals rejected, strings); cfg constants (root, then
//! `[Mod]`); Java-module overrides (`modules::REGISTRY`); cfg `<-` overrides with arity
//! checks (TLC_CONFIG_*, exit 151); `processConstantDefns` (fold 0-arity constant-level
//! defs only in modules with `!is_instantiated || parameter_free`, errors swallowed,
//! `C <- R` checked but not cached unless aggressiveConstantCaching); `processConfig`;
//! action splitting.

mod bindings;

pub(crate) use bindings::Bindings;

use crate::globals::TlcGlobals;
use crate::tool::Tool;
use tlars_msg::{Ec, Report};
use tlars_sany::cfg::ModelConfig;
use tlars_sany::graph::SemanticGraph;

/// Command-line additions that `ParameterizedSpecObj` folds into the spec.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliSpecExtras {
    /// `-inv EXPR` and `-invlevel N` (`RuntimeInvariantTemplate`), in argument order.
    pub invariants: Vec<RuntimeInvariant>,
    /// `-postCondition Mod!Op`, `-dumpTrace` and the trace-spec binary trace
    /// (`ParameterizedSpecObj.POST_CONDITIONS`).
    pub postconditions: Vec<PostCondition>,
    /// `-loadTrace` (`ParameterizedSpecObj.CONSTRAINTS`).
    pub constraints: Vec<TraceConstraint>,
    /// `-loadTrace`'s VIEW (`ParameterizedSpecObj.VIEW`, first one wins).
    pub view: Option<(String, String)>,
}

/// An invariant given on the command line: the modules its `INSTANCE`s name (extended into
/// the root module) and its source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInvariant {
    pub modules: Vec<String>,
    pub expr: String,
}

/// `ParameterizedSpecObj.PostCondition`: `module!operator`, optionally with a constant bound
/// to a file name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostCondition {
    pub module: String,
    pub operator: String,
    pub constant: Option<(String, String)>,
}

/// `ParameterizedSpecObj.Constraint`: `module!operator` with a constant bound to a file name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceConstraint {
    pub module: String,
    pub operator: String,
    pub constant: (String, String),
}

pub struct ToolInput<'a> {
    pub graph: SemanticGraph,
    pub config: ModelConfig,
    pub globals: TlcGlobals,
    pub extras: CliSpecExtras,
    pub out: &'a dyn Report,
}

/// A config/binding failure: printed by the caller with `ec.exit_status()` (151 family).
#[derive(Debug)]
pub enum SpecError {
    Config { ec: Ec, params: Vec<String> },
    /// User `<Mod>.class` override found; v1 warns and falls back (not an error), kept here
    /// for `-messagesAsErrors`.
    UnsupportedJavaOverride { module: String },
}

pub fn build_tool(input: ToolInput<'_>) -> Result<Tool, SpecError> {
    todo!()
}
