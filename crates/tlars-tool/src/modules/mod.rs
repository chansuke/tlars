//! Native standard-module operators (`tlc2.module.*`), one registry.
//!
//! Keyed like `TLARegistry` + `@TLAPlusOperator`: (module, TLA+ name, arity). Registry order
//! is `TLCBuiltInOverrides` order, so `..._OVERRIDE_LOADED` messages print in TLC's order.
//! Overrides replace bodies for *evaluation only*; SANY still level-checks the TLA+ text.
//! User `.class` overrides are not supported in v1 (see DESIGN.md).

mod sequences;

use crate::ctx::Ctx;
use crate::error::EvalResult;
use crate::eval::{EvalControl, Ev};
use crate::state::StateRef;
use crate::value::Value;
use tlars_sany::graph::ExprOrOpArg;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct NativeId(u16);

pub(crate) type StrictFn = fn(&mut Ev<'_>, &[Value]) -> EvalResult<Value>;
/// `@Evaluation`-style: receives unevaluated arguments (TLCGet, Assert, ...).
pub(crate) type LazyFn = fn(&mut Ev<'_>, &[ExprOrOpArg], &Ctx, StateRef<'_>, StateRef<'_>, EvalControl) -> EvalResult<Value>;

#[derive(Clone, Copy)]
pub(crate) enum NativeImpl {
    Strict(StrictFn),
    Lazy(LazyFn),
}

pub(crate) struct NativeOp {
    pub(crate) module: &'static str,
    pub(crate) tla_name: &'static str,
    pub(crate) arity: u8,
    pub(crate) imp: NativeImpl,
    /// `warn = false` / built-in: no OVERRIDE_LOADED message.
    pub(crate) silent: bool,
}

pub(crate) static REGISTRY: &[NativeOp] = &[
    NativeOp { module: "Sequences", tla_name: "Append", arity: 2, imp: NativeImpl::Strict(sequences::append), silent: true },
    NativeOp { module: "Sequences", tla_name: "Len", arity: 1, imp: NativeImpl::Strict(sequences::len), silent: true },
    // Naturals, Integers, FiniteSets, Bags, TLC, TLCGetSet, TLCExt, Randomization,
    // TransitiveClosure, Strings, Json ...
];
