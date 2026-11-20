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

impl VarTable {
    /// `UniqueString.getVarLoc()`: the slot the name was given by `setLoc`. Keyed by name, as
    /// in TLC, so a variable an instantiated module declares under the same name shares it.
    pub(crate) fn slot_of(&self, name: Atom) -> Option<VarSlot> {
        self.names.iter().position(|n| *n == name).map(|i| VarSlot(i as u16))
    }

    pub(crate) fn slot(&self, index: usize) -> VarSlot {
        VarSlot(index as u16)
    }
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
    pub(crate) fn values(&self) -> &[Value] {
        &self.0
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot(self.0.iter().map(|v| Some(v.clone())).collect())
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
    /// `TLCStateMut.getUnassigned()`, in the order it prints them: sorted by variable name.
    pub(crate) fn unassigned(&self, vars: &VarTable, interner: &tlars_sany::Interner) -> Vec<String> {
        let mut names: Vec<String> = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_none())
            .map(|(i, _)| interner.text(vars.names[i]).to_string())
            .collect();
        names.sort();
        names
    }
    /// `isGoodState` + copy: `None` if some variable is unassigned.
    pub(crate) fn freeze(&self) -> Option<State> {
        if !self.all_assigned() {
            return None;
        }
        Some(State(self.0.iter().map(|v| v.clone().expect("all assigned")).collect()))
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot(self.0.clone())
    }
    pub(crate) fn copy(&self) -> PartialState {
        PartialState(self.0.clone())
    }
    pub(crate) fn from_values(values: &[Value]) -> PartialState {
        PartialState(values.iter().map(|v| Some(v.clone())).collect())
    }
}

/// `TLCStateFun`: persistent bindings for ENABLED (bind returns a new state).
pub struct FunState {
    /// Cons list of `(slot, value)`, newest first, as `TLCStateFun.bind` conses.
    cell: Option<Arc<FunCell>>,
}

struct FunCell {
    slot: VarSlot,
    value: Value,
    next: Option<Arc<FunCell>>,
}

impl FunState {
    pub(crate) const EMPTY: FunState = FunState { cell: None };

    pub(crate) fn bind(&self, slot: VarSlot, value: Value) -> FunState {
        FunState { cell: Some(Arc::new(FunCell { slot, value, next: self.cell.clone() })) }
    }

    pub(crate) fn lookup(&self, slot: VarSlot) -> Option<&Value> {
        let mut cur = self.cell.as_deref();
        while let Some(cell) = cur {
            if cell.slot == slot {
                return Some(&cell.value);
            }
            cur = cell.next.as_deref();
        }
        None
    }
}

/// What `eval` reads variables from. `Empty` is TLC's `TLCState.Empty`, `Null` its
/// `TLCState.Null` (a `null` reference): both hold no variable, but a `null` successor also
/// stops `LazyValue` from reusing a cached value.
#[derive(Clone, Copy)]
pub enum StateRef<'a> {
    Empty,
    Null,
    Full(&'a State),
    Partial(&'a PartialState),
    Fun(&'a FunState),
}

impl StateRef<'_> {
    /// `s.lookup(name)`: the value of a variable, or `None` where TLC's state returns `null`.
    pub(crate) fn get(&self, slot: VarSlot) -> Option<&Value> {
        match self {
            StateRef::Empty | StateRef::Null => None,
            StateRef::Full(s) => Some(s.get(slot)),
            StateRef::Partial(p) => p.lookup(slot),
            StateRef::Fun(f) => f.lookup(slot),
        }
    }

    /// `TLCState.Null`, which `LazyValue.getValue` treats differently from `Empty`.
    pub(crate) fn is_null(&self) -> bool {
        matches!(self, StateRef::Null)
    }

    /// `s.copy()` as `LazyValue`'s cache keeps it; `None` for `Null`.
    pub(crate) fn snapshot(&self) -> Option<Snapshot> {
        match self {
            StateRef::Null => None,
            StateRef::Empty => Some(Snapshot(Box::new([]))),
            StateRef::Full(s) => Some(s.snapshot()),
            StateRef::Partial(p) => Some(p.snapshot()),
            // `TLCStateFun.copy` is never reached: ENABLED evaluates with KeepLazy cleared
            // and the thunks it conses are not cachable.
            StateRef::Fun(_) => None,
        }
    }
}

/// Copy of the assigned part of a state, for `LazyValue` cache guards (`TLCState.isSubset`).
pub(crate) struct Snapshot(Box<[Option<Value>]>);

impl Snapshot {
    /// `TLCState.isSubset(s1, s2)` (`TLCState.java:259-291`), with `self` as `s2`: every
    /// variable `self` has a value for has the same value in `s1`. An equality that throws
    /// makes the answer unknown, which is not a cache hit.
    pub(crate) fn covers(&self, ev: &mut crate::eval::Ev<'_>, s1: &StateRef<'_>) -> bool {
        let slots = self.0.len();
        for i in 0..slots {
            let Some(val2) = self.0[i].clone() else { continue };
            let val1 = s1.get(crate::state::VarSlot(i as u16)).cloned();
            let equal = match val1 {
                None => false,
                Some(val1) => matches!(val1.tlc_equals(ev, &val2), Ok(true)),
            };
            if !equal {
                return false;
            }
        }
        true
    }

    /// An empty snapshot stands for `TLCState.Empty`, which every state is a subset of.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
