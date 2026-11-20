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
use crate::spec::OpBinding;
use tlars_sany::graph::{BuiltinOp, ExprId, ExprNode, OpDefKind, SymbolRef};

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
        let graph = &self.tool().graph;
        match graph.expr(expr) {
            ExprNode::OpAppl { op, args, bound, .. } => {
                if let SymbolRef::OpDef(id) = *op {
                    if args.is_empty() && bound.is_none() {
                        match self.tool().bindings.op_def(id) {
                            Some(OpBinding::Folded(v)) => return Ok(v.clone()),
                            None => {
                                let def = graph.op_def(id);
                                if def.kind == OpDefKind::User && def.params.is_empty() {
                                    let body = def.body.expect("a user definition has a body");
                                    return self.eval(body, c, s0, s1, control);
                                }
                            }
                            Some(_) => {}
                        }
                    }
                }
                Err(EvalError::unimplemented(format!("evalAppl of {} (M2 unit 4)", graph.loc(expr).tlc_text(graph.interner()))))
            }
            ExprNode::Label { body, .. } => self.eval(*body, c, s0, s1, control),
            ExprNode::Numeral { .. } | ExprNode::String { .. } => {
                Ok(self.tool().bindings.literal(expr).expect("processConstants bound every literal").clone())
            }
            ExprNode::LetIn { .. } | ExprNode::SubstIn { .. } | ExprNode::APSubstIn { .. } | ExprNode::At { .. } => {
                Err(EvalError::unimplemented(format!("eval of {} (M2 unit 4)", graph.loc(expr).tlc_text(graph.interner()))))
            }
            ExprNode::Decimal { .. } => unreachable!("processConstants rejects decimals"),
        }
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
