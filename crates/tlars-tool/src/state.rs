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

/// `TLCState.INIT_LEVEL`: an initial state is at level 1, so that `TLCGet("level")` in the
/// first evaluation of the next-state relation is 1 (`TLCState.java:32-38`).
pub const INIT_LEVEL: u32 = 1;

/// A complete state. Cheap to clone; shared across workers.
#[derive(Clone)]
pub struct State {
    values: Arc<[Value]>,
    /// `TLCState.level`, as `setPredecessor` maintains it: one more than the predecessor's.
    /// `TLCGet("level")` is the only reader; no equality, fingerprint or printing sees it.
    level: u32,
    /// `TLCState.uid`, which `Worker.writeState` sets to the trace record the state was
    /// written at; `None` is `TLCState.INIT_UID`, "not in the trace yet". `TLCExt!Trace` is
    /// the only reader, and like `level` it takes no part in equality, fingerprints or
    /// printing.
    uid: Option<TraceUid>,
}

/// The position of a state's record in the trace store, as `TLCState.uid` holds it. The
/// checker chooses the encoding; the evaluator only hands it back through
/// [`crate::effects::MainChecker::trace_fingerprints`].
pub type TraceUid = u64;

/// One record of the chain `ConcurrentTLCTrace.getTrace` walks: a state's fingerprint and
/// the trace record it was written at.
pub type TraceStep = (crate::fp64::Fp, TraceUid);

impl State {
    /// `TLCState.isInitial()`: `level == INIT_LEVEL`.
    pub fn is_initial(&self) -> bool {
        self.level == INIT_LEVEL
    }
    pub fn uid(&self) -> Option<TraceUid> {
        self.uid
    }
    /// `Worker.writeState`: the state is now in the trace at `uid`.
    pub fn set_uid(&mut self, uid: TraceUid) {
        self.uid = Some(uid);
    }
    pub fn get(&self, slot: VarSlot) -> &Value {
        &self.values[slot.index()]
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    /// `TLCState.getLevel()`.
    pub fn level(&self) -> u32 {
        self.level
    }
    pub(crate) fn values(&self) -> &[Value] {
        &self.values
    }
    /// `new TLCStateMut(vals)`: the state a symmetry permutation produced, which
    /// `fingerPrint` evaluates the VIEW against (`TLCStateMut.java:265-268`). The private
    /// constructor leaves `level` at `INIT_LEVEL` (`TLCState.java:38`).
    pub(crate) fn of_values(values: Vec<Value>) -> State {
        State { values: values.into(), level: INIT_LEVEL, uid: None }
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot { values: SnapValues::Full(self.values.clone()), fun: None }
    }
}

/// The successor (or initial state) under construction. Generation binds a slot, recurses
/// into the continuation, then unbinds: exactly `TLCState.bind` / `unbind`.
pub struct PartialState {
    slots: Box<[Option<Value>]>,
    /// `TLCState.level`. `getNextStates` builds the successor as
    /// `TLCState.Empty.createEmpty().setPredecessor(state)`, so it is the parent's level plus
    /// one; the initial-state generator leaves it at `INIT_LEVEL`.
    level: u32,
}

impl PartialState {
    pub(crate) fn empty(vars: usize) -> Self {
        PartialState { slots: vec![None; vars].into_boxed_slice(), level: INIT_LEVEL }
    }
    /// `TLCState.Empty.createEmpty().setPredecessor(s0)` (`TLCState.java:144-153`).
    pub(crate) fn successor_of(vars: usize, s0: &State) -> Self {
        PartialState { slots: vec![None; vars].into_boxed_slice(), level: s0.level.saturating_add(1) }
    }
    pub(crate) fn bind(&mut self, slot: VarSlot, v: Value) {
        self.slots[slot.index()] = Some(v);
    }
    pub(crate) fn unbind(&mut self, slot: VarSlot) {
        self.slots[slot.index()] = None;
    }
    pub fn lookup(&self, slot: VarSlot) -> Option<&Value> {
        self.slots[slot.index()].as_ref()
    }
    pub fn all_assigned(&self) -> bool {
        self.slots.iter().all(Option::is_some)
    }
    /// `TLCStateMut.getUnassigned()`, in the order it prints them: sorted by variable name.
    pub(crate) fn unassigned(&self, vars: &VarTable, interner: &tlars_sany::Interner) -> Vec<String> {
        let mut names: Vec<String> =
            self.slots.iter().enumerate().filter(|(_, v)| v.is_none()).map(|(i, _)| interner.text(vars.names[i]).to_string()).collect();
        names.sort();
        names
    }
    /// `isGoodState` + copy: `None` if some variable is unassigned.
    pub(crate) fn freeze(&self) -> Option<State> {
        if !self.all_assigned() {
            return None;
        }
        Some(State { values: self.slots.iter().map(|v| v.clone().expect("all assigned")).collect(), level: self.level, uid: None })
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot { values: SnapValues::Slots(self.slots.clone()), fun: None }
    }
    /// `TLCState.copy()`.
    pub fn copy(&self) -> PartialState {
        PartialState { slots: self.slots.clone(), level: self.level }
    }
    pub(crate) fn from_values(values: &[Value]) -> PartialState {
        PartialState { slots: values.iter().map(|v| Some(v.clone())).collect(), level: INIT_LEVEL }
    }
    /// A complete state as the partial form the equality search compares.
    pub fn from_state(s: &State) -> PartialState {
        PartialState { slots: s.values().iter().map(|v| Some(v.clone())).collect(), level: s.level }
    }
    /// `copy()` for a caller outside this crate.
    pub fn copy_out(&self) -> PartialState {
        self.copy()
    }
}

/// `TLCStateFun`: persistent bindings for ENABLED (bind returns a new state).
///
/// Keyed by variable *name*, not by [`VarSlot`]: `TLCStateFun.bind` keeps the `SymbolNode`
/// `getVar` returned and `lookup` compares `var == cur.name.getName()`
/// (`TLCStateFun.java:47-59`). ENABLED's cutoff can reach a variable an instantiated module
/// declares, which has no slot in `vars[]` at all.
pub struct FunState {
    /// Cons list of `(name, value)`, newest first, as `TLCStateFun.bind` conses.
    cell: Option<Arc<FunCell>>,
}

struct FunCell {
    name: Atom,
    value: Value,
    next: Option<Arc<FunCell>>,
}

impl FunState {
    pub(crate) const EMPTY: FunState = FunState { cell: None };

    pub(crate) fn bind(&self, name: Atom, value: Value) -> FunState {
        FunState { cell: Some(Arc::new(FunCell { name, value, next: self.cell.clone() })) }
    }

    pub(crate) fn lookup(&self, name: Atom) -> Option<&Value> {
        let mut cur = self.cell.as_deref();
        while let Some(cell) = cur {
            if cell.name == name {
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
    /// A `TLCStateMut` indexes `values` by `name.getVarLoc()` and returns `null` for a name
    /// with no slot, while a `TLCStateFun` searches its cons list by name.
    pub(crate) fn get(&self, name: Atom, slot: Option<VarSlot>) -> Option<&Value> {
        match self {
            StateRef::Empty | StateRef::Null => None,
            StateRef::Full(s) => slot.map(|slot| s.get(slot)),
            StateRef::Partial(p) => slot.and_then(|slot| p.lookup(slot)),
            StateRef::Fun(f) => f.lookup(name),
        }
    }

    /// `s0.getLevel()`. `TLCState.Empty` and `TLCStateFun.Empty` are plain `TLCState`s whose
    /// level field still holds the value it starts at.
    pub(crate) fn level(&self) -> u32 {
        match self {
            StateRef::Full(s) => s.level(),
            StateRef::Partial(p) => p.level,
            StateRef::Empty | StateRef::Null | StateRef::Fun(_) => INIT_LEVEL,
        }
    }

    /// `TLCState.Null`, which `LazyValue.getValue` treats differently from `Empty`.
    pub(crate) fn is_null(&self) -> bool {
        matches!(self, StateRef::Null)
    }

    /// `s.copy()` as `LazyValue`'s cache keeps it; `None` for `Null`.
    pub(crate) fn snapshot(&self, vars: &VarTable) -> Option<Snapshot> {
        match self {
            StateRef::Null => None,
            StateRef::Empty => Some(Snapshot { values: SnapValues::Slots(Box::new([])), fun: None }),
            StateRef::Full(s) => Some(s.snapshot()),
            StateRef::Partial(p) => Some(p.snapshot()),
            // `TLCStateFun.copy()` returns `this` (`TLCStateFun.java:66-71`), so the cache
            // keeps the ENABLED state itself: both the bindings `isSubset` reads through
            // `vars[]` and the identity its fast path compares. ENABLED does cons cachable
            // thunks (`getLetContext`, `getOpContext(.., true)`), so this is reached.
            StateRef::Fun(f) => {
                let values = vars.names.iter().map(|&n| f.lookup(n).cloned()).collect();
                Some(Snapshot { values: SnapValues::Slots(values), fun: f.cell.clone() })
            }
        }
    }
}

/// Copy of the assigned part of a state, for `LazyValue` cache guards (`TLCState.isSubset`).
pub(crate) struct Snapshot {
    /// The value of each `vars[]` slot, as `isSubset` reads it out of the cached state.
    values: SnapValues,
    /// The cached `TLCStateFun`, for `isSubset`'s `s1 == s2` fast path.
    fun: Option<Arc<FunCell>>,
}

/// A complete state's values are immutable, so its copy shares them instead of copying
/// each one, which `TLCStateMut.copy()` would.
enum SnapValues {
    Slots(Box<[Option<Value>]>),
    Full(Arc<[Value]>),
}

impl SnapValues {
    fn len(&self) -> usize {
        match self {
            SnapValues::Slots(v) => v.len(),
            SnapValues::Full(v) => v.len(),
        }
    }
    fn get(&self, i: usize) -> Option<&Value> {
        match self {
            SnapValues::Slots(v) => v[i].as_ref(),
            SnapValues::Full(v) => Some(&v[i]),
        }
    }
}

impl Snapshot {
    /// `TLCState.isSubset(s1, s2)` (`TLCState.java:259-291`), with `self` as `s2`: every
    /// variable `self` has a value for has the same value in `s1`. An equality that throws
    /// makes the answer unknown, which is not a cache hit, but two states that *are* the same
    /// object answer yes before any value is compared. `equals` is `Value.equals`, false when
    /// it throws.
    pub(crate) fn covers(&self, vars: &VarTable, s1: &StateRef<'_>, mut equals: impl FnMut(&Value, &Value) -> bool) -> bool {
        if let (Some(cached), StateRef::Fun(f)) = (&self.fun, s1)
            && let Some(cell) = &f.cell
            && Arc::ptr_eq(cached, cell)
        {
            return true;
        }
        // Sharing the very values of `s1` makes every slot below the same object, which
        // answers yes without an `equals`; this only skips the loop.
        if let (SnapValues::Full(cached), StateRef::Full(s)) = (&self.values, s1)
            && Arc::ptr_eq(cached, &s.values)
        {
            return true;
        }
        for i in 0..self.values.len() {
            let Some(val2) = self.values.get(i) else { continue };
            let name = vars.names[i];
            // `Objects.equals(val1, val2)`, which answers yes for the same object without
            // calling `equals`. A cached state shares the values of every variable the step
            // left alone, so this is the common case.
            let equal = match s1.get(name, Some(VarSlot(i as u16))) {
                None => false,
                Some(val1) => val1.same_object(val2) || equals(val1, val2),
            };
            if !equal {
                return false;
            }
        }
        true
    }

    /// An empty snapshot stands for `TLCState.Empty`, which every state is a subset of.
    pub(crate) fn is_empty(&self) -> bool {
        self.values.len() == 0
    }
}
