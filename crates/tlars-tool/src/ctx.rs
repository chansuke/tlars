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
    Lambda(Arc<OpLambda>),
    /// The `@` of an EXCEPT component.
    ExceptAt(Value),
}

impl Ctx {
    pub const EMPTY: Ctx = Ctx(None);

    pub(crate) fn cons(&self, name: SymbolRef, binding: Binding) -> Ctx {
        Ctx(Some(Arc::new(Cell { name: CellName::Sym(name), binding, next: self.clone() })))
    }
    pub(crate) fn branch(&self) -> Ctx {
        todo!()
    }
    /// `Context.lookup(var)`: first cell whose name is `name`.
    pub(crate) fn lookup(&self, name: SymbolRef) -> Option<&Binding> {
        todo!()
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
    /// semantically equivalent (`EvalControl.semanticallyEquivalent`).
    pub(crate) fn cached(&self, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> Option<Value> {
        todo!()
    }
    /// `getValue`: cached, or evaluate in the captured context and publish.
    pub(crate) fn force(&self, ev: &mut Ev<'_>, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> EvalResult<Value> {
        todo!()
    }
}

/// `OpLambdaValue`: `LAMBDA x, y : e` passed as an operator argument.
pub(crate) struct OpLambda {
    pub(crate) params: Box<[ParamId]>,
    pub(crate) body: ExprId,
    pub(crate) ctx: Ctx,
}
