//! `getInitStates*`. Same continuation-passing shape as `next`, binding unprimed variables
//! in one `PartialState`: `x = e` binds or tests, `x \in S` enumerates `S.elements(Native)`,
//! `x \subseteq S` enumerates SUBSET S, `\E` iterates normalised contexts, IF/CASE take one
//! branch, SubstIn pushes thunks, and once every variable is bound the rest are filters.

use super::expr::{Builtin, Resolved};
use super::next::{ActItem, ActKind, Acts};
use super::{EvalControl, Ev, InitSink};
use crate::ctx::{Binding, Ctx};
use crate::error::{EvalError, Flow};
use crate::state::{PartialState, StateRef};
use crate::value::Value;
use tlars_msg::Ec;
use tlars_sany::graph::{ExprId, ExprNode, ExprOrOpArg};

impl<'t> Ev<'t> {
    /// `Tool.getInitStates(IStateFunctor)`: the init predicates form one conjunction. Items
    /// are consed tail-to-head so evaluation order is init[0], init[1], ... (TLC fixed an
    /// A, D, C, B ordering bug here in 2018; porting the loop literally inherits the fix).
    pub fn init_states(&mut self, sink: &mut dyn InitSink) -> Flow {
        let tool = self.tool;
        let inits = tool.init_actions();
        let Some((first, rest)) = inits.split_first() else { return Ok(()) };
        let mut ps = PartialState::empty(tool.vars().decls.len());
        self.push_init_preds(rest, Acts::Empty, &mut |ev, acts| ev.init_impl(first.pred, acts, &first.ctx, &mut ps, sink))
    }

    fn push_init_preds(&mut self, rest: &[crate::actions::Action], acts: Acts<'_>, k: &mut dyn FnMut(&mut Self, Acts<'_>) -> Flow) -> Flow {
        match rest.split_last() {
            None => k(self, acts),
            Some((last, init)) => {
                let item = ActItem { pred: last.pred, ctx: &last.ctx, kind: ActKind::Pred, next: acts };
                self.push_init_preds(init, Acts::Cons(&item), k)
            }
        }
    }

    /// `getInitStates(SemanticNode, ActionItemList, Context, TLCState, IStateFunctor)`.
    pub(crate) fn init_impl(&mut self, pred: ExprId, acts: Acts<'_>, c: &Ctx, ps: &mut PartialState, sink: &mut dyn InitSink) -> Flow {
        self.enter_generator(pred);
        let r = self.init_impl_inner(pred, acts, c, ps, sink);
        self.leave_generator(r.as_ref().err());
        r
    }

    fn init_impl_inner(&mut self, pred: ExprId, acts: Acts<'_>, c: &Ctx, ps: &mut PartialState, sink: &mut dyn InitSink) -> Flow {
        let graph = self.tool.graph.clone();
        match graph.expr(pred) {
            ExprNode::OpAppl { .. } => self.init_appl(pred, acts, c, ps, sink),
            ExprNode::LetIn { body, .. } | ExprNode::Label { body, .. } => self.init_impl(*body, acts, c, ps, sink),
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                let (substs, body) = (substs.clone(), *body);
                let c1 = self.subst_context(&substs, c);
                self.init_impl(body, acts, &c1, ps, sink)
            }
            _ => {
                let reason = format!("The init state relation is not a boolean expression.\n{}", graph.expr_tlc_text(pred));
                Err(EvalError::detailed(reason, Some(graph.loc(pred))).into())
            }
        }
    }

    /// `getInitStatesAppl` (`Tool.java:570-884`).
    fn init_appl(&mut self, pred: ExprId, acts: Acts<'_>, c: &Ctx, ps: &mut PartialState, sink: &mut dyn InitSink) -> Flow {
        let graph = self.tool.graph.clone();
        let ExprNode::OpAppl { op, args, .. } = graph.expr(pred) else { unreachable!() };
        let (op, args) = (*op, args.clone());
        let mut code = self.opcode(op);
        if code.is_none() {
            match self.lookup(op, c, StateRef::Partial(ps), false) {
                Resolved::UserOp(d) | Resolved::Binding(Binding::OpDef(d)) => {
                    code = self.opcode(tlars_sany::graph::SymbolRef::OpDef(d));
                    if code.is_none() {
                        let body = graph.op_def(d).body.expect("a user definition has a body");
                        let c1 = self.op_context(d, &args, c, true);
                        return self.init_impl(body, acts, &c1, ps, sink);
                    }
                }
                Resolved::Binding(Binding::Thunk(t)) => {
                    match t.cached(self, StateRef::Partial(ps), StateRef::Null, EvalControl::CLEAR) {
                        None => {
                            let (e, tc) = (t.expr, t.ctx.clone());
                            return self.init_impl(e, acts, &tc, ps, sink);
                        }
                        Some(v) => return self.init_filter(pred, acts, ps, sink, v),
                    }
                }
                other => {
                    let v = self.apply_resolved(other, op, pred, &args, c, StateRef::Partial(ps), StateRef::Empty, EvalControl::INIT)?;
                    return self.init_filter(pred, acts, ps, sink, v);
                }
            }
        }
        let Some(code) = code else { unreachable!("resolved above") };
        let arg = |i: usize| args[i].expect_expr();
        let eval = |ev: &mut Self, e: ExprId, c: &Ctx, ps: &PartialState| {
            ev.eval(e, c, StateRef::Partial(ps), StateRef::Empty, EvalControl::INIT)
        };
        match code {
            Builtin::DisjList | Builtin::Lor => {
                for a in args.iter() {
                    self.init_impl(a.expect_expr(), acts, c, ps, sink)?;
                }
                Ok(())
            }
            Builtin::ConjList | Builtin::Land => {
                let Some((first, rest)) = args.split_first() else {
                    return self.init_continue(acts, ps, sink);
                };
                let first = first.expect_expr();
                self.push_init_conjuncts(rest, c, acts, &mut |ev, acts1| ev.init_impl(first, acts1, c, ps, sink))
            }
            Builtin::BoundedExists => {
                let ExprNode::OpAppl { bound, .. } = graph.expr(pred) else { unreachable!() };
                let bound = bound.as_deref().expect("a bounded quantifier has bounds");
                let body = arg(0);
                let mut contexts = self.contexts(bound, c, StateRef::Partial(ps), StateRef::Empty, EvalControl::INIT)?;
                while let Some(c1) = contexts.next(self)? {
                    self.init_impl(body, acts, &c1, ps, sink)?;
                }
                Ok(())
            }
            Builtin::BoundedForall => {
                let ExprNode::OpAppl { bound, .. } = graph.expr(pred) else { unreachable!() };
                let bound = bound.as_deref().expect("a bounded quantifier has bounds");
                let body = arg(0);
                let mut contexts = self.contexts(bound, c, StateRef::Partial(ps), StateRef::Empty, EvalControl::INIT)?;
                let Some(first) = contexts.next(self)? else {
                    return self.init_continue(acts, ps, sink);
                };
                let mut rest = Vec::new();
                while let Some(c2) = contexts.next(self)? {
                    rest.push(c2);
                }
                self.push_init_forall(body, &rest, acts, ps, sink, &first)
            }
            Builtin::IfThenElse => {
                let v = eval(self, arg(0), c, ps)?;
                match v {
                    Value::Bool(b) => self.init_impl(if b { arg(1) } else { arg(2) }, acts, c, ps, sink),
                    _ => {
                        let reason = format!(
                            "In computing initial states, a non-boolean expression ({}) was used as the condition of an IF.\n{}",
                            v.kind_string(),
                            graph.expr_tlc_text(pred)
                        );
                        Err(EvalError::detailed(reason, Some(graph.loc(pred))).into())
                    }
                }
            }
            Builtin::Case => {
                let mut other = None;
                for a_i in args.iter() {
                    let pair = a_i.expect_expr();
                    let ExprNode::OpAppl { args: pair_args, .. } = graph.expr(pair) else { unreachable!("CASE arms are $Pair") };
                    let pair_args = pair_args.clone();
                    match pair_args[0] {
                        ExprOrOpArg::Absent => other = Some(pair_args[1].expect_expr()),
                        guard => {
                            let v = eval(self, guard.expect_expr(), c, ps)?;
                            match v {
                                Value::Bool(true) => return self.init_impl(pair_args[1].expect_expr(), acts, c, ps, sink),
                                Value::Bool(false) => {}
                                _ => {
                                    let arm = pair_args[1].expect_expr();
                                    let reason = format!(
                                        "In computing initial states, a non-boolean expression ({}) was used as a guard condition of a CASE.\n{}",
                                        v.kind_string(),
                                        graph.expr_tlc_text(arm)
                                    );
                                    return Err(EvalError::detailed(reason, Some(graph.loc(arm))).into());
                                }
                            }
                        }
                    }
                }
                match other {
                    Some(e) => self.init_impl(e, acts, c, ps, sink),
                    None => {
                        let reason = format!(
                            "In computing initial states, TLC encountered a CASE with no conditions true.\n{}",
                            graph.expr_tlc_text(pred)
                        );
                        Err(EvalError::detailed(reason, Some(graph.loc(pred))).into())
                    }
                }
            }
            Builtin::FcnApply => {
                let fval = eval(self, arg(0), c, ps)?;
                let fval = match &fval {
                    Value::FcnLambda(l) if l.converted.get().is_none() => {
                        let c1 = self.fcn_context(l, &args, c, StateRef::Partial(ps), StateRef::Empty, EvalControl::INIT)?;
                        let body = l.body;
                        return self.init_impl(body, acts, &c1, ps, sink);
                    }
                    Value::FcnLambda(l) => Value::FcnRcd(l.converted.get().expect("just checked").clone()),
                    _ => fval.clone(),
                };
                if !fval.is_function() {
                    let reason = format!(
                        "In computing initial states, a non-function ({}) was applied as a function.\n{}",
                        fval.kind_string(),
                        graph.expr_tlc_text(pred)
                    );
                    return Err(EvalError::detailed(reason, Some(graph.loc(pred))).into());
                }
                let argv = eval(self, arg(1), c, ps)?;
                let bval = fval.apply(self, &argv, EvalControl::INIT)?;
                let Value::Bool(b) = bval else {
                    let message = self.out.text(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING2, &["initial states", "boolean", &graph.expr_tlc_text(pred)]);
                    return Err(EvalError {
                        loc: Some(graph.loc(arg(1))),
                        ..EvalError::runtime(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING2, message)
                    }
                    .into());
                };
                if b { self.init_continue(acts, ps, sink) } else { Ok(()) }
            }
            Builtin::Eq => self.init_assign(pred, &args, acts, c, ps, sink, Assign::Eq),
            Builtin::In => self.init_assign(pred, &args, acts, c, ps, sink, Assign::In),
            Builtin::Subseteq => self.init_assign(pred, &args, acts, c, ps, sink, Assign::Subseteq),
            Builtin::Implies => {
                let v = eval(self, arg(0), c, ps)?;
                match v {
                    Value::Bool(true) => self.init_impl(arg(1), acts, c, ps, sink),
                    Value::Bool(false) => self.init_continue(acts, ps, sink),
                    _ => {
                        let reason = format!(
                            "In computing initial states of a predicate of form P => Q, P was {}\n.{}",
                            v.kind_string(),
                            graph.expr_tlc_text(pred)
                        );
                        Err(EvalError::detailed(reason, Some(graph.loc(pred))).into())
                    }
                }
            }
            Builtin::Nop => self.init_impl(arg(0), acts, c, ps, sink),
            // Every other built-in operator is evaluated as a boolean filter.
            _ => {
                let v = eval(self, pred, c, ps)?;
                let Value::Bool(b) = v else {
                    let shown = v.to_tlc_string(self)?;
                    let reason = format!(
                        "In computing initial states, TLC expected a boolean expression,\nbut instead found {shown}.\n{}",
                        graph.expr_tlc_text(pred)
                    );
                    return Err(EvalError::detailed(reason, Some(graph.loc(pred))).into());
                };
                if b { self.init_continue(acts, ps, sink) } else { Ok(()) }
            }
        }
    }

    /// The `opcode == 0` tail of `getInitStatesAppl`: the value must be a boolean.
    fn init_filter(&mut self, pred: ExprId, acts: Acts<'_>, ps: &mut PartialState, sink: &mut dyn InitSink, v: Value) -> Flow {
        let Value::Bool(b) = v else {
            let graph = self.tool.graph.clone();
            let shown = v.to_tlc_string(self)?;
            let message =
                self.out.text(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, &["initial states", "boolean", &shown, &graph.expr_tlc_text(pred)]);
            return Err(EvalError {
                loc: Some(graph.loc(pred)),
                ..EvalError::runtime(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, message)
            }
            .into());
        };
        if b { self.init_continue(acts, ps, sink) } else { Ok(()) }
    }

    /// `x = e`, `x \in S` and `x \subseteq S`: bind an unassigned variable, else test it.
    #[allow(clippy::too_many_arguments)]
    fn init_assign(
        &mut self,
        pred: ExprId,
        args: &[ExprOrOpArg],
        acts: Acts<'_>,
        c: &Ctx,
        ps: &mut PartialState,
        sink: &mut dyn InitSink,
        kind: Assign,
    ) -> Flow {
        let graph = self.tool.graph.clone();
        let Some(slot) = self.get_var(args[0].expect_expr(), c, false) else {
            // Not an assignment: TLC casts the value to a BoolValue without checking.
            let v = self.eval(pred, c, StateRef::Partial(ps), StateRef::Empty, EvalControl::INIT)?;
            let Value::Bool(b) = v else { return Err(EvalError::class_cast(v.java_class(), "BoolValue").into()) };
            if !b {
                return Ok(());
            }
            return self.init_continue(acts, ps, sink);
        };
        let rval = self.eval(args[1].expect_expr(), c, StateRef::Partial(ps), StateRef::Empty, EvalControl::INIT)?;
        let rval = if kind == Assign::Subseteq { crate::value::subset(rval) } else { rval };
        let bound = ps.lookup(slot).cloned();
        match (kind, bound) {
            (Assign::Eq, None) => {
                ps.bind(slot, rval);
                let r = self.init_continue(acts, ps, sink);
                ps.unbind(slot);
                r
            }
            (Assign::Eq, Some(lval)) => {
                if lval.tlc_equals(self, &rval)? { self.init_continue(acts, ps, sink) } else { Ok(()) }
            }
            (_, None) => {
                if !rval.is_enumerable() {
                    let reason = format!(
                        "In computing initial states, the right side of \\IN is not enumerable.\n{}",
                        graph.expr_tlc_text(pred)
                    );
                    return Err(EvalError::detailed(reason, Some(graph.loc(pred))).into());
                }
                let mut elems = rval.elements(self, crate::value::enumerate::EnumOrder::Native)?.expect("an enumerable value enumerates");
                while let Some(elem) = elems.next(self)? {
                    ps.bind(slot, elem);
                    let r = self.init_continue(acts, ps, sink);
                    ps.unbind(slot);
                    r?;
                }
                Ok(())
            }
            (_, Some(lval)) => {
                if rval.member(self, &lval)? { self.init_continue(acts, ps, sink) } else { Ok(()) }
            }
        }
    }

    fn push_init_conjuncts(
        &mut self,
        rest: &[ExprOrOpArg],
        c: &Ctx,
        acts: Acts<'_>,
        k: &mut dyn FnMut(&mut Self, Acts<'_>) -> Flow,
    ) -> Flow {
        match rest.split_last() {
            None => k(self, acts),
            Some((last, init)) => {
                let item = ActItem { pred: last.expect_expr(), ctx: c, kind: ActKind::Conj(rest.len() as u32), next: acts };
                self.push_init_conjuncts(init, c, Acts::Cons(&item), k)
            }
        }
    }

    fn push_init_forall(
        &mut self,
        body: ExprId,
        rest: &[Ctx],
        acts: Acts<'_>,
        ps: &mut PartialState,
        sink: &mut dyn InitSink,
        first: &Ctx,
    ) -> Flow {
        match rest.split_first() {
            None => self.init_impl(body, acts, first, ps, sink),
            Some((next, tail)) => {
                let item = ActItem { pred: body, ctx: next, kind: ActKind::Pred, next: acts };
                self.push_init_forall(body, tail, Acts::Cons(&item), ps, sink, first)
            }
        }
    }

    /// `getInitStates(ActionItemList, TLCState, IStateFunctor)`: pop or emit.
    pub(crate) fn init_continue(&mut self, acts: Acts<'_>, ps: &mut PartialState, sink: &mut dyn InitSink) -> Flow {
        let Acts::Cons(item) = acts else {
            return sink.add_init(self, ps);
        };
        // `getInitStates`' all-assigned fast path: the remaining items are boolean tests.
        if ps.all_assigned() {
            let mut acts = acts;
            while let Acts::Cons(item) = acts {
                let v = self.eval(item.pred, item.ctx, StateRef::Partial(ps), StateRef::Empty, EvalControl::INIT)?;
                let Value::Bool(b) = v else {
                    let graph = self.tool.graph.clone();
                    let shown = v.to_tlc_string(self)?;
                    let text = graph.expr_tlc_text(item.pred);
                    let message = self.out.text(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, &["initial states", "boolean", &shown, &text]);
                    return Err(EvalError {
                        loc: Some(graph.loc(item.pred)),
                        ..EvalError::runtime(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, message)
                    }
                    .into());
                };
                if !b {
                    return Ok(());
                }
                acts = item.next;
            }
            return sink.add_init(self, ps);
        }
        self.init_impl(item.pred, item.next, item.ctx, ps, sink)
    }
}

/// Which assignment form `init_assign` handles.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Assign {
    Eq,
    In,
    Subseteq,
}
