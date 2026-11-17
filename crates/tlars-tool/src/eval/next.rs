//! `getNextStates*`: continuation-passing over an action-item list.
//!
//! `ActionItemList` becomes [`Acts`], a cons list *borrowed from the Rust stack*: each
//! conjunct pushes one `ActItem` local and recurses, so generating a successor allocates no
//! list cells, and the continuation's lifetime is checked by the compiler.
//! The successor is built in one `&mut PartialState` with bind/unbind, as in TLC.

use super::{EvalControl, Ev, NextSink};
use crate::actions::Action;
use crate::ctx::Ctx;
use crate::error::Flow;
use crate::fp64::Fp;
use crate::state::{PartialState, State, StateRef};
use tlars_sany::graph::{BuiltinOp, ExprId, ExprNode, ExprOrOpArg};

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
        self.begin_expansion(s0_fp);
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
        let mut s1 = PartialState::empty(s0.len());
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
        let tool = self.tool;
        match tool.graph().expr(pred) {
            ExprNode::OpAppl { .. } => self.next_appl(a, pred, acts, c, s0, s1, sink),
            ExprNode::LetIn { body, .. } | ExprNode::Label { body, .. } => self.next_impl(a, *body, acts, c, s0, s1, sink),
            ExprNode::SubstIn { .. } | ExprNode::APSubstIn { .. } => self.next_subst_in(a, pred, acts, c, s0, s1, sink),
            _ => Err(self.not_boolean_next(pred, c).into()),
        }
    }

    /// `getNextStatesApplImpl` (excerpt: the connectives that shape successor order).
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
        let tool = self.tool;
        let ExprNode::OpAppl { op, args, .. } = tool.graph().expr(pred) else { unreachable!() };
        match self.opcode(*op, c) {
            Some(BuiltinOp::ConjList | BuiltinOp::Land) => self.next_conj(a, args, acts, c, s0, s1, sink),
            Some(BuiltinOp::DisjList | BuiltinOp::Lor) => {
                for arg in args.iter() {
                    self.next_impl(a, arg.expect_expr(), acts, c, s0, s1, sink)?;
                }
                Ok(())
            }
            // TODO(pseudocode, the rest of getNextStatesApplImpl):
            //   user op (opcode 0): lookup(op, c) -> OpDef: recurse into body with
            //     getOpContext(args as call-by-name Thunks); LazyValue: use cached value or
            //     recurse into lv.expr in lv.ctx; ThmOrAssumpDef likewise.
            //   \E x \in S: for ctx in contexts(S, Normalized, first var fastest) recurse.
            //   \A: cons all but first context onto acts.
            //   x' = e: if x' unbound bind (follow Thunk to its expr: `x <- y` binds y'),
            //     else test equals. x' \in S: S.elements(Native) — duplicates of lazy
            //     SetCup/Union are generated twice (counts as "generated").
            //   IF/CASE: guard, one branch (CASE: first true guard in source order).
            //   [A]_v: A's states, then UNCHANGED v.  <A>_v: push Changed(v).
            //   UNCHANGED e: process_unchanged.  f[x] on FcnLambda: expand body.
            //   anything else: boolean filter via eval, then next_continue.
            _ => todo!(),
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
                let v1 = self.eval(item.pred, item.ctx, StateRef::Partial(s1), StateRef::Empty, EvalControl::CLEAR)?;
                if v0.tlc_equals(self, &v1)? { Ok(()) } else { self.next_continue(a, item.next, s0, s1, sink) }
            }
        }
    }

    /// `getNextStatesAllAssigned`: every variable bound, remaining items are boolean tests.
    fn next_all_assigned(&mut self, a: &Action, acts: Acts<'_>, s0: &State, s1: &mut PartialState, sink: &mut dyn NextSink) -> Flow {
        todo!()
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
        todo!()
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
        todo!()
    }
}
