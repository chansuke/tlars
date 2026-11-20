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

pub(crate) use bindings::{Bindings, DeclBinding, OpBinding, OpRcd};

use crate::ctx::Ctx;
use crate::error::EvalErrorKind;
use crate::eval::{EvalControl, WorkerLocals};
use crate::globals::TlcGlobals;
use crate::state::{StateRef, VarTable};
use crate::tool::Tool;
use crate::value::Value;
use tlars_msg::{Ec, Report};
use tlars_sany::Atom;
use tlars_sany::cfg::{CfgConstant, CfgValue, ModelConfig};
use tlars_sany::graph::{ExprId, ExprNode, ExprOrOpArg, Level, ModuleNode, OpDefId, OpDefKind, SemanticGraph, SymbolRef};

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
    /// `TLC.process`'s `catch (Throwable)` (`TLC.java:1424-1445`): an exception nobody meant
    /// to raise, printed by `MP.printError(EC.GENERAL, cause)` and never suppressed.
    Unexpected { class: &'static str, message: String },
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
    let model_values = model_values(&config, &graph);
    let mut binder = CfgBind {
        graph: &graph,
        defns: Default::default(),
        decls: (0..graph.decl_count()).map(|_| None).collect(),
        op_defs,
        bodies: (0..graph.expr_count()).map(|_| None).collect(),
    };
    binder.bind(&config, &model_values)?;
    let CfgBind { defns: _, decls, op_defs, bodies, .. } = binder;
    let mut tool = Tool {
        bindings: Bindings {
            op_defs: op_defs.into(),
            bodies: bodies.into(),
            decls: decls.into(),
            literals: literals.into(),
        },
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
        model_values,
        globals,
    };
    fold_constant_defns(&mut tool, out)?;
    process_config(&config, &tool)?;
    Ok(tool)
}

/// `ModelValue.mvs` after `ModelValue.setValues()` (`Spec.java:101`): one instance per cfg
/// model value, indexed by its creation order, which is the order the cfg parse met them.
fn model_values(config: &ModelConfig, graph: &SemanticGraph) -> Box<[crate::value::ModelValue]> {
    config
        .model_values
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let index = u16::try_from(i).expect("ModelValue refuses more than Short.MAX_VALUE + 1 model values");
            crate::value::ModelValue::new(name, index, &graph.interner().text(name))
        })
        .collect()
}

/// `SpecProcessor.java:551-776`: the cfg's constants and `<-` overrides applied to the
/// root module's declarations and definitions.
///
/// TLC writes four tables here, and the order of the writes is what decides which binding
/// wins: the three side tables indexed by graph id, plus `defns`, the name-keyed table that
/// is the only thing an override's right-hand side resolves against.
struct CfgBind<'g> {
    graph: &'g SemanticGraph,
    /// `this.defns`. An entry TLC `put` with a `null` value shadows an earlier binding
    /// instead of leaving it in place, so the value is an `Option`.
    defns: std::collections::BTreeMap<Atom, Option<OpBinding>>,
    decls: Vec<Option<DeclBinding>>,
    op_defs: Vec<Option<OpBinding>>,
    bodies: Vec<Option<OpBinding>>,
}

impl CfgBind<'_> {
    fn bind(&mut self, config: &ModelConfig, mvs: &[crate::value::ModelValue]) -> Result<(), SpecError> {
        let graph = self.graph;
        let root = graph.module(graph.root());
        // `:503-526`: the predefined names, before `processConstants` snapshots `defns`.
        for name in ["TRUE", "FALSE", "BOOLEAN", "STRING"] {
            let value = predefined(name).expect("a predefined name has a value");
            self.defns.insert(graph.interner().intern(name), Some(OpBinding::Value(value)));
        }
        let constants = constant_table(&config.constants, graph, mvs)?;
        // `:556-569`: every root constant needs a cfg value or an override.
        for &d in root.constant_decls.iter() {
            let name = graph.decl(d).name;
            let text = graph.interner().text(name);
            let val = constants.get(&*text);
            if val.is_none() && config.overrides.get(&text.to_string(), java_hash(&text)).is_none() {
                return Err(SpecError::Config {
                    ec: Ec::TLC_CONFIG_VALUE_NOT_ASSIGNED_TO_CONSTANT_PARAM,
                    params: vec![text.to_string()],
                });
            }
            self.decls[d.index()] = val.map(decl_binding);
            self.defns.insert(name, val.cloned());
        }
        // `:572-585`: a cfg value on a definition replaces it; without one the definition
        // itself goes into `defns`.
        for &d in root.op_defs.iter() {
            let name = graph.op_def(d).name;
            match constants.get(&*graph.interner().text(name)) {
                None => {
                    self.defns.insert(name, Some(OpBinding::Replaced(d)));
                }
                Some(val) => {
                    self.op_defs[d.index()] = Some(val.clone());
                    self.defns.insert(name, Some(val.clone()));
                }
            }
        }
        self.mod_constants(config, mvs)?;
        self.module_overrides();
        self.overrides(config)?;
        Ok(())
    }

    /// `:589-606`: `C = [Mod] v` binds the *body* of `Mod`'s definition, not the definition,
    /// and TLC does not allow it on a constant declaration.
    fn mod_constants(&mut self, config: &ModelConfig, mvs: &[crate::value::ModelValue]) -> Result<(), SpecError> {
        let graph = self.graph;
        for (_, m) in graph.modules() {
            let module = graph.interner().text(m.name).to_string();
            let Some(lines) = config.mod_constants.get(&module, java_hash(&module)) else { continue };
            let constants = constant_table(lines, graph, mvs)?;
            for &d in m.op_defs.iter() {
                let def = graph.op_def(d);
                let Some(body) = def.body else { continue };
                if let Some(val) = constants.get(&*graph.interner().text(def.name)) {
                    self.bodies[body.index()] = Some(val.clone());
                }
            }
        }
        Ok(())
    }

    /// `processModuleOverrides` (`SpecProcessor.java:2119-2190`) for the standard modules:
    /// each module named like a `tlc2.module` class has the bodies of its definitions (its
    /// extended ones included) bound to the class's methods, and the method also lands in
    /// `defns` under the definition's name. A later module rebinds a shared definition.
    /// Methods tlars does not implement yet stay unimplemented natives.
    fn module_overrides(&mut self) {
        let graph = self.graph;
        for (mid, m) in graph.modules() {
            let module = graph.interner().text(m.name);
            if !crate::modules::JAVA_MODULES.contains(&&*module) {
                continue;
            }
            for &d in m.op_defs.iter() {
                let def = graph.op_def(d);
                let Some(body) = def.body else { continue };
                let binding = match crate::modules::NativeId::of_module(&module, &graph.interner().text(def.name)) {
                    Some(id) => OpBinding::Native(id),
                    None if def.module == mid => OpBinding::Unimplemented,
                    None => continue,
                };
                self.bodies[body.index()] = Some(binding.clone());
                self.defns.insert(def.name, Some(binding));
            }
        }
    }

    /// `:608-776`: the `<-` overrides. The right-hand side is looked up in `defns` by its
    /// cfg spelling, which interns it (`Defns.get(String)`).
    fn overrides(&mut self, config: &ModelConfig) -> Result<(), SpecError> {
        let graph = self.graph;
        let root = graph.module(graph.root());
        let mut overridden: Vec<String> = Vec::new();
        // `:610-630`: constant declarations first, with no arity check here.
        for &d in root.constant_decls.iter() {
            let lhs = graph.decl(d).name;
            let Some(val) = self.rhs_of(config, lhs)? else { continue };
            self.decls[d.index()] = Some(match &val {
                OpBinding::Replaced(op) => DeclBinding::Op(*op),
                OpBinding::Table(lines) => DeclBinding::Table(lines.clone()),
                OpBinding::Value(v) => DeclBinding::Value(v.clone()),
                OpBinding::Native(id) => DeclBinding::Native(*id),
                OpBinding::Unimplemented => return Err(unimplemented("a constant overridden with an unimplemented Java method")),
            });
            self.defns.insert(lhs, Some(val));
            overridden.push(graph.interner().text(lhs).to_string());
        }
        // `:647-736`: definitions, where a definition on the right must have the same arity.
        for &d in root.op_defs.iter() {
            let lhs = graph.op_def(d).name;
            let Some(val) = self.rhs_of(config, lhs)? else { continue };
            if let OpBinding::Replaced(op) = val
                && graph.op_def(d).params.len() != graph.op_def(op).params.len()
            {
                let rhs = graph.interner().text(graph.op_def(op).name).to_string();
                return Err(SpecError::Config {
                    ec: Ec::TLC_CONFIG_WRONG_SUBSTITUTION_NUMBER_OF_ARGS,
                    params: vec![graph.interner().text(lhs).to_string(), rhs],
                });
            }
            self.op_defs[d.index()] = Some(val.clone());
            self.defns.insert(lhs, Some(val));
            overridden.push(graph.interner().text(lhs).to_string());
        }
        // `:738-747`: every left-hand side must have matched something in the root module.
        for (key, _) in config.overrides.elements() {
            if !overridden.contains(key) {
                return Err(SpecError::Config { ec: Ec::TLC_CONFIG_ID_DOES_NOT_APPEAR_IN_SPEC, params: vec![key.clone()] });
            }
        }
        self.mod_overrides(config)
    }

    /// `:750-800`: `C <- [Mod] Op` binds `Mod`'s definition body; the right-hand side still
    /// comes from the root module's `defns`.
    fn mod_overrides(&mut self, config: &ModelConfig) -> Result<(), SpecError> {
        let graph = self.graph;
        for (_, m) in graph.modules() {
            let module = graph.interner().text(m.name).to_string();
            let Some(defs) = config.mod_overrides.get(&module, java_hash(&module)) else { continue };
            let mut overridden: Vec<String> = Vec::new();
            for &d in m.op_defs.iter() {
                let def = graph.op_def(d);
                let Some(body) = def.body else { continue };
                let lhs = graph.interner().text(def.name).to_string();
                let Some(rhs) = defs.get(&lhs, java_hash(&lhs)) else { continue };
                if defs.get(rhs, java_hash(rhs)).is_some() {
                    return Err(SpecError::Config { ec: Ec::TLC_CONFIG_RHS_ID_APPEARED_AFTER_LHS_ID, params: vec![rhs.clone()] });
                }
                let Some(val) = self.defn_of(rhs) else {
                    return Err(SpecError::Config { ec: Ec::TLC_CONFIG_WRONG_SUBSTITUTION, params: vec![lhs, rhs.clone()] });
                };
                if let OpBinding::Replaced(op) = val
                    && def.params.len() != graph.op_def(op).params.len()
                {
                    let rhs = graph.interner().text(graph.op_def(op).name).to_string();
                    return Err(SpecError::Config { ec: Ec::TLC_CONFIG_WRONG_SUBSTITUTION_NUMBER_OF_ARGS, params: vec![lhs, rhs] });
                }
                self.bodies[body.index()] = Some(val);
                overridden.push(lhs);
            }
            for (key, _) in defs.elements() {
                if !overridden.contains(key) {
                    return Err(SpecError::Config { ec: Ec::TLC_CONFIG_ID_DOES_NOT_APPEAR_IN_SPEC, params: vec![key.clone()] });
                }
            }
        }
        // `:802-811`: the cfg named a module the spec does not have.
        for (module, _) in config.mod_overrides.elements() {
            if graph.modules().all(|(_, m)| &*graph.interner().text(m.name) != module) {
                return Err(SpecError::Config { ec: Ec::TLC_NO_MODULES, params: vec![module.clone()] });
            }
        }
        Ok(())
    }

    /// One `lhs <- rhs` line: the checks TLC runs on both override loops and the `defns`
    /// lookup of `rhs`. `None` when the cfg overrides nothing under this name.
    fn rhs_of(&self, config: &ModelConfig, lhs: Atom) -> Result<Option<OpBinding>, SpecError> {
        let text = self.graph.interner().text(lhs).to_string();
        let Some(rhs) = config.overrides.get(&text, java_hash(&text)) else { return Ok(None) };
        if config.overrides.get(rhs, java_hash(rhs)).is_some() {
            return Err(SpecError::Config { ec: Ec::TLC_CONFIG_RHS_ID_APPEARED_AFTER_LHS_ID, params: vec![rhs.clone()] });
        }
        match self.defn_of(rhs) {
            Some(val) => Ok(Some(val)),
            None => Err(SpecError::Config { ec: Ec::TLC_CONFIG_WRONG_SUBSTITUTION, params: vec![text, rhs.clone()] }),
        }
    }

    /// `this.defns.get(rhs)`: interns the cfg identifier, as `Defns.get(String)` does.
    fn defn_of(&self, rhs: &str) -> Option<OpBinding> {
        let name = self.graph.interner().intern(rhs);
        self.defns.get(&name).cloned().flatten()
    }
}

/// `rootConsts[i].setToolObject(toolId, val)` for a cfg value.
fn decl_binding(val: &OpBinding) -> DeclBinding {
    match val {
        OpBinding::Value(v) => DeclBinding::Value(v.clone()),
        OpBinding::Table(lines) => DeclBinding::Table(lines.clone()),
        OpBinding::Native(_) | OpBinding::Replaced(_) | OpBinding::Unimplemented => {
            unreachable!("a cfg constant table holds only values and operator records")
        }
    }
}

/// `initializeConstants` + `makeConstantTable` (`SpecProcessor.java:2355-2410`): the cfg's
/// `CONSTANT` lines as a name → tool-object table. A line without arguments holds the value
/// itself and replaces whatever the name had; lines with arguments accumulate into one
/// `OpRcdValue` whose arity every later line must match.
fn constant_table(
    lines: &[CfgConstant],
    graph: &SemanticGraph,
    mvs: &[crate::value::ModelValue],
) -> Result<std::collections::BTreeMap<String, OpBinding>, SpecError> {
    let mut table: std::collections::BTreeMap<String, OpBinding> = Default::default();
    for line in lines {
        let value = cfg_value(&line.value, graph, mvs);
        if line.args.is_empty() {
            table.insert(line.lhs.clone(), OpBinding::Value(value));
            continue;
        }
        let args: Box<[Value]> = line.args.iter().map(|a| cfg_value(a, graph, mvs)).collect();
        match table.get_mut(&line.lhs) {
            None => {
                table.insert(line.lhs.clone(), OpBinding::Table(std::sync::Arc::from(vec![(args, value)])));
            }
            Some(OpBinding::Table(rcd)) => {
                if rcd[0].0.len() != args.len() {
                    return Err(SpecError::Config { ec: Ec::TLC_CONFIG_OP_ARITY_INCONSISTENT, params: vec![line.lhs.clone()] });
                }
                let mut rows = rcd.to_vec();
                rows.push((args, value));
                *rcd = std::sync::Arc::from(rows);
            }
            // `(OpRcdValue) val` on a plain value: TLC throws a ClassCastException that
            // nothing catches, which is the top-level handler's business (exit 255).
            Some(_) => return Err(unimplemented("a cfg constant given both with and without arguments")),
        }
    }
    Ok(table)
}

/// `ModelConfig.parseValue`'s result as a [`Value`]: a `{...}` set stays as written
/// (`new SetEnumValue(elems, false)`), and a model value is the instance the cfg parse
/// created.
fn cfg_value(value: &CfgValue, graph: &SemanticGraph, mvs: &[crate::value::ModelValue]) -> Value {
    match value {
        CfgValue::Int(n) => Value::Int(*n),
        CfgValue::Str(a) => Value::Str(*a),
        CfgValue::Bool(b) => Value::Bool(*b),
        CfgValue::ModelValue(a) => {
            let mv = mvs.iter().find(|m| m.name == *a).expect("the cfg parse recorded every model value it made");
            Value::Model(*mv)
        }
        CfgValue::Set(elems) => Value::set_enum(elems.iter().map(|e| cfg_value(e, graph, mvs)).collect()),
    }
}

fn java_hash(s: &str) -> i32 {
    tlars_java::string_hash_code(s)
}

/// A failure nothing in `processSpec` catches, so `TLC.process`'s `catch (Throwable)` sees
/// it. `Assert.fail(reason)` builds a `TLCRuntimeException` without parameters, which TLC
/// reports under its parent class.
fn uncaught(e: crate::error::EvalError) -> SpecError {
    match e.kind {
        EvalErrorKind::Runtime if e.ec == Ec::GENERAL => SpecError::Unexpected { class: "java.lang.RuntimeException", message: e.message },
        EvalErrorKind::Eval => SpecError::Unexpected { class: "tlc2.tool.EvalException", message: e.message },
        EvalErrorKind::Jvm { class } => SpecError::Unexpected { class, message: e.message },
        _ => unimplemented(&format!("a cfg constant whose value fails to normalise with: {}", e.message)),
    }
}

/// A spec-processing path tlars does not mirror yet. It travels as `Assert.fail`'s generic
/// code so the caller prints it and stops, instead of guessing TLC's output.
fn unimplemented(what: &str) -> SpecError {
    SpecError::Config { ec: Ec::GENERAL, params: vec![format!("tlars does not implement {what} yet")] }
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
                        self.op_defs[id.index()] = Some(OpBinding::Value(v));
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
/// `Strings`' Java methods are added there too.
fn predefined(name: &str) -> Option<Value> {
    match name {
        "TRUE" => Some(Value::Bool(true)),
        "FALSE" => Some(Value::Bool(false)),
        "BOOLEAN" => Some(Value::set_enum_normalized(vec![Value::Bool(false), Value::Bool(true)])),
        // `Strings.STRING()`, the one method of `tlc2.module.Strings` (`:503-526`).
        "STRING" => Some(Value::User(std::sync::Arc::new(crate::value::UserValue::String))),
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
fn fold_constant_defns(tool: &mut Tool, out: &dyn Report) -> Result<(), SpecError> {
    let graph = tool.graph.clone();
    for (_, m) in graph.modules() {
        if !m.is_instantiated || m.parameter_free {
            fold_module(tool, &graph, m, out)?;
        }
    }
    Ok(())
}

fn fold_module(tool: &mut Tool, graph: &SemanticGraph, m: &ModuleNode, out: &dyn Report) -> Result<(), SpecError> {
    fold_constant_decls(tool, graph, m, out)?;
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
        // A Java override on the body makes `lookup` return it instead of the definition.
        if tool.bindings.body_chain(graph, body).is_some() {
            continue;
        }
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
            tool.bindings.op_defs[target.index()] = Some(OpBinding::Value(v));
        }
    }
    for &inner in m.inner_modules.iter() {
        fold_module(tool, graph, graph.module(inner), out)?;
    }
    Ok(())
}

/// `processConstantDefns`' first loop (`SpecProcessor.java:261-341`): a cfg value bound to a
/// constant declaration is fully initialised, and a definition bound to one must have the
/// declaration's arity and, when it takes no argument, is evaluated once up front.
fn fold_constant_decls(tool: &mut Tool, graph: &SemanticGraph, m: &ModuleNode, out: &dyn Report) -> Result<(), SpecError> {
    for &d in m.constant_decls.iter() {
        match tool.bindings.decls[d.index()].clone() {
            // `IValue.initialize()`: deep normalisation plus a fingerprint whose only other
            // effect is `convertAndCache` on a lazy class, which no cfg value can be.
            Some(DeclBinding::Value(v)) => {
                let mut locals = WorkerLocals::new(0, tool.globals.seed);
                let mut ev = tool.ev(&mut locals, out);
                v.deep_normalize(&mut ev).map_err(uncaught)?;
            }
            Some(DeclBinding::Table(lines)) => {
                let mut locals = WorkerLocals::new(0, tool.globals.seed);
                let mut ev = tool.ev(&mut locals, out);
                for (args, value) in lines.iter() {
                    for v in args.iter().chain(std::iter::once(value)) {
                        v.deep_normalize(&mut ev).map_err(uncaught)?;
                    }
                }
            }
            Some(DeclBinding::Op(op)) => fold_substituted_constant(tool, graph, d, op, out)?,
            // `MethodValue.initialize` skips the fingerprint and normalises nothing.
            Some(DeclBinding::Native(_)) | Some(DeclBinding::Var(_)) | None => {}
        }
    }
    Ok(())
}

/// The `C <- Op` half of that loop. TLC caches the value on `Op`, not on `C`, so a
/// non-deterministic definition is re-evaluated at every use of `C` (issue 648).
fn fold_substituted_constant(
    tool: &mut Tool,
    graph: &SemanticGraph,
    decl: tlars_sany::graph::DeclId,
    op: OpDefId,
    out: &dyn Report,
) -> Result<(), SpecError> {
    let names = || {
        let c = graph.interner().text(graph.decl(decl).name).to_string();
        let o = graph.interner().text(graph.op_def(op).name).to_string();
        (c, o)
    };
    if graph.op_def(op).params.len() as i32 != graph.decl(decl).arity {
        let (c, o) = names();
        return Err(SpecError::Config { ec: Ec::TLC_CONFIG_WRONG_SUBSTITUTION_NUMBER_OF_ARGS, params: vec![c, o] });
    }
    if !graph.op_def(op).params.is_empty() {
        return Ok(());
    }
    let body = graph.op_def(op).body.expect("a user definition has a body");
    let mut locals = WorkerLocals::new(0, tool.globals.seed);
    let evaluated = {
        let mut ev = tool.ev(&mut locals, out);
        ev.eval(body, &Ctx::EMPTY, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)
            .and_then(|v| v.deep_normalize(&mut ev).map(|()| v))
    };
    match evaluated {
        Ok(v) => {
            tool.bindings.op_defs[op.index()] = Some(OpBinding::Value(v));
            Ok(())
        }
        // `catch (Assert.TLCRuntimeException | EvalException e)`: only a genuinely
        // non-constant definition is reported; any other failure resurfaces at the use site
        // (issue 1109). The addendum is the exception's own message, which for
        // `Assert.fail(ec, params)` is the framed message.
        Err(e) => {
            if !matches!(e.kind, EvalErrorKind::Runtime | EvalErrorKind::Eval) {
                return Err(unimplemented(&format!("a constant substitution that fails with: {}", e.message)));
            }
            if graph.level(body) == Level::Constant {
                return Ok(());
            }
            let addendum = match e.kind {
                EvalErrorKind::Eval => String::new(),
                _ => format!(" - specifically: {}", e.message),
            };
            let (c, o) = names();
            Err(SpecError::Config { ec: Ec::TLC_CONFIG_SUBSTITUTION_NON_CONSTANT, params: vec![c, o, addendum] })
        }
    }
}

/// `processConfig` (`SpecProcessor.java:782-955`): INIT/NEXT/SPECIFICATION and the named
/// checks. An empty cfg binds nothing, but a spec with variables still needs an INIT.
fn process_config(config: &ModelConfig, tool: &Tool) -> Result<(), SpecError> {
    // Processed before the INIT/NEXT checks (`:785-890`).
    let checks_before = config.init.is_some()
        || config.next.is_some()
        || config.specification.is_some()
        || !config.invariants.is_empty()
        || !config.properties.is_empty();
    if checks_before {
        todo!("M3: INVARIANT, SPECIFICATION, INIT/NEXT and PROPERTY in processConfig");
    }
    // `:891-896`: no init predicate, but variables to initialise.
    if tool.var_count() != 0 {
        return Err(SpecError::Failure { ec: Ec::TLC_CONFIG_MISSING_INIT });
    }
    // Processed after them (`:944-952`), or later by the Tool and the checker.
    let checks_after = !config.constraints.is_empty()
        || !config.action_constraints.is_empty()
        || config.rl_reward.is_some()
        || config.periodic.is_some()
        || !config.possible.is_empty()
        || !config.postconditions.is_empty()
        || config.view.is_some()
        || config.symmetry.is_some()
        || config.alias.is_some();
    if checks_after {
        todo!("M3: constraints, _RL_REWARD, _PERIODIC, _POSSIBLE, POSTCONDITION, VIEW, SYMMETRY and ALIAS");
    }
    Ok(())
}
