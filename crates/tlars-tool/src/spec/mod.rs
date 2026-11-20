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
use crate::eval::expr::Builtin;
use crate::eval::{EvalControl, WorkerLocals};
use crate::globals::TlcGlobals;
use crate::state::{StateRef, VarTable};
use crate::tool::{NamedCheck, Tool};
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
    let CfgBind { defns, decls, op_defs, bodies, .. } = binder;
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
        postcondition_error: None,
        counter_example: false,
        symmetry: Box::new([]),
        temporal: crate::tool::Temporal { fairness: Box::new([]), properties: Box::new([]) },
        fp64: crate::fp64::Fp64::new(globals.fp_index),
        model_values,
        globals,
    };
    let mut defns = defns;
    // `SpecProcessor.snapshot`, taken before `processConstantDefns` and reached through
    // `Spec.unprocessedDefns`: SYMMETRY resolves against it.
    let unprocessed = defns.clone();
    fold_constant_defns(&mut tool, &mut defns, out)?;
    process_config(&config, &mut tool, &defns, &unprocessed, out)?;
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
fn fold_constant_defns(tool: &mut Tool, defns: &mut Defns, out: &dyn Report) -> Result<(), SpecError> {
    let graph = tool.graph.clone();
    for (_, m) in graph.modules() {
        if !m.is_instantiated || m.parameter_free {
            fold_module(tool, defns, &graph, m, out)?;
        }
    }
    Ok(())
}

fn fold_module(tool: &mut Tool, defns: &mut Defns, graph: &SemanticGraph, m: &ModuleNode, out: &dyn Report) -> Result<(), SpecError> {
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
            tool.bindings.op_defs[target.index()] = Some(OpBinding::Value(v.clone()));
            // `:367-370`: the name-keyed table takes the value too, but only where it still
            // holds this very definition. That is why `INVARIANT Inv` on `Inv == TRUE`
            // reaches `processConfigInvariants` as a boolean and warns (2149).
            let name = graph.op_def(target).name;
            if matches!(defns.get(&name), Some(Some(OpBinding::Replaced(d))) if *d == target) {
                defns.insert(name, Some(OpBinding::Value(v)));
            }
        }
    }
    for &inner in m.inner_modules.iter() {
        fold_module(tool, defns, graph, graph.module(inner), out)?;
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
            Some(DeclBinding::Native(_)) | None => {}
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

/// What `defns` holds after `CfgBind`: the name-keyed table every cfg identifier resolves
/// against (`Defns`). A `None` entry is one TLC `put` with a `null` value.
type Defns = std::collections::BTreeMap<Atom, Option<OpBinding>>;

/// One predicate the cfg named: its expression, the context splitting it needs, and the
/// definition it came from (absent for a conjunct of SPECIFICATION).
struct Pred {
    expr: ExprId,
    ctx: Ctx,
    op_def: Option<OpDefId>,
}

/// `processConfig` (`SpecProcessor.java:782-955`): INIT/NEXT/SPECIFICATION and the named
/// checks. An empty cfg binds nothing, but a spec with variables still needs an INIT.
fn process_config(config: &ModelConfig, tool: &mut Tool, defns: &Defns, unprocessed: &Defns, out: &dyn Report) -> Result<(), SpecError> {
    let graph = tool.graph.clone();
    let mut pass = ConfigPass {
        init_preds: Vec::new(),
        next_pred: None,
        invariants: Vec::new(),
        temporals: Vec::new(),
    };
    pass.invariants(config, tool, defns, out)?;
    match config.specification.as_deref().filter(|s| !s.is_empty()) {
        None => pass.init_and_next(config, tool, defns)?,
        Some(spec) => {
            if config.init.as_deref().is_some_and(|s| !s.is_empty()) || config.next.as_deref().is_some_and(|s| !s.is_empty()) {
                return Err(SpecError::Failure { ec: Ec::TLC_CONFIG_NOT_BOTH_SPEC_AND_INIT });
            }
            match defn_of(defns, &graph, spec) {
                Some(OpBinding::Replaced(d)) => {
                    if !graph.op_def(d).params.is_empty() {
                        return Err(SpecError::Config { ec: Ec::TLC_CONFIG_ID_REQUIRES_NO_ARG, params: vec![spec.to_string()] });
                    }
                    let body = graph.op_def(d).body.expect("a user definition has a body");
                    pass.config_spec(body, &Ctx::EMPTY, false, tool, out)?;
                }
                None => {
                    return Err(SpecError::Config {
                        ec: Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED,
                        params: vec!["name".to_string(), spec.to_string()],
                    });
                }
                Some(other) => {
                    let shown = binding_string(tool, &other, out)?;
                    return Err(SpecError::Config {
                        ec: Ec::TLC_CONFIG_ID_HAS_VALUE,
                        params: vec!["value".to_string(), spec.to_string(), shown],
                    });
                }
            }
        }
    }
    if !config.properties.is_empty() {
        return Err(unimplemented("PROPERTY (M4)"));
    }
    // `:891-902`: the two completeness checks, in TLC's order.
    let has_checks = !pass.invariants.is_empty();
    if pass.init_preds.is_empty() && (has_checks || tool.var_count() != 0) {
        return Err(SpecError::Failure { ec: Ec::TLC_CONFIG_MISSING_INIT });
    }
    if pass.next_pred.is_none() && has_checks {
        return Err(SpecError::Failure { ec: Ec::TLC_CONFIG_MISSING_NEXT });
    }
    let model_constraints = named_checks(&config.constraints, tool, defns, out, "constraint")?;
    let action_constraints = named_checks(&config.action_constraints, tool, defns, out, "action constraint")?;
    if config.rl_reward.is_some() || config.periodic.is_some() {
        return Err(unimplemented("_RL_REWARD and _PERIODIC (M5)"));
    }
    config_possible(config, tool, defns)?;
    let alias = config_alias(config, tool, defns, out);
    let view = config_view(config, tool, defns)?;
    let symmetry = config_symmetry(config, tool, unprocessed, out)?;
    let (postconditions, postcondition_error) = config_postconditions(config, tool, defns, out);
    let counter_example = defns.contains_key(&graph.interner().intern("CounterExample"));

    // `deadlock = deadlock && tool.getModelConfig().getCheckDeadlock()` (`TLC.java:1408`).
    tool.globals.check_deadlock = tool.globals.check_deadlock && config.check_deadlock.unwrap_or(true);
    let ConfigPass { init_preds, next_pred, invariants, temporals } = pass;
    tool.invariants = invariants
        .into_iter()
        .map(|(name, p)| NamedCheck { name, expr: p.expr, ctx: p.ctx })
        .collect();
    tool.model_constraints = model_constraints.into();
    tool.action_constraints = action_constraints.into();
    tool.view = view;
    tool.alias = alias;
    tool.symmetry = symmetry;
    tool.postconditions = postconditions;
    tool.postcondition_error = postcondition_error;
    tool.counter_example = counter_example;
    tool.temporal = crate::tool::Temporal {
        fairness: temporals.into_iter().map(|p| (p.expr, p.ctx)).collect(),
        properties: Box::new([]),
    };
    // The `Tool` constructor's splitting (`Tool.java:224-240`). Only the next-state
    // relation is split; `getInitStateSpec()` hands the initial predicates over as they are,
    // so an init predicate that is not a boolean fails during generation, not here.
    let mut actions = Vec::new();
    {
        let mut locals = WorkerLocals::new(0, tool.globals.seed);
        let mut ev = tool.ev(&mut locals, out);
        if let Some(next) = &next_pred {
            crate::actions::split_next(&mut ev, next.expr, &next.ctx, next.op_def, &mut actions).map_err(uncaught)?;
        }
    }
    tool.init_actions = init_preds
        .iter()
        .enumerate()
        .map(|(i, p)| crate::actions::init_action(i, p.expr, &p.ctx, p.op_def))
        .collect();
    tool.actions = actions.into();
    Ok(())
}

/// What `processConfig` collects before the completeness checks.
struct ConfigPass {
    init_preds: Vec<Pred>,
    next_pred: Option<Pred>,
    invariants: Vec<(Atom, Pred)>,
    temporals: Vec<Pred>,
}

impl ConfigPass {
    /// `processConfigInvariants` (`SpecProcessor.java:1004-1045`).
    fn invariants(&mut self, config: &ModelConfig, tool: &Tool, defns: &Defns, out: &dyn Report) -> Result<(), SpecError> {
        let graph = tool.graph.clone();
        for name in &config.invariants {
            match defn_of(defns, &graph, name) {
                Some(OpBinding::Replaced(d)) => {
                    if !graph.op_def(d).params.is_empty() {
                        return Err(SpecError::Config {
                            ec: Ec::TLC_CONFIG_ID_REQUIRES_NO_ARG,
                            params: vec!["invariant".to_string(), name.clone()],
                        });
                    }
                    // An invariant must be a state predicate. TLC adds a note about its own
                    // level-checking bug when the body has level parameters; the graph does
                    // not carry `levelParams` yet, so the note is never added.
                    if graph.op_level(d) >= Level::Action {
                        let shown = graph.interner().text(graph.op_def(d).name).to_string();
                        return Err(SpecError::Config { ec: Ec::TLC_INVARIANT_VIOLATED_LEVEL, params: vec![shown] });
                    }
                    let body = graph.op_def(d).body.expect("a user definition has a body");
                    let atom = graph.interner().intern(name);
                    self.invariants.push((atom, Pred { expr: body, ctx: Ctx::EMPTY, op_def: Some(d) }));
                }
                None => {
                    return Err(SpecError::Config {
                        ec: Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED,
                        params: vec!["invariant".to_string(), name.clone()],
                    });
                }
                Some(OpBinding::Value(Value::Bool(true))) => {
                    out.warning(Ec::TLC_INVARIANT_CONSTANT_LEVEL, &[name, "TRUE"]).map_err(warning_as_error)?;
                }
                Some(other) => {
                    let shown = binding_string(tool, &other, out)?;
                    return Err(SpecError::Config {
                        ec: Ec::TLC_CONFIG_ID_HAS_VALUE,
                        params: vec!["invariant".to_string(), name.clone(), shown],
                    });
                }
            }
        }
        Ok(())
    }

    /// `processConfigInitAndNext` (`SpecProcessor.java:957-1002`).
    fn init_and_next(&mut self, config: &ModelConfig, tool: &Tool, defns: &Defns) -> Result<(), SpecError> {
        let graph = tool.graph.clone();
        let one = |name: &str, what: &str| -> Result<Pred, SpecError> {
            match defn_of(defns, &graph, name) {
                None => Err(SpecError::Config {
                    ec: Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED,
                    params: vec![what.to_string(), name.to_string()],
                }),
                Some(OpBinding::Replaced(d)) => {
                    if !graph.op_def(d).params.is_empty() {
                        return Err(SpecError::Config {
                            ec: Ec::TLC_CONFIG_ID_REQUIRES_NO_ARG,
                            params: vec![what.to_string(), name.to_string()],
                        });
                    }
                    let body = graph.op_def(d).body.expect("a user definition has a body");
                    Ok(Pred { expr: body, ctx: Ctx::EMPTY, op_def: Some(d) })
                }
                Some(_) => Err(SpecError::Config {
                    ec: Ec::TLC_CONFIG_ID_MUST_NOT_BE_CONSTANT,
                    params: vec![what.to_string(), name.to_string()],
                }),
            }
        };
        if let Some(name) = config.init.as_deref().filter(|s| !s.is_empty()) {
            self.init_preds.push(one(name, "initial predicate")?);
        }
        if let Some(name) = config.next.as_deref().filter(|s| !s.is_empty()) {
            self.next_pred = Some(one(name, "next state action")?);
        }
        Ok(())
    }

    /// `processConfigSpec` (`SpecProcessor.java:1047-1394`): the structural decomposition of
    /// SPECIFICATION into an initial predicate, one `[][Next]_v` and temporal formulas.
    ///
    /// A substitution is pushed onto the context here instead of wrapping the expression in
    /// a fresh `SubstInNode` (`Specs.addSubsts`); the bindings it makes are the same ones
    /// `getNextStates` would push at generation time, and they are not cachable either way.
    fn config_spec(&mut self, pred: ExprId, c: &Ctx, substituted: bool, tool: &Tool, out: &dyn Report) -> Result<(), SpecError> {
        let graph = tool.graph.clone();
        match graph.expr(pred) {
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                let body = *body;
                let mut locals = WorkerLocals::new(0, tool.globals.seed);
                let c1 = {
                    let mut ev = tool.ev(&mut locals, out);
                    ev.subst_context(substs, c)
                };
                // `subs.cons(pred1)`: from here on the formula is under a substitution.
                return self.config_spec(body, &c1, true, tool, out);
            }
            ExprNode::LetIn { body, .. } => return self.config_spec(*body, c, substituted, tool, out),
            ExprNode::OpAppl { .. } => {}
            _ => {}
        }
        if let ExprNode::OpAppl { op, args, .. } = graph.expr(pred) {
            let (op, args) = (*op, args.clone());
            let mut locals = WorkerLocals::new(0, tool.globals.seed);
            let (code, resolved) = {
                let mut ev = tool.ev(&mut locals, out);
                (ev.opcode(op), ev.lookup(op, c, StateRef::Empty, false))
            };
            if args.is_empty() {
                return self.config_spec_name(pred, op, c, substituted, tool, out, resolved);
            }
            // Parameterised instantiation: `Spec == R(42)` where `R(c) == INSTANCE ...`.
            // `subs.isEmpty()` (`SpecProcessor.java:1160`): TLC refuses to expand a
            // parameterised operator inside a substitution rather than walk the body in a
            // context that does not match it.
            if let crate::eval::expr::Resolved::UserOp(d) | crate::eval::expr::Resolved::Binding(crate::ctx::Binding::OpDef(d)) = resolved
                && graph.op_def(d).body.is_some()
                && !graph.op_def(d).recursive
                && graph.op_def(d).params.len() == args.len()
                && !substituted
            {
                let body = graph.op_def(d).body.expect("just checked");
                let mut locals = WorkerLocals::new(0, tool.globals.seed);
                let c1 = {
                    let mut ev = tool.ev(&mut locals, out);
                    ev.op_context(d, &args, c, false)
                };
                return self.config_spec(body, &c1, substituted, tool, out);
            }
            match code {
                Some(Builtin::TemporalExists | Builtin::TemporalForall) => {
                    return Err(SpecError::Failure { ec: Ec::TLC_SPECIFICATION_FEATURES_TEMPORAL_QUANTIFIER });
                }
                Some(Builtin::ConjList | Builtin::Land) => {
                    for a in args.iter() {
                        self.config_spec(a.expect_expr(), c, substituted, tool, out)?;
                    }
                    return Ok(());
                }
                Some(Builtin::Box) => return self.config_spec_box(pred, &args, c, tool, out),
                Some(Builtin::Nop) => return self.config_spec(args[0].expect_expr(), c, substituted, tool, out),
                _ => {}
            }
        }
        self.config_spec_level(pred, c, tool, out)
    }

    /// The `args.length == 0` branch: a name that stands for another formula.
    fn config_spec_name(
        &mut self,
        pred: ExprId,
        op: tlars_sany::graph::SymbolRef,
        c: &Ctx,
        substituted: bool,
        tool: &Tool,
        out: &dyn Report,
        resolved: crate::eval::expr::Resolved,
    ) -> Result<(), SpecError> {
        use crate::eval::expr::Resolved;
        let graph = tool.graph.clone();
        let name = || {
            let mut locals = WorkerLocals::new(0, tool.globals.seed);
            tool.ev(&mut locals, out).symbol_name(op)
        };
        match resolved {
            Resolved::UserOp(d) | Resolved::Binding(crate::ctx::Binding::OpDef(d)) => {
                if !graph.op_def(d).params.is_empty() {
                    return Err(SpecError::Config { ec: Ec::TLC_CONFIG_OP_NO_ARGS, params: vec![name()] });
                }
                let body = graph.op_def(d).body.expect("a user definition has a body");
                let mut locals = WorkerLocals::new(0, tool.globals.seed);
                let level = {
                    let mut ev = tool.ev(&mut locals, out);
                    ev.level_bound(body, c)
                };
                if level == 1 {
                    self.init_preds.push(Pred { expr: body, ctx: c.clone(), op_def: Some(d) });
                    return Ok(());
                }
                self.config_spec(body, c, substituted, tool, out)
            }
            Resolved::Binding(crate::ctx::Binding::Thunk(t)) => {
                let (e, tc) = (t.expr, t.ctx.clone());
                self.config_spec(e, &tc, substituted, tool, out)
            }
            Resolved::Value(Value::Bool(true)) | Resolved::Binding(crate::ctx::Binding::Value(Value::Bool(true))) => Ok(()),
            Resolved::Value(Value::Bool(false)) | Resolved::Binding(crate::ctx::Binding::Value(Value::Bool(false))) => {
                Err(SpecError::Config { ec: Ec::TLC_CONFIG_SPEC_IS_TRIVIAL, params: vec![name()] })
            }
            Resolved::Value(Value::Undef) | Resolved::Binding(crate::ctx::Binding::Value(Value::Undef)) => {
                Err(SpecError::Config { ec: Ec::TLC_CONFIG_OP_NOT_IN_SPEC, params: vec![name()] })
            }
            Resolved::Value(v) | Resolved::Binding(crate::ctx::Binding::Value(v)) => {
                let shown = value_string(tool, &v, out)?;
                Err(SpecError::Config { ec: Ec::TLC_CONFIG_OP_IS_EQUAL, params: vec![name(), shown, "spec".to_string()] })
            }
            _ => {
                let _ = pred;
                Err(SpecError::Config { ec: Ec::TLC_CONFIG_OP_NOT_IN_SPEC, params: vec![name()] })
            }
        }
    }

    /// `[]A`: `[][Next]_v` is the next-state relation, anything else a temporal formula.
    fn config_spec_box(
        &mut self,
        pred: ExprId,
        args: &[ExprOrOpArg],
        c: &Ctx,
        tool: &Tool,
        out: &dyn Report,
    ) -> Result<(), SpecError> {
        let graph = tool.graph.clone();
        let box_arg = args[0].expect_expr();
        let mut locals = WorkerLocals::new(0, tool.globals.seed);
        if let ExprNode::OpAppl { op, args: inner, .. } = graph.expr(box_arg) {
            let (op, inner) = (*op, inner.clone());
            // TLC cannot handle `[]S` for a constant- or state-level formula.
            if graph.level(box_arg) <= Level::State {
                let shown = graph.expr_tlc_text(box_arg);
                return Err(SpecError::Config { ec: Ec::TLC_LIVE_CANNOT_HANDLE_FORMULA, params: vec![shown] });
            }
            let code = tool.ev(&mut locals, out).opcode(op);
            if code == Some(Builtin::SquareAct) {
                let action = inner[0].expect_expr();
                let subscript = inner[1].expect_expr();
                check_subscript(tool, subscript, c, out)?;
                if self.next_pred.is_some() {
                    return Err(SpecError::Failure { ec: Ec::TLC_CANT_HANDLE_TOO_MANY_NEXT_STATE_RELS });
                }
                self.next_pred = Some(Pred { expr: action, ctx: c.clone(), op_def: None });
                return Ok(());
            }
        }
        self.temporals.push(Pred { expr: pred, ctx: c.clone(), op_def: None });
        Ok(())
    }

    /// The level-based tail of `processConfigSpec`.
    fn config_spec_level(&mut self, pred: ExprId, c: &Ctx, tool: &Tool, out: &dyn Report) -> Result<(), SpecError> {
        let mut locals = WorkerLocals::new(0, tool.globals.seed);
        let level = {
            let mut ev = tool.ev(&mut locals, out);
            ev.level_bound(pred, c)
        };
        match level {
            0 | 1 => {
                self.init_preds.push(Pred { expr: pred, ctx: c.clone(), op_def: None });
                Ok(())
            }
            3 => {
                self.temporals.push(Pred { expr: pred, ctx: c.clone(), op_def: None });
                Ok(())
            }
            _ => {
                let shown = tool.graph.expr_tlc_text(pred);
                Err(SpecError::Config { ec: Ec::TLC_CANT_HANDLE_CONJUNCT, params: vec![shown] })
            }
        }
    }
}

/// `SubscriptCollector` (`SpecProcessor.java:1180-1344`): the variables a `[][Next]_v`
/// subscript mentions, and the warning per variable it leaves out.
fn check_subscript(tool: &Tool, subscript: ExprId, c: &Ctx, out: &dyn Report) -> Result<(), SpecError> {
    let mut found = Vec::new();
    let mut locals = WorkerLocals::new(0, tool.globals.seed);
    let collected = {
        let mut ev = tool.ev(&mut locals, out);
        collect_subscript(&mut ev, subscript, c, &mut found)
    };
    if collected.is_err() {
        out.warning(Ec::TLC_COULD_NOT_DETERMINE_SUBSCRIPT, &[]).map_err(warning_as_error)?;
        return Ok(());
    }
    for (i, &name) in tool.vars.names.iter().enumerate() {
        if !found.contains(&tool.vars.slot(i)) {
            let text = tool.graph.interner().text(name).to_string();
            out.warning(Ec::TLC_SUBSCRIPT_CONTAIN_NO_STATE_VAR, &[&text]).map_err(warning_as_error)?;
        }
    }
    Ok(())
}

/// `SubscriptCollector.enter`. The `Assert.fail(TLC_CANT_HANDLE_SUBSCRIPT, ...)` it raises on
/// an expression it cannot walk is caught by the caller as "could not determine".
fn collect_subscript(ev: &mut crate::eval::Ev<'_>, subscript: ExprId, c: &Ctx, found: &mut Vec<crate::state::VarSlot>) -> Result<(), ()> {
    if let Some(slot) = ev.get_var(subscript, c, false) {
        found.push(slot);
        return Ok(());
    }
    let graph = ev.tool().graph.clone();
    match graph.expr(subscript) {
        ExprNode::OpAppl { op, args, .. } => {
            let (op, args) = (*op, args.clone());
            match ev.opcode(op) {
                Some(Builtin::Tuple) => {
                    for a in args.iter() {
                        collect_subscript(ev, a.expect_expr(), c, found)?;
                    }
                    Ok(())
                }
                // Any other built-in operator is ignored.
                Some(_) => Ok(()),
                None => match ev.lookup(op, c, StateRef::Empty, false) {
                    crate::eval::expr::Resolved::UserOp(d) | crate::eval::expr::Resolved::Binding(crate::ctx::Binding::OpDef(d)) => {
                        let Some(body) = graph.op_def(d).body else { return Ok(()) };
                        let c1 = ev.op_context(d, &args, c, false);
                        collect_subscript(ev, body, &c1, found)
                    }
                    crate::eval::expr::Resolved::Binding(crate::ctx::Binding::Thunk(t)) => {
                        let (e, tc) = (t.expr, t.ctx.clone());
                        collect_subscript(ev, e, &tc, found)
                    }
                    // An overridden operator is ignored.
                    _ => Ok(()),
                },
            }
        }
        ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
            let body = *body;
            let c1 = ev.subst_context(substs, c);
            collect_subscript(ev, body, &c1, found)
        }
        ExprNode::LetIn { body, .. } | ExprNode::Label { body, .. } => collect_subscript(ev, *body, c, found),
        _ => Err(()),
    }
}

/// `processModelConstraints` / `processActionConstraints` (`SpecProcessor.java:2196-2260`):
/// each name must be a 0-arity definition.
fn named_checks(
    names: &[String],
    tool: &Tool,
    defns: &Defns,
    out: &dyn Report,
    what: &str,
) -> Result<Vec<NamedCheck>, SpecError> {
    let graph = tool.graph.clone();
    let mut checks = Vec::new();
    for name in names {
        match defn_of(defns, &graph, name) {
            Some(OpBinding::Replaced(d)) => {
                if !graph.op_def(d).params.is_empty() {
                    return Err(SpecError::Config {
                        ec: Ec::TLC_CONFIG_ID_REQUIRES_NO_ARG,
                        params: vec![what.to_string(), name.clone()],
                    });
                }
                let body = graph.op_def(d).body.expect("a user definition has a body");
                checks.push(NamedCheck { name: graph.interner().intern(name), expr: body, ctx: Ctx::EMPTY });
            }
            None => {
                return Err(SpecError::Config {
                    ec: Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED,
                    params: vec![what.to_string(), name.clone()],
                });
            }
            // A constraint that folded to TRUE is accepted and contributes nothing; any
            // other value is rejected (`SpecProcessor.java:1684-1692`).
            Some(OpBinding::Value(Value::Bool(true))) => {}
            Some(other) => {
                let shown = binding_string(tool, &other, out)?;
                return Err(SpecError::Config {
                    ec: Ec::TLC_CONFIG_ID_HAS_VALUE,
                    params: vec![what.to_string(), name.clone(), shown],
                });
            }
        }
    }
    Ok(checks)
}

/// `processConfigPossible`'s validation loop (`SpecProcessor.java:1764-1790`). Every name
/// must be a 0-arity, non-temporal definition. Witnessing them needs the module TLC
/// generates, parses and analyses at run time, which is not here yet.
fn config_possible(config: &ModelConfig, tool: &Tool, defns: &Defns) -> Result<(), SpecError> {
    if config.possible.is_empty() {
        return Ok(());
    }
    let graph = tool.graph.clone();
    for name in &config.possible {
        let fail = |ec: Ec| SpecError::Config { ec, params: vec!["possible".to_string(), name.clone()] };
        match defn_of(defns, &graph, name) {
            None => return Err(fail(Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED)),
            Some(OpBinding::Replaced(d)) => {
                if !graph.op_def(d).params.is_empty() {
                    return Err(fail(Ec::TLC_CONFIG_ID_REQUIRES_NO_ARG));
                }
                // https://github.com/tlaplus/tlaplus/issues/827
                if graph.op_level(d) >= Level::Temporal {
                    return Err(SpecError::Config {
                        ec: Ec::TLC_CONFIG_ID_HAS_VALUE,
                        params: vec![
                            "possible".to_string(),
                            name.clone(),
                            "a temporal formula; only state- and action-level predicates are supported".to_string(),
                        ],
                    });
                }
            }
            Some(_) => return Err(fail(Ec::TLC_CONFIG_ID_MUST_NOT_BE_CONSTANT)),
        }
    }
    Err(unimplemented("_POSSIBLE's generated module and its named registers (M3 unit 5)"))
}

/// `Spec.hasAlias` + `Spec.getAliasSpec` (`Spec.java:274-304`). TLC calls `getAliasSpec`
/// once per printed trace state, so the lookup's failure is carried instead of raised: the
/// name resolves against `defns`, which means a constant-level ALIAS has been folded to a
/// value and fails as a constant.
fn config_alias(config: &ModelConfig, tool: &Tool, defns: &Defns, out: &dyn Report) -> Option<crate::tool::AliasSpec> {
    let name = config.alias.as_deref().filter(|s| !s.is_empty())?;
    let graph = tool.graph.clone();
    let fail = |ec: Ec| Err(out.text(ec, &["alias", name]));
    let body = match defn_of(defns, &graph, name) {
        None => fail(Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED),
        Some(OpBinding::Replaced(d)) if !graph.op_def(d).params.is_empty() => fail(Ec::TLC_CONFIG_ID_REQUIRES_NO_ARG),
        Some(OpBinding::Replaced(d)) => Ok(graph.op_def(d).body.expect("a user definition has a body")),
        Some(_) => fail(Ec::TLC_CONFIG_ID_MUST_NOT_BE_CONSTANT),
    };
    Some(crate::tool::AliasSpec { body, trace_symbol: trace_symbol(&graph) })
}

/// `emt.getContext("TLCExt").getSymbol("Trace")`: the symbol `evalAlias` binds the printed
/// prefix to, or `None` when the module is not in scope and `Trace` cannot appear.
fn trace_symbol(graph: &SemanticGraph) -> Option<SymbolRef> {
    let (_, module) = graph.modules().find(|(_, m)| &*graph.interner().text(m.name) == "TLCExt")?;
    let d = *module.op_defs.iter().find(|&&d| &*graph.interner().text(graph.op_def(d).name) == "Trace")?;
    Some(SymbolRef::OpDef(d))
}

/// `Spec.getPostConditionSpecs` (`Spec.java:406-435`), which `checkPostCondition` calls
/// when the search is over. Every name must be a 0-arity definition of `defns`; the first
/// one that is not raises an `Assert.fail` that escapes `checkPostCondition`, so TLC then
/// checks nothing at all. The lookups are pure, so they run here and the message waits.
fn config_postconditions(config: &ModelConfig, tool: &Tool, defns: &Defns, out: &dyn Report) -> (Box<[NamedCheck]>, Option<String>) {
    let graph = tool.graph.clone();
    let mut checks = Vec::new();
    for name in &config.postconditions {
        let fail = |ec: Ec| Some(out.text(ec, &["post condition", name]));
        match defn_of(defns, &graph, name) {
            None => return (Box::new([]), fail(Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED)),
            Some(OpBinding::Replaced(d)) => {
                if !graph.op_def(d).params.is_empty() {
                    return (Box::new([]), fail(Ec::TLC_CONFIG_ID_REQUIRES_NO_ARG));
                }
                let body = graph.op_def(d).body.expect("a user definition has a body");
                checks.push(NamedCheck { name: graph.interner().intern(name), expr: body, ctx: Ctx::EMPTY });
            }
            // `processModuleOverrides` puts a `defns` entry only for a definition that has a
            // matching Java method; tlars binds every definition of a `tlc2.module` class,
            // so a name that lands here may well be an ordinary definition for TLC.
            Some(OpBinding::Unimplemented) => {
                let what = format!("the Java override of the POSTCONDITION {name} (M3 unit 6)");
                return (Box::new([]), Some(format!("tlars does not implement {what} yet")));
            }
            Some(_) => return (Box::new([]), fail(Ec::TLC_CONFIG_ID_MUST_NOT_BE_CONSTANT)),
        }
    }
    (checks.into(), None)
}

/// `Tool.getSymmetryPerms` (`Tool.java:3836-3943`), the other half of what
/// `TLCStateMut.setTool` computes. The name resolves against `unprocessedDefns`, the
/// snapshot taken *before* `processConstantDefns`, so a constant-level symmetry definition
/// is still a definition here — unlike VIEW and ALIAS.
fn config_symmetry(
    config: &ModelConfig,
    tool: &Tool,
    unprocessed: &Defns,
    out: &dyn Report,
) -> Result<Box<[crate::symmetry::MvPerm]>, SpecError> {
    let Some(name) = config.symmetry.as_deref().filter(|s| !s.is_empty()) else { return Ok(Box::new([])) };
    let graph = tool.graph.clone();
    let d = match defn_of(unprocessed, &graph, name) {
        None => {
            return Err(SpecError::Config {
                ec: Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED,
                params: vec!["symmetry function".to_string(), name.to_string()],
            });
        }
        Some(OpBinding::Replaced(d)) => d,
        // `Assert.fail(String)`: an exception nobody meant to raise, so `TLC.process`
        // reports it as a `java.lang.RuntimeException`.
        Some(_) => {
            return Err(SpecError::Unexpected {
                class: "java.lang.RuntimeException",
                message: format!("The symmetry function {name} must specify a set of permutations."),
            });
        }
    };
    let body = graph.op_def(d).body.expect("a user definition has a body");
    let mut locals = WorkerLocals::new(0, tool.globals.seed);
    let mut ev = tool.ev(&mut locals, out);
    let fcns = ev.eval(body, &Ctx::EMPTY, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR).map_err(uncaught)?;
    // `!(fcns instanceof Enumerable) || !(fcns instanceof SetEnumValue)`.
    let Value::SetEnum(set) = &fcns else {
        return Err(SpecError::Unexpected {
            class: "java.lang.RuntimeException",
            message: "The symmetry operator must specify a set of functions.".to_string(),
        });
    };
    let values = set.snapshot().to_vec();
    let mut generators = Vec::with_capacity(values.len());
    for v in &values {
        let Value::FcnRcd(fcn) = v else {
            return Err(SpecError::Unexpected {
                class: "java.lang.RuntimeException",
                message: "The symmetry values must be function records.".to_string(),
            });
        };
        let (domain, images) = fcn.snapshot();
        let Some(domain) = domain else {
            return Err(unimplemented("a SYMMETRY function whose domain is an interval (TLC raises a NullPointerException)"));
        };
        let mut pairs = Vec::with_capacity(images.len());
        for (dval, rval) in domain.iter().zip(images.iter()) {
            match (dval, rval) {
                (Value::Model(a), Value::Model(b)) => pairs.push((*a, *b)),
                _ => {
                    return Err(SpecError::Unexpected {
                        class: "java.lang.RuntimeException",
                        message: "Symmetry function must have model values as domain and range.".to_string(),
                    });
                }
            }
        }
        generators.push(crate::symmetry::Generator { pairs });
    }
    // `argNodes.length >= values.size()`: as many permuted sets as permutations means every
    // symmetry set holds at most one element, and TLC then warns and uses no symmetry at
    // all. Naming the offending sets needs `SyntaxTreeNode.getHumanReadableImage`, which the
    // frozen graph does not carry.
    let arg_count = match graph.expr(body) {
        ExprNode::OpAppl { args, .. } => args.len(),
        // `(OpApplNode) opDef.getBody()` on another node kind.
        _ => return Err(unimplemented("a SYMMETRY whose definition body is not an operator application")),
    };
    if arg_count >= values.len() {
        return Err(unimplemented("the SYMMETRY too-small-set warning (2300), which needs SemanticNode.getHumanReadableImage (M3)"));
    }
    let subgroup = crate::symmetry::permutation_group(&mut ev, &generators, &tool.model_values).map_err(uncaught)?;
    // The second offender check only looks at `Permutations(S)` where `S` is a name the cfg
    // bound to a set, so it is also reachable only with an image to print.
    let members: Vec<Atom> = subgroup.iter().flat_map(|p| p.images()).map(|mv| mv.name).collect();
    for arg in permutation_set_args(tool, body) {
        let elems = arg.snapshot().to_vec();
        let found = elems.iter().any(|e| matches!(e, Value::Model(mv) if members.contains(&mv.name)));
        if !found {
            return Err(unimplemented("the SYMMETRY too-small-set warning (2300), which needs SemanticNode.getHumanReadableImage (M3)"));
        }
    }
    Ok(subgroup.into())
}

/// `getSetEnumValueFromArgumentNode` over every argument of the symmetry definition's body
/// (`Tool.java:3944-3971`): the set a `Permutations(S)` argument permutes, but only where
/// `S` is a name whose tool object is a `SetEnumValue`, which is what a cfg `S = {..}`
/// leaves behind. An inline `Permutations({A, B})` yields nothing.
fn permutation_set_args(tool: &Tool, body: ExprId) -> Vec<std::sync::Arc<crate::value::SetEnumValue>> {
    let graph = &tool.graph;
    let ExprNode::OpAppl { args, .. } = graph.expr(body) else { return Vec::new() };
    let mut sets = Vec::new();
    for arg in args.iter() {
        let ExprOrOpArg::Expr(node) = arg else { continue };
        let ExprNode::OpAppl { op, args: operands, .. } = graph.expr(*node) else { continue };
        let SymbolRef::OpDef(outer) = op else { continue };
        if &*graph.interner().text(graph.op_def(*outer).name) != "Permutations" || operands.len() != 1 {
            continue;
        }
        let ExprOrOpArg::Expr(inner) = &operands[0] else { continue };
        let ExprNode::OpAppl { op: inner_op, .. } = graph.expr(*inner) else { continue };
        let bound = match inner_op {
            SymbolRef::Decl(dec) => match &tool.bindings.decls[dec.index()] {
                Some(DeclBinding::Value(v)) => Some(v.clone()),
                _ => None,
            },
            SymbolRef::OpDef(d) => match tool.bindings.op_def(*d) {
                Some(OpBinding::Value(v)) => Some(v.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(Value::SetEnum(set)) = bound {
            sets.push(set);
        }
    }
    sets
}

/// `Spec.getViewSpec` (`Spec.java:243-272`), which `TLCStateMut.setTool` calls from the
/// `Tool` constructor: after `processConfig` and before the next-state relation is split.
/// The name resolves against `defns`, so a VIEW whose definition `processConstantDefns`
/// folded to a value is "not a definition" and fails as a constant.
fn config_view(config: &ModelConfig, tool: &Tool, defns: &Defns) -> Result<Option<NamedCheck>, SpecError> {
    let Some(name) = config.view.as_deref().filter(|s| !s.is_empty()) else { return Ok(None) };
    let graph = tool.graph.clone();
    match defn_of(defns, &graph, name) {
        None => Err(SpecError::Config {
            ec: Ec::TLC_CONFIG_SPECIFIED_NOT_DEFINED,
            params: vec!["view function".to_string(), name.to_string()],
        }),
        Some(OpBinding::Replaced(d)) => {
            if !graph.op_def(d).params.is_empty() {
                return Err(SpecError::Config {
                    ec: Ec::TLC_CONFIG_ID_REQUIRES_NO_ARG,
                    params: vec!["view function".to_string(), name.to_string()],
                });
            }
            let body = graph.op_def(d).body.expect("a user definition has a body");
            Ok(Some(NamedCheck { name: graph.interner().intern(name), expr: body, ctx: Ctx::EMPTY }))
        }
        Some(_) => Err(SpecError::Config {
            ec: Ec::TLC_CONFIG_ID_MUST_NOT_BE_CONSTANT,
            params: vec!["view function".to_string(), name.to_string()],
        }),
    }
}

/// `this.defns.get(name)`, which interns the cfg identifier.
fn defn_of(defns: &Defns, graph: &SemanticGraph, name: &str) -> Option<OpBinding> {
    defns.get(&graph.interner().intern(name)).cloned().flatten()
}

/// `val.toString()` of whatever `defns` held, as the `%3%` of `TLC_CONFIG_ID_HAS_VALUE`.
fn binding_string(tool: &Tool, binding: &OpBinding, out: &dyn Report) -> Result<String, SpecError> {
    match binding {
        OpBinding::Value(v) => value_string(tool, v, out),
        OpBinding::Table(lines) => {
            let mut locals = WorkerLocals::new(0, tool.globals.seed);
            let mut ev = tool.ev(&mut locals, out);
            crate::value::print::op_rcd_string(&mut ev, lines).map_err(uncaught)
        }
        OpBinding::Native(id) => Ok(format!("<Java Method: {}>", id.op().signature)),
        OpBinding::Replaced(_) | OpBinding::Unimplemented => Err(unimplemented("the printed form of this cfg binding")),
    }
}

fn value_string(tool: &Tool, v: &Value, out: &dyn Report) -> Result<String, SpecError> {
    let mut locals = WorkerLocals::new(0, tool.globals.seed);
    let mut ev = tool.ev(&mut locals, out);
    v.to_tlc_string(&mut ev).map_err(uncaught)
}

/// `-messagesAsErrors` turns a warning into `Assert.fail(code, params)`.
fn warning_as_error(e: tlars_msg::WarningAsError) -> SpecError {
    SpecError::Config { ec: e.ec, params: vec![e.message] }
}
