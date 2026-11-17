//! `TLCStateMut` (complete), the partially bound successor TLC builds with `bind`/`unbind`,
//! and `TLCStateFun` (ENABLED). Values are indexed by [`VarSlot`] in `vars[]` order, which
//! is SANY's reversed-Hashtable order (`SpecProcessor.java:473-482`).

use crate::value::Value;
use std::sync::Arc;
use tlars_sany::Atom;
use tlars_sany::graph::DeclId;

/// Index into `vars[]`. Replaces TLC's `UniqueString.setLoc`/`getVarLoc`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct VarSlot(u16);

impl VarSlot {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// `vars[]`: the single source of variable order for fingerprint, symmetry compare and
/// printing. Built once from `ModuleNode::variable_decls` of the root module.
pub(crate) struct VarTable {
    pub(crate) decls: Box<[DeclId]>,
    pub(crate) names: Box<[Atom]>,
}

/// A complete state. Cheap to clone; shared across workers.
#[derive(Clone)]
pub struct State(Arc<[Value]>);

impl State {
    pub fn get(&self, slot: VarSlot) -> &Value {
        &self.0[slot.index()]
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// The successor (or initial state) under construction. Generation binds a slot, recurses
/// into the continuation, then unbinds: exactly `TLCState.bind` / `unbind`.
pub struct PartialState(Box<[Option<Value>]>);

impl PartialState {
    pub(crate) fn empty(vars: usize) -> Self {
        PartialState(vec![None; vars].into_boxed_slice())
    }
    pub(crate) fn bind(&mut self, slot: VarSlot, v: Value) {
        self.0[slot.index()] = Some(v);
    }
    pub(crate) fn unbind(&mut self, slot: VarSlot) {
        self.0[slot.index()] = None;
    }
    pub fn lookup(&self, slot: VarSlot) -> Option<&Value> {
        self.0[slot.index()].as_ref()
    }
    pub fn all_assigned(&self) -> bool {
        self.0.iter().all(Option::is_some)
    }
    /// `isGoodState` + copy: `None` if some variable is unassigned.
    pub(crate) fn freeze(&self) -> Option<State> {
        todo!()
    }
}

/// `TLCStateFun`: persistent bindings for ENABLED (bind returns a new state).
pub struct FunState {
    // cons list of (slot, value)
}

/// What `eval` reads variables from. `Empty` is TLC's `TLCState.Empty`.
#[derive(Clone, Copy)]
pub enum StateRef<'a> {
    Empty,
    Full(&'a State),
    Partial(&'a PartialState),
    Fun(&'a FunState),
}

/// Copy of the assigned part of a state, for `LazyValue` cache guards (`TLCState.isSubset`).
pub(crate) struct Snapshot(Box<[Option<Value>]>);
