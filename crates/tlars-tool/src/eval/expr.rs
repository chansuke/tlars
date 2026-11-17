//! `eval*` / `evalAppl*` / `lookup` / `getOpContext` / `contexts`.
//!
//! `lookup` checks the `Ctx` chain first, then the spec's `Bindings` (TLC: context before
//! tool objects), so INSTANCE substitutions shadow the instantiated module's declarations.
//! Operator arguments and LET definitions become `Binding::Thunk` (call-by-name).

use super::{EvalControl, Ev};
use crate::ctx::{Binding, Ctx};
use crate::error::{EvalError, EvalResult};
use crate::state::StateRef;
use crate::value::Value;
use tlars_sany::graph::{BuiltinOp, ExprId, SymbolRef};

/// What a name resolved to (`SymbolNodeValueLookupProvider.lookup`).
pub(crate) enum Resolved {
    Binding(Binding),
    Value(Value),
    UserOp(tlars_sany::graph::OpDefId),
    Native(crate::modules::NativeId),
    Var(crate::state::VarSlot),
}

impl<'t> Ev<'t> {
    /// `Tool.eval(expr, c, s0, s1, control)`.
    pub fn eval(&mut self, expr: ExprId, c: &Ctx, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> EvalResult<Value> {
        self.enter(expr)?;
        let r = self.eval_impl(expr, c, s0, s1, control);
        self.leave();
        r
    }

    fn eval_impl(&mut self, expr: ExprId, c: &Ctx, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> EvalResult<Value> {
        // TODO: OpAppl -> eval_appl; LetIn -> cons cachable Thunks for 0-arity defs;
        // SubstIn -> cons Thunks (cachable = true); Label -> body; Numeral/String -> the
        // processConstants value; At -> EXCEPT_AT lookup.
        todo!()
    }

    pub(crate) fn lookup(&mut self, sym: SymbolRef, c: &Ctx, s0: StateRef<'_>, cutoff: bool) -> Resolved {
        todo!()
    }

    /// Builtin opcode of an applied operator, after `lookup` (an INSTANCE or cfg override may
    /// map a user name onto a builtin). TLC: `BuiltInOPs.getOpCode(opDef.getName())`.
    pub(crate) fn opcode(&mut self, op: SymbolRef, c: &Ctx) -> Option<BuiltinOp> {
        todo!()
    }

    /// `getOpContext`: bind params to call-by-name thunks over `c`.
    pub(crate) fn op_context(&mut self, def: tlars_sany::graph::OpDefId, args: &[tlars_sany::graph::ExprOrOpArg], c: &Ctx, cachable: bool) -> Ctx {
        todo!()
    }

    pub(crate) fn not_boolean_next(&mut self, pred: ExprId, c: &Ctx) -> EvalError {
        todo!("'The next state relation is not a boolean expression.' with location")
    }
}
