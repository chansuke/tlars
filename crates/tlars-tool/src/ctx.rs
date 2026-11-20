//! `tlc2.util.Context` (cons list of name → binding) and `LazyValue` (call-by-name thunk).
//!
//! `Ctx` is an `Arc` cons list because closures (`SetPredValue`, `FcnLambdaValue`,
//! `OpLambdaValue`) capture it and can travel to other workers inside a state.

use crate::eval::{EvalControl, Ev};
use crate::error::EvalResult;
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
    Native(crate::modules::NativeId),
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
    pub(crate) fn branch(&self) -> Ctx {
        todo!()
    }
    /// `Context.lookup(var)`: first cell whose name is `name`.
    pub(crate) fn lookup(&self, name: SymbolRef) -> Option<&Binding> {
        let mut cur = self.0.as_deref();
        while let Some(cell) = cur {
            if let CellName::Sym(s) = cell.name {
                if s == name {
                    return Some(&cell.binding);
                }
            }
            cur = cell.next.0.as_deref();
        }
        None
    }
    /// `Context.lookup(var, cutoff)`.
    pub(crate) fn lookup_cutoff(&self, name: SymbolRef, cutoff: bool) -> Option<&Binding> {
        todo!()
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
    s0: Option<crate::state::Snapshot>,
    s1: Option<crate::state::Snapshot>,
    control: EvalControl,
}

impl Thunk {
    pub(crate) fn new(expr: ExprId, ctx: Ctx, cachable: bool) -> Arc<Thunk> {
        Arc::new(Thunk { expr, ctx, cachable, cache: Mutex::new(None) })
    }
    /// `getCachedValue`: hit only if s0 ⊆ cached s0, s1 ⊆ cached s1 and controls are
    /// semantically equivalent (`EvalControl.semanticallyEquivalent`). Only evaluations
    /// without states are cached so far: the state snapshots come with M3.
    pub(crate) fn cached(&self, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> Option<Value> {
        if !matches!((s0, s1), (StateRef::Empty, StateRef::Empty)) {
            return None;
        }
        let cache = self.cache.lock().expect("thunk cache").clone()?;
        (cache.s0.is_none() && cache.s1.is_none() && control.semantically_equivalent(cache.control)).then(|| cache.value.clone())
    }
    /// `getValue`: cached, or evaluate in the captured context and publish.
    pub(crate) fn force(&self, ev: &mut Ev<'_>, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> EvalResult<Value> {
        if let Some(v) = self.cached(s0, s1, control) {
            return Ok(v);
        }
        let value = ev.eval(self.expr, &self.ctx, s0, s1, control)?;
        if self.cachable && matches!((s0, s1), (StateRef::Empty, StateRef::Empty)) {
            *self.cache.lock().expect("thunk cache") = Some(Arc::new(ThunkCache { value: value.clone(), s0: None, s1: None, control }));
        }
        Ok(value)
    }
}

/// `OpLambdaValue`: `LAMBDA x, y : e` passed as an operator argument.
pub(crate) struct OpLambda {
    pub(crate) params: Box<[ParamId]>,
    pub(crate) body: ExprId,
    pub(crate) ctx: Ctx,
}
