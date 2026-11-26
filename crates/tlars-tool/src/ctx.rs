//! `tlc2.util.Context` (cons list of name → binding) and `LazyValue` (call-by-name thunk).
//!
//! `Ctx` is an `Arc` cons list because closures (`SetPredValue`, `FcnLambdaValue`,
//! `OpLambdaValue`) capture it and can travel to other workers inside a state.

use crate::error::EvalResult;
use crate::eval::{Ev, EvalControl};
use crate::state::StateRef;
use crate::value::Value;
use std::sync::{Arc, Mutex};
use tlars_sany::graph::{ExprId, OpDefId, ParamId, SymbolRef};

#[derive(Clone, Default)]
pub struct Ctx(Option<Arc<Cell>>);

struct Cell {
    name: CellName,
    binding: Binding,
    next: Ctx,
}

enum CellName {
    Sym(SymbolRef),
    /// `ToolGlobals.EXCEPT_AT`: the `@` of an EXCEPT component. TLC keys it on an `OpDefNode`
    /// it builds outside the semantic graph, so it has no `SymbolRef`.
    ExceptAt,
    /// `Context.branch`: ENABLED's cutoff marker (lookup with `cutoff` stops here).
    Branch,
}

/// A Java override: an index into `modules`' registry of native operators.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct NativeId(u16);

impl NativeId {
    pub(crate) const fn new(index: u16) -> NativeId {
        NativeId(index)
    }
    pub(crate) fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// What a context cell holds. Evaluation-only TLC values (`LazyValue`, `OpLambdaValue`,
/// `MethodValue`) live here and never inside a [`Value`], so a thunk cannot end up in a set
/// or a state.
#[derive(Clone)]
pub(crate) enum Binding {
    Value(Value),
    Thunk(Arc<Thunk>),
    /// Operator argument bound to a user definition (`OpArgNode`).
    OpDef(OpDefId),
    /// Operator argument bound to a Java override (`MethodValue`).
    Native(NativeId),
    /// Operator argument bound to a cfg `op(a, b) = v` constant (`OpRcdValue`).
    OpRcd(crate::value::OpRcd),
    #[expect(dead_code, reason = "LAMBDA operator arguments are not bound yet")]
    Lambda(Arc<OpLambda>),
    /// The `@` of an EXCEPT component.
    ExceptAt(Value),
}

impl Ctx {
    pub const EMPTY: Ctx = Ctx(None);

    pub(crate) fn cons(&self, name: SymbolRef, binding: Binding) -> Ctx {
        Ctx(Some(Arc::new(Cell { name: CellName::Sym(name), binding, next: self.clone() })))
    }
    /// `c.cons(EXCEPT_AT, atVal)`; an inner EXCEPT shadows an outer one, as the chain does.
    pub(crate) fn cons_except_at(&self, value: Value) -> Ctx {
        let binding = Binding::ExceptAt(value);
        Ctx(Some(Arc::new(Cell { name: CellName::ExceptAt, binding, next: self.clone() })))
    }
    /// `c.lookup(EXCEPT_AT)`.
    pub(crate) fn except_at(&self) -> Option<&Value> {
        let mut cur = self.0.as_deref();
        while let Some(cell) = cur {
            if let (CellName::ExceptAt, Binding::ExceptAt(v)) = (&cell.name, &cell.binding) {
                return Some(v);
            }
            cur = cell.next.0.as_deref();
        }
        None
    }
    /// `Context.branch(c)`: a nameless cell that `lookup(var, true)` stops at.
    pub(crate) fn branch(&self) -> Ctx {
        Ctx(Some(Arc::new(Cell { name: CellName::Branch, binding: Binding::Value(Value::Undef), next: self.clone() })))
    }
    /// `Context.isDeepEmpty()` (`Context.java:216-221`): the empty context, or the single
    /// `BaseBranch` cell. Java tests identity against `BaseBranch`, which is the one instance
    /// `Context.branch(Empty)` ever returns, so a branch cell directly over the empty context
    /// is exactly that instance. A branch over anything else is a fresh cell and not deeply
    /// empty, as is any cons.
    pub(crate) fn is_deep_empty(&self) -> bool {
        match &self.0 {
            None => true,
            Some(cell) => matches!(cell.name, CellName::Branch) && cell.next.0.is_none(),
        }
    }
    /// `Context.lookup(var)`: first cell whose name is `name`.
    pub(crate) fn lookup(&self, name: SymbolRef) -> Option<&Binding> {
        self.lookup_cutoff(name, false)
    }
    /// `Context.lookup(var, cutoff)`: with `cutoff`, the search stops at a branch cell.
    pub(crate) fn lookup_cutoff(&self, name: SymbolRef, cutoff: bool) -> Option<&Binding> {
        let mut cur = self.0.as_deref();
        while let Some(cell) = cur {
            match cell.name {
                CellName::Sym(s) if s == name => return Some(&cell.binding),
                CellName::Branch if cutoff => return None,
                _ => {}
            }
            cur = cell.next.0.as_deref();
        }
        None
    }
}

/// `LazyValue`: an expression + its context, evaluated on demand, cached under guards
/// (`LazyValue.java:164-204`). The cache may be *replaced*, so it is a `Mutex`, not a
/// `OnceLock`; racing workers each publish a whole entry, and an entry is only returned for
/// the states it was computed in.
pub(crate) struct Thunk {
    pub(crate) expr: ExprId,
    pub(crate) ctx: Ctx,
    pub(crate) cachable: bool,
    cache: Mutex<Option<Arc<ThunkCache>>>,
}

struct ThunkCache {
    value: Value,
    /// `None` for the state TLC copied as `null` (`TLCState.Null`), which `isSubset` treats
    /// as the set of all states.
    s0: Option<crate::state::Snapshot>,
    s1: Option<crate::state::Snapshot>,
    control: EvalControl,
}

impl Thunk {
    pub(crate) fn new(expr: ExprId, ctx: Ctx, cachable: bool) -> Arc<Thunk> {
        Arc::new(Thunk { expr, ctx, cachable, cache: Mutex::new(None) })
    }
    /// `getCachedValue` (`LazyValue.java:162-174`): hit only if s0 ⊆ cached s0, s1 ⊆ cached s1
    /// and the controls are semantically equivalent.
    pub(crate) fn cached(&self, ev: &mut Ev<'_>, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> Option<Value> {
        let cache = self.cache.lock().expect("thunk cache").clone()?;
        if !control.semantically_equivalent(cache.control) {
            return None;
        }
        (covers(ev, &cache.s0, &s0) && covers(ev, &cache.s1, &s1)).then(|| cache.value.clone())
    }
    /// `getValue`: cached, or evaluate in the captured context and publish.
    pub(crate) fn force(&self, ev: &mut Ev<'_>, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> EvalResult<Value> {
        if let Some(v) = self.cached(ev, s0, s1, control) {
            return Ok(v);
        }
        let value = ev.eval(self.expr, &self.ctx, s0, s1, control)?;
        if self.cachable {
            let (s0_copy, s1_copy) = { (s0.snapshot(ev.tool().vars()), s1.snapshot(ev.tool().vars())) };
            let entry = ThunkCache { value: value.clone(), s0: s0_copy, s1: s1_copy, control };
            *self.cache.lock().expect("thunk cache") = Some(Arc::new(entry));
        }
        Ok(value)
    }
}

/// `TLCState.isSubset(s, cached)`: a cached `null` or `Empty` state covers every state.
fn covers(ev: &mut Ev<'_>, cached: &Option<crate::state::Snapshot>, s: &StateRef<'_>) -> bool {
    match cached {
        None => true,
        Some(snap) => snap.is_empty() || snap.covers(ev.tool().vars(), s, |a, b| matches!(a.tlc_equals(ev, b), Ok(true))),
    }
}

/// `OpLambdaValue`: `LAMBDA x, y : e` passed as an operator argument.
#[expect(dead_code, reason = "LAMBDA operator arguments are not bound yet")]
pub(crate) struct OpLambda {
    pub(crate) params: Box<[ParamId]>,
    pub(crate) body: ExprId,
    pub(crate) ctx: Ctx,
}
