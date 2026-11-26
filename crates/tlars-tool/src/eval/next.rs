//! `getNextStates*`: continuation-passing over an action-item list.
//!
//! `ActionItemList` becomes [`Acts`], a cons list *borrowed from the Rust stack*: each
//! conjunct pushes one `ActItem` local and recurses, so generating a successor allocates no
//! list cells, and the continuation's lifetime is checked by the compiler.
//! The successor is built in one `&mut PartialState` with bind/unbind, as in TLC.

use super::expr::{Builtin, Resolved};
use super::{EvalControl, Ev, NextSink};
use crate::actions::Action;
use crate::ctx::{Binding, Ctx};
use crate::error::{EvalError, Flow};
use crate::fp64::Fp;
use crate::state::{PartialState, State, StateRef};
use crate::value::Value;
use tlars_msg::Ec;
use tlars_sany::graph::{ExprId, ExprNode, ExprOrOpArg};

/// `ActionItemList`. `Empty` = all conjuncts done: hand the successor to the sink.
#[derive(Clone, Copy)]
pub(crate) enum Acts<'a> {
    Empty,
    Cons(&'a ActItem<'a>),
}

pub(crate) struct ActItem<'a> {
    pub(crate) pred: ExprId,
    pub(crate) ctx: &'a Ctx,
    pub(crate) kind: ActKind,
    pub(crate) next: Acts<'a>,
}

/// `ActionItemList.kind`: `> 0` conjunct index, `-1` predicate, `-2` UNCHANGED,
/// `-3` `pred' # pred` (from `<A>_v`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ActKind {
    Conj(u32),
    Pred,
    Unchanged,
    Changed,
}

impl<'t> Ev<'t> {
    /// `Tool.getNextStates(functor, state)`: every action in split order. A `Halt` from the
    /// sink (TLC: an exception out of `addElement`) stops the loop. `s0_fp` is the parent's
    /// fingerprint: it seeds RandomElement for this expansion (`fp ^ seed`, once per parent).
    pub fn next_states(&mut self, s0: &State, s0_fp: Fp, sink: &mut dyn NextSink) -> Flow {
        self.begin_expansion(s0, s0_fp);
        let tool = self.tool;
        for action in tool.actions() {
            self.next_states_of_action(s0, action, sink)?;
        }
        Ok(())
    }

    /// One action only (simulation, DFID, trace replay).
    pub fn next_states_of(&mut self, s0: &State, action: crate::actions::ActionId, sink: &mut dyn NextSink) -> Flow {
        let tool = self.tool;
        self.next_states_of_action(s0, tool.action(action), sink)
    }

    fn next_states_of_action(&mut self, s0: &State, action: &Action, sink: &mut dyn NextSink) -> Flow {
        let mut s1 = PartialState::successor_of(s0.len(), s0);
        self.next_impl(action, action.pred, Acts::Empty, &action.ctx, s0, &mut s1, sink)
    }

    /// `getNextStatesImpl`: dispatch on node kind.
    pub(crate) fn next_impl(
        &mut self,
        a: &Action,
        pred: ExprId,
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
    ) -> Flow {
        self.enter_generator(pred)?;
        let r = self.next_impl_inner(a, pred, acts, c, s0, s1, sink);
        self.leave_generator(r.as_ref().err());
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn next_impl_inner(
        &mut self,
        a: &Action,
        pred: ExprId,
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
    ) -> Flow {
        let tool = self.tool;
        match tool.graph().expr(pred) {
            ExprNode::OpAppl { .. } => self.next_appl(a, pred, acts, c, s0, s1, sink),
            ExprNode::LetIn { body, .. } | ExprNode::Label { body, .. } => self.next_impl(a, *body, acts, c, s0, s1, sink),
            ExprNode::SubstIn { .. } | ExprNode::APSubstIn { .. } => self.next_subst_in(a, pred, acts, c, s0, s1, sink),
            _ => Err(self.not_boolean_next(pred, c).into()),
        }
    }

    /// `getNextStatesApplImpl` (`Tool.java:1100-1548`).
    fn next_appl(
        &mut self,
        a: &Action,
        pred: ExprId,
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
    ) -> Flow {
        let graph = &self.tool().graph;
        let ExprNode::OpAppl { op, args, .. } = graph.expr(pred) else { unreachable!() };
        let (op, args) = (*op, &**args);
        let mut code = self.opcode(op);
        if code.is_none() {
            // A user-defined operator, unless a cfg override or INSTANCE put a built-in here.
            match self.lookup(op, c, StateRef::Full(s0), false) {
                Resolved::UserOp(d) | Resolved::Binding(Binding::OpDef(d)) => {
                    code = self.opcode(tlars_sany::graph::SymbolRef::OpDef(d));
                    if code.is_none() {
                        let body = graph.op_def(d).body.expect("a user definition has a body");
                        let c1 = self.op_context(d, args, c, true);
                        return self.next_impl(a, body, acts, &c1, s0, s1, sink);
                    }
                }
                // `Tool.java:1129-1132`.
                Resolved::Thm(t) => {
                    let body = graph.thm(t).body;
                    let c1 = self.thm_context(t, args, c);
                    return self.next_impl(a, body, acts, &c1, s0, s1, sink);
                }
                Resolved::Binding(Binding::Thunk(t)) => {
                    match t.cached(self, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR) {
                        None => {
                            let (e, tc) = (t.expr, t.ctx.clone());
                            return self.next_impl(a, e, acts, &tc, s0, s1, sink);
                        }
                        Some(v) => return self.next_filter(a, pred, acts, s0, s1, sink, v),
                    }
                }
                other => {
                    // `getNextStatesApplEvalAppl`: an `OpValue` is applied to the arguments,
                    // any other value is the result.
                    let v = self.apply_resolved(other, op, pred, args, c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                    return self.next_filter(a, pred, acts, s0, s1, sink, v);
                }
            }
        }
        let Some(code) = code else { unreachable!("resolved above") };
        let arg = |i: usize| args[i].expect_expr();
        match code {
            Builtin::ConjList | Builtin::Land => self.next_conj(a, args, acts, c, s0, s1, sink),
            Builtin::DisjList | Builtin::Lor => {
                for arg in args.iter() {
                    self.next_impl(a, arg.expect_expr(), acts, c, s0, s1, sink)?;
                }
                Ok(())
            }
            Builtin::BoundedExists => {
                let ExprNode::OpAppl { bound, .. } = graph.expr(pred) else { unreachable!() };
                let bound = bound.as_deref().expect("a bounded quantifier has bounds");
                let body = arg(0);
                let mut contexts = self.contexts(bound, c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                while let Some(c1) = contexts.next(self)? {
                    self.next_impl(a, body, acts, &c1, s0, s1, sink)?;
                }
                Ok(())
            }
            // The first binding is evaluated, the rest become predicate items.
            Builtin::BoundedForall => {
                let ExprNode::OpAppl { bound, .. } = graph.expr(pred) else { unreachable!() };
                let bound = bound.as_deref().expect("a bounded quantifier has bounds");
                let body = arg(0);
                let mut contexts = self.contexts(bound, c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                let Some(first) = contexts.next(self)? else {
                    return self.next_continue(a, acts, s0, s1, sink);
                };
                let mut rest = Vec::new();
                while let Some(c2) = contexts.next(self)? {
                    rest.push(c2);
                }
                self.push_forall(a, body, &rest, acts, s0, s1, sink, &first)
            }
            Builtin::IfThenElse => {
                let v = self.eval(arg(0), c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                match v {
                    Value::Bool(b) => self.next_impl(a, if b { arg(1) } else { arg(2) }, acts, c, s0, s1, sink),
                    _ => {
                        let reason = format!(
                            "In computing next states, a non-boolean expression ({}) was used as the condition of an IF.{}",
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
                            let guard = guard.expect_expr();
                            let v = self.eval(guard, c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                            match v {
                                Value::Bool(true) => return self.next_impl(a, pair_args[1].expect_expr(), acts, c, s0, s1, sink),
                                Value::Bool(false) => {}
                                _ => {
                                    let arm = pair_args[1].expect_expr();
                                    let reason = format!(
                                        "In computing next states, a non-boolean expression ({}) was used as a guard condition of a CASE.\n{}",
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
                    Some(e) => self.next_impl(a, e, acts, c, s0, s1, sink),
                    None => {
                        let reason = format!(
                            "In computing next states, TLC encountered a CASE with no conditions true.\n{}",
                            graph.expr_tlc_text(pred)
                        );
                        Err(EvalError::detailed(reason, Some(graph.loc(pred))).into())
                    }
                }
            }
            // `f[x]` on a lazy function expands the body at `x`.
            Builtin::FcnApply => {
                let fval = self.eval(arg(0), c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::KEEP_LAZY)?;
                let fval = match &fval {
                    Value::FcnLambda(l) if l.converted.get().is_none() => {
                        let c1 = self.fcn_context(l, args, c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                        let body = l.body;
                        return self.next_impl(a, body, acts, &c1, s0, s1, sink);
                    }
                    Value::FcnLambda(l) => Value::FcnRcd(l.converted.get().expect("just checked").clone()),
                    _ => fval.clone(),
                };
                if !fval.is_function() {
                    let reason = format!(
                        "In computing next states, a non-function ({}) was applied as a function.\n{}",
                        fval.kind_string(),
                        graph.expr_tlc_text(pred)
                    );
                    return Err(EvalError::detailed(reason, Some(graph.loc(pred))).into());
                }
                let argv = self.eval(arg(1), c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                let bval = fval.apply(self, &argv, EvalControl::CLEAR)?;
                let Value::Bool(b) = bval else {
                    let message = self.out.text(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING2, &["next states", "boolean", &graph.expr_tlc_text(pred)]);
                    return Err(EvalError::runtime(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING2, message).with_loc(Some(graph.loc(arg(1)))).into());
                };
                if b { self.next_continue(a, acts, s0, s1, sink) } else { Ok(()) }
            }
            Builtin::AngleAct => {
                let item = ActItem { pred: args[1].expect_expr(), ctx: c, kind: ActKind::Changed, next: acts };
                self.next_impl(a, arg(0), Acts::Cons(&item), c, s0, s1, sink)
            }
            // `[A]_e`: A's successors, then the stuttering step.
            Builtin::SquareAct => {
                self.next_impl(a, arg(0), acts, c, s0, s1, sink)?;
                self.process_unchanged(a, args[1].expect_expr(), acts, c, s0, s1, sink)
            }
            Builtin::Eq => self.next_assign(a, pred, args, acts, c, s0, s1, sink, Assign::Eq),
            Builtin::In => self.next_assign(a, pred, args, acts, c, s0, s1, sink, Assign::In),
            Builtin::Subseteq => self.next_assign(a, pred, args, acts, c, s0, s1, sink, Assign::Subseteq),
            Builtin::Implies => {
                let v = self.eval(arg(0), c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                match v {
                    Value::Bool(true) => self.next_impl(a, arg(1), acts, c, s0, s1, sink),
                    Value::Bool(false) => self.next_continue(a, acts, s0, s1, sink),
                    _ => {
                        let reason = format!(
                            "In computing next states of a predicate of the form P => Q, P was\n{}.\n{}",
                            v.kind_string(),
                            graph.expr_tlc_text(pred)
                        );
                        Err(EvalError::detailed(reason, Some(graph.loc(pred))).into())
                    }
                }
            }
            Builtin::Unchanged => self.process_unchanged(a, arg(0), acts, c, s0, s1, sink),
            Builtin::Nop => self.next_impl(a, arg(0), acts, c, s0, s1, sink),
            Builtin::Cdot => Err(super::enabled::cdot_unsupported(self, pred).into()),
            // Every other built-in operator is a boolean filter.
            _ => {
                let v = self.eval(pred, c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                self.next_filter(a, pred, acts, s0, s1, sink, v)
            }
        }
    }

    /// `getNextStatesApplUsrDefOp` and the `default` case: the value must be a boolean, and a
    /// true one continues with the remaining items.
    fn next_filter(
        &mut self,
        a: &Action,
        pred: ExprId,
        acts: Acts<'_>,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
        v: Value,
    ) -> Flow {
        let Value::Bool(b) = v else {
            let graph = &self.tool().graph;
            let shown = v.to_tlc_string(self)?;
            let message =
                self.out.text(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, &["next states", "boolean", &shown, &graph.expr_tlc_text(pred)]);
            return Err(EvalError::runtime(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, message).with_loc(Some(graph.loc(pred)))
            .into());
        };
        if b { self.next_continue(a, acts, s0, s1, sink) } else { Ok(()) }
    }

    /// `x' = e`, `x' \in S` and `x' \subseteq S`: bind the primed variable if it is still
    /// free, otherwise test what it is bound to.
    #[allow(clippy::too_many_arguments)]
    fn next_assign(
        &mut self,
        a: &Action,
        pred: ExprId,
        args: &[ExprOrOpArg],
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
        kind: Assign,
    ) -> Flow {
        let graph = &self.tool().graph;
        let Some(slot) = self.get_primed_var_slot(args[0].expect_expr(), c, false) else {
            // Not an assignment: TLC casts the value to a BoolValue without checking.
            let v = self.eval(pred, c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
            let Value::Bool(b) = v else { return Err(EvalError::class_cast(v.java_class(), "BoolValue").into()) };
            if !b {
                return Ok(());
            }
            return self.next_continue(a, acts, s0, s1, sink);
        };
        let rval = self.eval(args[1].expect_expr(), c, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
        let rval = if kind == Assign::Subseteq {
            self.set_source(args[1].expect_expr(), crate::value::subset(rval))
        } else {
            rval
        };
        let bound = s1.lookup(slot).cloned();
        match (kind, bound) {
            (Assign::Eq, None) => {
                s1.bind(slot, rval);
                let r = self.next_continue(a, acts, s0, s1, sink);
                s1.unbind(slot);
                r
            }
            (Assign::Eq, Some(lval)) => {
                if lval.tlc_equals(self, &rval)? { self.next_continue(a, acts, s0, s1, sink) } else { Ok(()) }
            }
            (_, None) => {
                if !rval.is_enumerable() {
                    let reason = format!(
                        "In computing next states, the right side of \\IN is not enumerable.\n{}",
                        graph.expr_tlc_text(pred)
                    );
                    return Err(EvalError::detailed(reason, Some(graph.loc(pred))).into());
                }
                let mut elems = rval.elements(self, crate::value::enumerate::EnumOrder::Native)?.expect("an enumerable value enumerates");
                while let Some(elem) = elems.next(self)? {
                    s1.bind(slot, elem);
                    let r = self.next_continue(a, acts, s0, s1, sink);
                    s1.unbind(slot);
                    r?;
                }
                Ok(())
            }
            (_, Some(lval)) => {
                if rval.member(self, &lval)? { self.next_continue(a, acts, s0, s1, sink) } else { Ok(()) }
            }
        }
    }

    /// Conjunction: push `args[1..]` (so they run left to right after `args[0]`), then
    /// evaluate `args[0]`. TLC: `for i = alen-1 .. 1: acts = acts.cons(args[i], c, i)`.
    fn next_conj(
        &mut self,
        a: &Action,
        args: &[ExprOrOpArg],
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
    ) -> Flow {
        let Some((first, rest)) = args.split_first() else {
            return self.next_continue(a, acts, s0, s1, sink);
        };
        let first = first.expect_expr();
        self.push_conjuncts(rest, c, acts, &mut |ev, acts1| ev.next_impl(a, first, acts1, c, s0, s1, sink))
    }

    /// Builds the continuation `rest[0] :: rest[1] :: ... :: acts` on the stack, then calls `k`.
    fn push_conjuncts(
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
                self.push_conjuncts(init, c, Acts::Cons(&item), k)
            }
        }
    }

    /// `\A x \in S : P`: the bindings after the first become predicate items, consed in
    /// enumeration order so the list holds them reversed, exactly as TLC's loop does.
    #[allow(clippy::too_many_arguments)]
    fn push_forall(
        &mut self,
        a: &Action,
        body: ExprId,
        rest: &[Ctx],
        acts: Acts<'_>,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
        first: &Ctx,
    ) -> Flow {
        match rest.split_first() {
            None => self.next_impl(a, body, acts, first, s0, s1, sink),
            Some((next, tail)) => {
                let item = ActItem { pred: body, ctx: next, kind: ActKind::Pred, next: acts };
                self.push_forall(a, body, tail, Acts::Cons(&item), s0, s1, sink, first)
            }
        }
    }

    /// `getNextStates0`: pop the next item, or emit the finished successor.
    pub(crate) fn next_continue(&mut self, a: &Action, acts: Acts<'_>, s0: &State, s1: &mut PartialState, sink: &mut dyn NextSink) -> Flow {
        let Acts::Cons(item) = acts else {
            return sink.add_next(self, s0, a.id(), s1);
        };
        if self.tool.globals().warn && s1.all_assigned() {
            return self.next_all_assigned(a, acts, s0, s1, sink);
        }
        match item.kind {
            ActKind::Conj(_) | ActKind::Pred => self.next_impl(a, item.pred, item.next, item.ctx, s0, s1, sink),
            ActKind::Unchanged => self.process_unchanged(a, item.pred, item.next, item.ctx, s0, s1, sink),
            ActKind::Changed => {
                let v0 = self.eval(item.pred, item.ctx, StateRef::Full(s0), StateRef::Empty, EvalControl::CLEAR)?;
                let v1 = self.eval(item.pred, item.ctx, StateRef::Partial(s1), StateRef::Null, EvalControl::CLEAR)?;
                if v0.tlc_equals(self, &v1)? { Ok(()) } else { self.next_continue(a, item.next, s0, s1, sink) }
            }
        }
    }

    /// `getNextStatesAllAssigned`: every variable bound, remaining items are boolean tests.
    fn next_all_assigned(&mut self, a: &Action, acts: Acts<'_>, s0: &State, s1: &mut PartialState, sink: &mut dyn NextSink) -> Flow {
        let mut acts = acts;
        while let Acts::Cons(item) = acts {
            match item.kind {
                ActKind::Conj(_) | ActKind::Pred => {
                    let v = self.eval(item.pred, item.ctx, StateRef::Full(s0), StateRef::Partial(s1), EvalControl::CLEAR)?;
                    let Value::Bool(b) = v else {
                        let graph = &self.tool().graph;
                        let shown = v.to_tlc_string(self)?;
                        let text = graph.expr_tlc_text(item.pred);
                        let message = self.out.text(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, &["next states", "boolean", &shown, &text]);
                        return Err(EvalError::runtime(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, message).with_loc(Some(graph.loc(item.pred)))
                        .into());
                    };
                    if !b {
                        return sink.add_unsatisfied(self, s0, a.id(), s1);
                    }
                }
                ActKind::Unchanged => return self.process_unchanged(a, item.pred, item.next, item.ctx, s0, s1, sink),
                ActKind::Changed => {
                    let v0 = self.eval(item.pred, item.ctx, StateRef::Full(s0), StateRef::Empty, EvalControl::CLEAR)?;
                    let v1 = self.eval(item.pred, item.ctx, StateRef::Partial(s1), StateRef::Null, EvalControl::CLEAR)?;
                    if v0.tlc_equals(self, &v1)? {
                        return Ok(());
                    }
                }
            }
            acts = item.next;
        }
        sink.add_next(self, s0, a.id(), s1)
    }

    /// `processUnchanged`: bind unbound primed vars to their s0 values, test bound ones.
    fn process_unchanged(
        &mut self,
        a: &Action,
        expr: ExprId,
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
    ) -> Flow {
        self.enter_generator(expr)?;
        let r = self.process_unchanged_inner(a, expr, acts, c, s0, s1, sink);
        self.leave_generator(r.as_ref().err());
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn process_unchanged_inner(
        &mut self,
        a: &Action,
        expr: ExprId,
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
    ) -> Flow {
        let graph = &self.tool().graph;
        if let Some(slot) = self.get_var_slot(expr, c, false) {
            let val0 = s0.get(slot).clone();
            match s1.lookup(slot).cloned() {
                None => {
                    s1.bind(slot, val0);
                    let r = self.next_continue(a, acts, s0, s1, sink);
                    s1.unbind(slot);
                    return r;
                }
                Some(val1) => {
                    if val0.tlc_equals(self, &val1)? {
                        return self.next_continue(a, acts, s0, s1, sink);
                    }
                    let name = graph.interner().text(self.tool.vars().names[slot.index()]).to_string();
                    crate::value::warn(self, Ec::TLC_UNCHANGED_VARIABLE_CHANGED, &[&name, &graph.expr_tlc_text(expr)])?;
                    return Ok(());
                }
            }
        }
        if let ExprNode::OpAppl { op, args, .. } = graph.expr(expr) {
            let (op, args) = (*op, &**args);
            if self.opcode(op) == Some(Builtin::Tuple) {
                let Some((first, rest)) = args.split_first() else {
                    return self.next_continue(a, acts, s0, s1, sink);
                };
                let first = first.expect_expr();
                return self.push_unchanged(rest, c, acts, &mut |ev, acts1| ev.process_unchanged(a, first, acts1, c, s0, s1, sink));
            }
            // A 0-arity user operator: follow it to its body.
            if self.opcode(op).is_none() && args.is_empty() {
                match self.lookup(op, c, StateRef::Empty, false) {
                    Resolved::UserOp(d) | Resolved::Binding(Binding::OpDef(d)) => {
                        if let Some(body) = graph.op_def(d).body {
                            return self.process_unchanged(a, body, acts, c, s0, s1, sink);
                        }
                    }
                    Resolved::Binding(Binding::Thunk(t)) => {
                        let (e, tc) = (t.expr, t.ctx.clone());
                        return self.process_unchanged(a, e, acts, &tc, s0, s1, sink);
                    }
                    _ => {}
                }
            }
        }
        self.verify_unchanged(a, expr, acts, c, s0, s1, sink)
    }

    /// `verifyUnchanged`: the expression is compared in both states without synthesising any
    /// successor value.
    fn verify_unchanged(
        &mut self,
        a: &Action,
        expr: ExprId,
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
    ) -> Flow {
        let v0 = self.eval(expr, c, StateRef::Full(s0), StateRef::Empty, EvalControl::CLEAR)?;
        let v1 = self.eval(expr, c, StateRef::Partial(s1), StateRef::Null, EvalControl::CLEAR)?;
        if v0.tlc_equals(self, &v1)? { self.next_continue(a, acts, s0, s1, sink) } else { Ok(()) }
    }

    /// `UNCHANGED <<a, b, c>>`: the components after the first become UNCHANGED items.
    fn push_unchanged(
        &mut self,
        rest: &[ExprOrOpArg],
        c: &Ctx,
        acts: Acts<'_>,
        k: &mut dyn FnMut(&mut Self, Acts<'_>) -> Flow,
    ) -> Flow {
        match rest.split_last() {
            None => k(self, acts),
            Some((last, init)) => {
                let item = ActItem { pred: last.expect_expr(), ctx: c, kind: ActKind::Unchanged, next: acts };
                self.push_unchanged(init, c, Acts::Cons(&item), k)
            }
        }
    }

    /// `getNextStatesImplSubstInKind`: cons one fresh `Thunk` (cachable = false) per
    /// substitution over the *outer* context, then recurse into the body.
    fn next_subst_in(
        &mut self,
        a: &Action,
        pred: ExprId,
        acts: Acts<'_>,
        c: &Ctx,
        s0: &State,
        s1: &mut PartialState,
        sink: &mut dyn NextSink,
    ) -> Flow {
        let graph = &self.tool().graph;
        let (ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. }) = graph.expr(pred) else {
            unreachable!("only a SubstIn reaches here")
        };
        let body = *body;
        let c1 = self.subst_context(substs, c);
        self.next_impl(a, body, acts, &c1, s0, s1, sink)
    }
}

/// Which assignment form `next_assign` handles.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Assign {
    Eq,
    In,
    Subseteq,
}
