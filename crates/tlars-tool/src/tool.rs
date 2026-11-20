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
use crate::value::ModelValue;
use std::marker::PhantomData;
use std::sync::Arc;
use tlars_msg::Report;
use tlars_sany::Atom;
use tlars_sany::graph::{ExprId, SemanticGraph};

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
}

/// Temporal part handed to the liveness checker (`Liveness.parseLiveness` input).
pub struct Temporal {
    pub fairness: Box<[(ExprId, Ctx)]>,
    pub properties: Box<[(Atom, ExprId, Ctx)]>,
}

pub struct Tool {
    pub(crate) graph: Arc<SemanticGraph>,
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
    pub(crate) alias: Option<NamedCheck>,
    pub(crate) postconditions: Box<[NamedCheck]>,
    /// Closed permutation group of SYMMETRY (`MVPerms`), empty if none.
    pub(crate) symmetry: Box<[MvPerm]>,
    pub(crate) temporal: Temporal,
    pub(crate) fp64: Fp64,
    pub(crate) globals: TlcGlobals,
    /// `ModelValue.mvs`, in creation order (cfg first).
    pub(crate) model_values: Box<[ModelValue]>,
    /// `TLCSet`'s registers, which live in TLC's worker threads and so outlast any one
    /// `WorkerLocals` (`effects::Registers`).
    pub(crate) registers: std::sync::Mutex<crate::effects::Registers>,
    /// What `TLCSet("-Dkey", v)` wrote, newest last. TLC calls `System.setProperty`, a JVM
    /// global; tlars keeps it per run, so two runs in one process cannot see each other's.
    pub(crate) set_properties: std::sync::Mutex<Vec<(String, String)>>,
    /// `TLCGlobals.mainChecker` as `TLCGet`/`TLCSet` use it, set once by the checker. Unset
    /// until then, which is TLC's `null`.
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
    pub fn has_liveness(&self) -> bool {
        !self.temporal.properties.is_empty() || !self.temporal.fairness.is_empty()
    }
    pub fn temporal(&self) -> &Temporal {
        &self.temporal
    }
    pub fn has_symmetry(&self) -> bool {
        !self.symmetry.is_empty()
    }
}
