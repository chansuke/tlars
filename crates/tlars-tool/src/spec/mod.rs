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

pub(crate) use bindings::{Bindings, OpBinding};

use crate::ctx::Ctx;
use crate::eval::{EvalControl, WorkerLocals};
use crate::globals::TlcGlobals;
use crate::state::{StateRef, VarTable};
use crate::tool::Tool;
use crate::value::Value;
use tlars_msg::{Ec, Report};
use tlars_sany::cfg::ModelConfig;
use tlars_sany::graph::{ExprId, ExprNode, ExprOrOpArg, Level, ModuleNode, OpDefKind, SemanticGraph, SymbolRef};

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
    /// `Assert.fail(ec, params)`: printed by `printTLCRuntimeException`, suppressible.
    Config { ec: Ec, params: Vec<String> },
    /// `Assert.fail(ec)`: no parameters, so `printTLCRuntimeException` falls back to
    /// `printError(ec, exception)`, which `-suppressMessages` does not hide.
    Failure { ec: Ec },
    /// User `<Mod>.class` override found; v1 warns and falls back (not an error), kept here
    /// for `-messagesAsErrors`.
    UnsupportedJavaOverride { module: String },
}

/// `SpecProcessor` from the root-module lookup on (`SpecProcessor.java:468`); the SANY
/// checks and TLC_STARTING before it are the caller's.
pub fn build_tool(input: ToolInput<'_>) -> Result<Tool, SpecError> {
    let ToolInput { graph, config, globals, extras, out } = input;
    if extras != CliSpecExtras::default() {
        todo!("M3: ParameterizedSpecObj extras");
    }
    let root = graph.module(graph.root());
    let vars = VarTable {
        decls: root.variable_decls.clone(),
        names: root.variable_decls.iter().map(|&d| graph.decl(d).name).collect(),
    };
    let mut constants = ConstantPass {
        graph: &graph,
        op_defs: (0..graph.op_def_count()).map(|_| None).collect(),
        literals: (0..graph.expr_count()).map(|_| None).collect(),
        processed: vec![false; graph.op_def_count()],
    };
    for (_, m) in graph.modules() {
        constants.module(m)?;
    }
    let ConstantPass { op_defs, literals, .. } = constants;
    if !config.constants.is_empty() || !config.mod_constants.is_empty() || !config.overrides.is_empty() || !config.mod_overrides.is_empty() {
        todo!("M3: cfg constants and overrides (SpecProcessor.initializeConstants, :551-720)");
    }
    if let Some(&d) = root.constant_decls.first() {
        let name = graph.interner().text(graph.decl(d).name).to_string();
        return Err(SpecError::Config { ec: Ec::TLC_CONFIG_VALUE_NOT_ASSIGNED_TO_CONSTANT_PARAM, params: vec![name] });
    }
    // TODO(M2 unit 4): processModuleOverrides binds the Java implementations of the standard
    // modules (`modules::REGISTRY`) here, before constant folding sees their TLA+ bodies.
    let mut tool = Tool {
        bindings: Bindings { op_defs: op_defs.into(), decls: (0..graph.decl_count()).map(|_| None).collect(), literals: literals.into() },
        graph: std::sync::Arc::new(graph),
        vars,
        init_actions: Box::new([]),
        actions: Box::new([]),
        invariants: Box::new([]),
        implied_inits: Box::new([]),
        implied_actions: Box::new([]),
        model_constraints: Box::new([]),
        action_constraints: Box::new([]),
        view: None,
        alias: None,
        postconditions: Box::new([]),
        symmetry: Box::new([]),
        temporal: crate::tool::Temporal { fairness: Box::new([]), properties: Box::new([]) },
        fp64: crate::fp64::Fp64::new(globals.fp_index),
        model_values: Box::new([]),
        globals,
    };
    fold_constant_defns(&mut tool, out);
    process_config(&config, &tool)?;
    Ok(tool)
}

/// `processConstants` (`SpecProcessor.java:1950-2091`): literal values for numerals and
/// strings, and the predefined `TRUE`/`FALSE`/`BOOLEAN`.
struct ConstantPass<'g> {
    graph: &'g SemanticGraph,
    op_defs: Vec<Option<OpBinding>>,
    literals: Vec<Option<Value>>,
    /// `processedDefs`: definitions already walked through an operator argument.
    processed: Vec<bool>,
}

impl ConstantPass<'_> {
    fn module(&mut self, m: &ModuleNode) -> Result<(), SpecError> {
        for &d in m.op_defs.iter() {
            if let Some(body) = self.graph.op_def(d).body {
                self.expr(body)?;
            }
        }
        for &inner in m.inner_modules.iter() {
            self.module(self.graph.module(inner))?;
        }
        for a in m.assumptions.iter() {
            self.expr(a.expr)?;
        }
        for &t in m.theorems.iter() {
            self.expr(t)?;
        }
        Ok(())
    }

    fn expr(&mut self, e: ExprId) -> Result<(), SpecError> {
        let graph = self.graph;
        match graph.expr(e) {
            ExprNode::OpAppl { op, args, bound, .. } => {
                // `defns` holds only the predefined names at this point; an operator found
                // there gets its value and its arguments are not visited.
                if let SymbolRef::OpDef(id) = *op {
                    if let Some(v) = predefined(&graph.interner().text(graph.op_def(id).name)) {
                        self.op_defs[id.index()] = Some(OpBinding::Folded(v));
                        return Ok(());
                    }
                }
                for a in args.iter() {
                    match *a {
                        ExprOrOpArg::Expr(x) => self.expr(x)?,
                        ExprOrOpArg::OpArg(sym) => self.op_arg(sym)?,
                        ExprOrOpArg::Absent => {}
                    }
                }
                for b in bound.iter().flat_map(|b| b.lists.iter()) {
                    self.expr(b.domain)?;
                }
                Ok(())
            }
            ExprNode::LetIn { defs, body, .. } => {
                for &d in defs.iter() {
                    if let Some(b) = graph.op_def(d).body {
                        self.expr(b)?;
                    }
                }
                self.expr(*body)
            }
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                for s in substs.iter() {
                    match s.expr {
                        ExprOrOpArg::Expr(x) => self.expr(x)?,
                        ExprOrOpArg::OpArg(sym) => self.op_arg(sym)?,
                        ExprOrOpArg::Absent => {}
                    }
                }
                self.expr(*body)
            }
            ExprNode::Label { body, .. } => self.expr(*body),
            ExprNode::At { .. } => Ok(()),
            ExprNode::Numeral { digits, .. } => match numeral_value(digits) {
                Some(v) => {
                    self.literals[e.index()] = Some(Value::Int(v));
                    Ok(())
                }
                None => Err(SpecError::Config { ec: Ec::TLC_INTEGER_TOO_BIG, params: vec![digits.to_string()] }),
            },
            ExprNode::Decimal { text, .. } => Err(SpecError::Config { ec: Ec::TLC_CANT_HANDLE_REAL_NUMBERS, params: vec![text.to_string()] }),
            ExprNode::String { value, .. } => {
                self.literals[e.index()] = Some(Value::Str(*value));
                Ok(())
            }
        }
    }

    /// `OpArgKind`: a user definition passed as an argument has its body walked once.
    fn op_arg(&mut self, sym: SymbolRef) -> Result<(), SpecError> {
        if let SymbolRef::OpDef(id) = sym {
            let def = self.graph.op_def(id);
            if def.kind == OpDefKind::User && !self.processed[id.index()] {
                self.processed[id.index()] = true;
                if let Some(b) = def.body {
                    self.expr(b)?;
                }
            }
        }
        Ok(())
    }
}

/// The values `processSpec` puts into `defns` before `processConstants` (`:503-509`).
/// `Strings`' Java methods are added there too; they come with the native modules.
fn predefined(name: &str) -> Option<Value> {
    match name {
        "TRUE" => Some(Value::Bool(true)),
        "FALSE" => Some(Value::Bool(false)),
        "BOOLEAN" => Some(Value::set_enum(vec![Value::Bool(false), Value::Bool(true)])),
        _ => None,
    }
}

/// `NumeralNode`'s `Integer.parseInt(digits, radix)`; `None` where SANY kept a `BigInteger`.
fn numeral_value(image: &str) -> Option<i32> {
    let lower = image.to_ascii_lowercase();
    let (radix, digits) = match lower.strip_prefix('\\') {
        Some(rest) => match rest.as_bytes().first() {
            Some(b'b') => (2, &rest[1..]),
            Some(b'o') => (8, &rest[1..]),
            Some(b'h') => (16, &rest[1..]),
            _ => unreachable!("SANY rejects other numeral prefixes"),
        },
        None => (10, lower.as_str()),
    };
    i32::from_str_radix(digits, radix).ok()
}

/// `processConstantDefns` (`SpecProcessor.java:226-357`): 0-arity constant-level definitions
/// are evaluated once and replaced by their value. Every failure is swallowed (`catch
/// (Throwable swallow)`), so the error resurfaces where the definition is used.
fn fold_constant_defns(tool: &mut Tool, out: &dyn Report) {
    let graph = tool.graph.clone();
    for (_, m) in graph.modules() {
        if !m.is_instantiated || m.parameter_free {
            fold_module(tool, &graph, m, out);
        }
    }
}

fn fold_module(tool: &mut Tool, graph: &SemanticGraph, m: &ModuleNode, out: &dyn Report) {
    // Constant declarations carry tool objects only once cfg constants are bound (M3).
    for &d in m.op_defs.iter() {
        let def = graph.op_def(d);
        let home = graph.module(def.module);
        let evaluate = !home.is_instantiated || home.parameter_free;
        if !evaluate || !def.params.is_empty() || def.kind != OpDefKind::User {
            continue;
        }
        // `lookup(opDef)`: a definition already bound to a value is not an OpDefNode.
        let target = match tool.bindings.op_def(d) {
            None => d,
            Some(OpBinding::Replaced(r)) => *r,
            Some(_) => continue,
        };
        let Some(body) = graph.op_def(target).body else { continue };
        if graph.level(body) != Level::Constant {
            continue;
        }
        let mut locals = WorkerLocals::new(0, tool.globals.seed);
        let folded = {
            let mut ev = tool.ev(&mut locals, out);
            ev.eval(body, &Ctx::EMPTY, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)
                .and_then(|v| v.deep_normalize(&mut ev).map(|()| v))
        };
        if let Ok(v) = folded {
            tool.bindings.op_defs[target.index()] = Some(OpBinding::Folded(v));
        }
    }
    for &inner in m.inner_modules.iter() {
        fold_module(tool, graph, graph.module(inner), out);
    }
}

/// `processConfig` (`SpecProcessor.java:735-955`): INIT/NEXT/SPECIFICATION and the named
/// checks. An empty cfg binds nothing, but a spec with variables still needs an INIT.
fn process_config(config: &ModelConfig, tool: &Tool) -> Result<(), SpecError> {
    let empty = config.init.is_none()
        && config.next.is_none()
        && config.specification.is_none()
        && config.invariants.is_empty()
        && config.properties.is_empty()
        && config.constraints.is_empty()
        && config.action_constraints.is_empty()
        && config.postconditions.is_empty()
        && config.possible.is_empty()
        && config.view.is_none()
        && config.symmetry.is_none()
        && config.alias.is_none()
        && config.periodic.is_none()
        && config.rl_reward.is_none();
    if !empty {
        todo!("M3: processConfig for a non-empty cfg");
    }
    // `:891-896`: no init predicate, but variables to initialise.
    if tool.var_count() != 0 {
        return Err(SpecError::Failure { ec: Ec::TLC_CONFIG_MISSING_INIT });
    }
    Ok(())
}
