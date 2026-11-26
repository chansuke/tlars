//! The frozen `Tool`: everything SpecProcessor and the `Tool` constructor computed, shared
//! read-only by every worker (`Arc<Tool>`). Per-worker state lives in `WorkerLocals`.

use crate::actions::{Action, ActionId};
use crate::ctx::Ctx;
use crate::eval::{Ev, WorkerLocals};
use crate::fp64::Fp64;
use crate::globals::TlcGlobals;
use crate::spec::Bindings;
use crate::state::VarTable;
use crate::symmetry::MvPerm;
use crate::value::{ModelValue, Value};
use std::marker::PhantomData;
use std::sync::Arc;
use tlars_msg::Report;
use tlars_sany::Atom;
use tlars_sany::graph::{ExprId, OpDefId, SemanticGraph};

/// Kinds of named checks; an id of one kind cannot be passed where another is expected.
#[derive(Clone, Copy, Debug)]
pub enum Invariant {}
#[derive(Clone, Copy, Debug)]
pub enum ImpliedInit {}
#[derive(Clone, Copy, Debug)]
pub enum ImpliedAction {}

#[derive(Debug)]
pub struct CheckId<K>(u32, PhantomData<K>);

impl<K> Clone for CheckId<K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<K> Copy for CheckId<K> {}

impl<K> CheckId<K> {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// An INVARIANT / implied property / constraint: its cfg name, expression and context.
pub(crate) struct NamedCheck {
    pub(crate) name: Atom,
    pub(crate) expr: ExprId,
    pub(crate) ctx: Ctx,
    /// The definition the cfg named, when there was one. `TLCGet("spec")` reports a
    /// constraint as its `OpDefNode`, so it prints the whole definition's location rather
    /// than the body's (`TLCGetSet.constraint2Value`).
    pub(crate) op_def: Option<OpDefId>,
}

/// One cfg `_POSSIBLE` predicate. TLC witnesses it by generating a module that wraps the
/// predicate in `_Possible!_Track` and wires that in as a constraint, then checks
/// `_Possible!_CheckName` as a post-condition (`SpecProcessor.processConfigPossible`,
/// `PossibleAction`). tlars evaluates `_Track` and `_CheckName` directly instead of
/// generating the module, and keeps the same `s:_possible` register they share.
pub(crate) struct PossibleCheck {
    /// The definition's name, which is the key `_Track` counts under and the `%1%` of 2780.
    pub(crate) name: Atom,
    /// The definition's body: what `_Track` evaluates, and the location 2780 reports, since
    /// `PossibleAction.getPred()` hands back the user predicate's body.
    pub(crate) expr: ExprId,
    /// SANY's level of the definition. State level and below becomes a CONSTRAINT, action
    /// level an ACTION_CONSTRAINT (`SpecProcessor.java:1861-1865`).
    pub(crate) is_action: bool,
}

/// The cfg ALIAS: the definition body `getAliasSpec()` returns, or the framed message of
/// the `Assert.fail` it raises instead.
pub(crate) struct AliasSpec {
    pub(crate) body: Result<ExprId, String>,
    /// `TLCExt!Trace`, which `evalAlias` binds to the behaviour printed so far when the
    /// module is in scope.
    pub(crate) trace_symbol: Option<tlars_sany::graph::SymbolRef>,
}

/// `Spec.getCounterExampleDef()` (`Spec.java:337-356`): what `defns` holds for
/// `CounterExample`, which decides whether the counterexample is bound at all.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CounterExampleDef {
    /// `defns.get("CounterExample") == null`: not used anywhere in the spec, so the
    /// POSTCONDITIONs are checked with an empty context.
    Absent,
    /// The `EvaluatingValue` of `TLCExt!CounterExample`, whose `getOpDef()` is the name the
    /// counterexample is bound to while the POSTCONDITIONs are checked.
    Evaluating(OpDefId),
    /// Anything else under that name, a definition of the spec's own among them:
    /// `Assert.fail(EC.GENERAL)`, whatever the cfg asks for.
    NotEvaluating,
}

/// `TLCExt!TLCCache`'s `HashMap<Value, Value>` per expression node, which TLC keeps as a tool
/// object on the node (`expr.setToolObject`). Each entry holds the key's `Value.hashCode()`
/// so that a lookup compares only the keys a `HashMap` would.
pub(crate) type TlcCaches = std::sync::Mutex<std::collections::BTreeMap<ExprId, Vec<(i32, Value, Value)>>>;

/// Temporal part handed to the liveness checker (`Liveness.parseLiveness` input).
pub struct Temporal {
    pub fairness: Box<[(ExprId, Ctx)]>,
    pub properties: Box<[(Atom, ExprId, Ctx)]>,
}

pub struct Tool {
    pub(crate) graph: Arc<SemanticGraph>,
    /// `BuiltInOPs.getOpCode(name)` of every definition, by `OpDefId`, so an application does
    /// not look its operator's name up each time.
    pub(crate) builtins: Box<[Option<crate::eval::expr::Builtin>]>,
    pub(crate) bindings: Bindings,
    pub(crate) vars: VarTable,
    pub(crate) init_actions: Box<[Action]>,
    pub(crate) actions: Box<[Action]>,
    pub(crate) invariants: Box<[NamedCheck]>,
    pub(crate) implied_inits: Box<[NamedCheck]>,
    pub(crate) implied_actions: Box<[NamedCheck]>,
    pub(crate) model_constraints: Box<[NamedCheck]>,
    pub(crate) action_constraints: Box<[NamedCheck]>,
    pub(crate) view: Option<NamedCheck>,
    /// `Spec.hasAlias()` plus what `Spec.getAliasSpec()` would return. TLC resolves the
    /// name every time it prints a trace state, so a name it cannot resolve is reported per
    /// state as `_ALIASEvalError` rather than at startup.
    pub(crate) alias: Option<AliasSpec>,
    /// `Spec.getPostConditionSpecs()`, resolved here because the lookups and the arity
    /// checks are pure. TLC runs them when the search ends, so a bad name is reported after
    /// the search, not at startup: [`Tool::postcondition_error`] holds that message.
    pub(crate) postconditions: Box<[NamedCheck]>,
    /// The cfg's `_POSSIBLE` predicates, in cfg order. They are checked ahead of
    /// `postconditions`, as `getPostConditionSpecs()` orders them.
    pub(crate) possible: Box<[PossibleCheck]>,
    /// The `Assert.fail` that `getPostConditionSpecs` raises on the first name it cannot
    /// resolve. It leaves TLC with no postconditions at all, and no `checkPostCondition*`
    /// catches it, so each caller handles it where TLC does.
    pub(crate) postcondition_error: Option<crate::error::EvalError>,
    /// What `getCounterExampleDef()` finds, which every
    /// `checkPostConditionWithCounterExample` asks for before it looks at the
    /// POSTCONDITIONs.
    pub(crate) counter_example: CounterExampleDef,
    /// Closed permutation group of SYMMETRY (`MVPerms`), empty if none.
    pub(crate) symmetry: Box<[MvPerm]>,
    pub(crate) temporal: Temporal,
    pub(crate) fp64: Fp64,
    pub(crate) globals: TlcGlobals,
    pub(crate) tlc_caches: TlcCaches,
    /// `ModelValue.mvs`, in creation order (cfg first).
    pub(crate) model_values: Box<[ModelValue]>,
    /// What `ModelValue.add` created past the cfg's own, which is `TLCExt!TLCModelValue`.
    /// `mvTable` is one table in TLC, but the cfg's half is frozen before anything can
    /// evaluate, and only that half sizes the SYMMETRY permutations.
    pub(crate) added_model_values: std::sync::Mutex<Vec<ModelValue>>,
    /// `TLCSet`'s registers, which live in TLC's worker threads and so outlast any one
    /// `WorkerLocals` (`effects::Registers`).
    pub(crate) registers: std::sync::Mutex<crate::effects::Registers>,
    /// `TLCEval`'s tool objects: `arg.setToolObject(tool.getId(), eval)` for a constant
    /// argument evaluated with an empty context (`TLCEval.tlcEvalConst`). The tool objects
    /// `SpecProcessor` writes are in [`Bindings`], which is frozen with the `Tool`; this is
    /// the one slot a *run* writes, so it needs interior mutability. TLC guards its tool
    /// objects here with a single static `ReadWriteLock`, which this one mutex is. The lock
    /// is never held across an evaluation: Java's write lock is reentrant, so a nested
    /// `TLCEval` reacquires it, and a plain mutex gets the same per-expression value by
    /// releasing first.
    pub(crate) tlc_eval_values: std::sync::Mutex<std::collections::BTreeMap<ExprId, crate::value::Value>>,
    /// What `TLCSet("-Dkey", v)` wrote, newest last. TLC calls `System.setProperty`, a JVM
    /// global; tlars keeps it per run, so two runs in one process cannot see each other's.
    pub(crate) set_properties: std::sync::Mutex<Vec<(String, String)>>,
    /// `TLCGlobals.mainChecker` as `TLCGet`/`TLCSet` use it, set once by the checker. Unset
    /// until then, which is TLC's `null`.
    /// `System.in`: `TLCSet("pause", TRUE)` reads one byte from it under the lock TLC takes
    /// on the state queue, so one paused worker at a time prompts and waits.
    pub(crate) stdin: std::sync::Mutex<Box<dyn std::io::Read + Send>>,
    pub(crate) main_checker: std::sync::OnceLock<Arc<dyn crate::effects::MainChecker>>,
}

impl Tool {
    /// A worker's view of the tool. `out` is where `Print`, warnings and evaluation-time
    /// messages go.
    pub fn ev<'t>(&'t self, locals: &'t mut WorkerLocals, out: &'t dyn Report) -> Ev<'t> {
        Ev::new(self, locals, out)
    }

    pub fn graph(&self) -> &SemanticGraph {
        &self.graph
    }
    pub fn globals(&self) -> &TlcGlobals {
        &self.globals
    }
    /// `TLCGlobals.mainChecker = this`: the checker publishes itself for `TLCGet`/`TLCSet`.
    pub fn attach_main_checker(&self, checker: Arc<dyn crate::effects::MainChecker>) {
        let _ = self.main_checker.set(checker);
    }
    pub(crate) fn main_checker(&self) -> Option<&dyn crate::effects::MainChecker> {
        self.main_checker.get().map(|c| &**c)
    }
    /// `System.getProperty(name)`: what `TLCSet` wrote last, else the CLI's `-D` option.
    pub(crate) fn property(&self, name: &str) -> Option<String> {
        let set = self.set_properties.lock().expect("system properties");
        if let Some((_, v)) = set.iter().rev().find(|(k, _)| k == name) {
            return Some(v.clone());
        }
        self.globals.properties.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }
    /// `Boolean.getBoolean(name)`: true only for a property that reads `true`, in any case.
    pub(crate) fn property_is_true(&self, name: &str) -> bool {
        self.property(name).is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }
    /// `(HashMap<Value, Value>) expr.getToolObject(tool.getId())` for `TLCExt!TLCCache`: the
    /// entry whose stored hash and key match, if any (`HashMap.get`). The value's own hash
    /// decides which entries `equals` is called on, which is why a key of another kind never
    /// raises a comparison error.
    pub(crate) fn tlc_cache_get(&self, expr: ExprId, hash: i32) -> Vec<(Value, Value)> {
        let caches = self.tlc_caches.lock().expect("TLCCache tool objects");
        match caches.get(&expr) {
            None => Vec::new(),
            Some(entries) => entries.iter().filter(|(h, _, _)| *h == hash).map(|(_, k, v)| (k.clone(), v.clone())).collect(),
        }
    }
    /// `cache.put(key, value)`, allocating the map on the node the way `setToolObject` does.
    pub(crate) fn tlc_cache_put(&self, expr: ExprId, key: Value, hash: i32, value: Value) {
        let mut caches = self.tlc_caches.lock().expect("TLCCache tool objects");
        caches.entry(expr).or_default().push((hash, key, value));
    }
    /// `Spec.getCounterExampleDef()`'s `OpDefNode`: the definition a counterexample is bound
    /// to, if `TLCExt!CounterExample`'s Java override is the one under that name.
    pub(crate) fn counter_example_def(&self) -> Option<OpDefId> {
        match self.counter_example {
            CounterExampleDef::Evaluating(d) => Some(d),
            CounterExampleDef::Absent | CounterExampleDef::NotEvaluating => None,
        }
    }
    /// `ModelValue.add(str)` (`ModelValue.java:109-124`): the model value of that name, which
    /// is created with the next creation index when the table has none. `None` is the
    /// `count > Short.MAX_VALUE` refusal of `ModelValue`'s constructor.
    pub(crate) fn add_model_value(&self, name: Atom, text: &str) -> Option<ModelValue> {
        if let Some(mv) = self.model_values.iter().find(|mv| mv.name == name) {
            return Some(*mv);
        }
        let mut added = self.added_model_values.lock().expect("model values");
        if let Some(mv) = added.iter().find(|mv| mv.name == name) {
            return Some(*mv);
        }
        let count = self.model_values.len() + added.len();
        if count > i16::MAX as usize {
            return None;
        }
        let mv = ModelValue::new(name, count as u16, text);
        added.push(mv);
        Some(mv)
    }
    /// `ModelValue.mvs[index]`: the model value with that creation index, counting the ones
    /// `ModelValue.add` made during the run.
    pub(crate) fn model_value_at(&self, index: i16) -> Option<ModelValue> {
        let index = usize::try_from(index).ok()?;
        match self.model_values.get(index) {
            Some(mv) => Some(*mv),
            None => {
                let added = self.added_model_values.lock().expect("model values");
                added.get(index - self.model_values.len()).copied()
            }
        }
    }

    /// `ModelValue.mvs.length`, which an out-of-bounds read reports.
    pub(crate) fn model_value_count(&self) -> usize {
        self.model_values.len() + self.added_model_values.lock().expect("model values").len()
    }

    /// `System.setProperty(name, value)`.
    pub(crate) fn set_property(&self, name: &str, value: String) {
        let mut set = self.set_properties.lock().expect("system properties");
        match set.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = value,
            None => set.push((name.to_string(), value)),
        }
    }
    /// Next-state actions in split order (= successor generation order).
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }
    pub fn action(&self, id: ActionId) -> &Action {
        &self.actions[id.index()]
    }
    pub(crate) fn init_actions(&self) -> &[Action] {
        &self.init_actions
    }
    pub(crate) fn vars(&self) -> &VarTable {
        &self.vars
    }
    /// Whether INIT (or SPECIFICATION) gave any initial predicate.
    pub fn has_init_predicates(&self) -> bool {
        !self.init_actions.is_empty()
    }
    pub fn var_count(&self) -> usize {
        self.vars.decls.len()
    }
    pub fn invariants(&self) -> impl Iterator<Item = CheckId<Invariant>> + use<> {
        (0..self.invariants.len() as u32).map(|i| CheckId(i, PhantomData))
    }
    pub fn implied_inits(&self) -> impl Iterator<Item = CheckId<ImpliedInit>> + use<> {
        (0..self.implied_inits.len() as u32).map(|i| CheckId(i, PhantomData))
    }
    pub fn implied_actions(&self) -> impl Iterator<Item = CheckId<ImpliedAction>> + use<> {
        (0..self.implied_actions.len() as u32).map(|i| CheckId(i, PhantomData))
    }
    pub fn invariant_name(&self, id: CheckId<Invariant>) -> Atom {
        self.invariants[id.0 as usize].name
    }
    pub fn implied_action_name(&self, id: CheckId<ImpliedAction>) -> Atom {
        self.implied_actions[id.0 as usize].name
    }
    /// `!livenessIsTrue()`: only a temporal PROPERTY turns liveness checking on; fairness
    /// alone is never checked (`Spec.java:358-360`).
    pub fn has_liveness(&self) -> bool {
        !self.temporal.properties.is_empty()
    }
    pub fn temporal(&self) -> &Temporal {
        &self.temporal
    }
    /// `Spec.hasAlias()`.
    pub fn has_alias(&self) -> bool {
        self.alias.is_some()
    }
    pub fn has_symmetry(&self) -> bool {
        !self.symmetry.is_empty()
    }
}
