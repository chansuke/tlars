//! `enabled*` (`Tool.java:2953-3600`): the next-state generator run over a `FunState`
//! (TLCStateFun) with a branched `Ctx`, answering whether *some* successor exists.
//!
//! The same continuation-passing shape as `next`, with two differences TLC also has:
//! the successor is a persistent [`FunState`] (so a binding needs no unbind), and the
//! answer is the first `true`, so every loop stops at the first enabled branch.

use super::expr::{Builtin, Resolved};
use super::next::{ActItem, ActKind, Acts};
use super::{EvalControl, Ev};
use crate::ctx::{Binding, Ctx};
use crate::error::{EvalError, EvalResult};
use crate::state::{FunState, StateRef};
use crate::value::Value;
use tlars_msg::Ec;
use tlars_sany::graph::{ExprId, ExprNode, ExprOrOpArg, SymbolRef};

/// `Assert.fail("The current version of TLC does not support action composition. ...")`,
/// shared by `evalAppl`, `getNextStatesAppl` and `enabledAppl` (`Tool.java:1527`, `:2804`,
/// `:3389`). The `tlc2.tool.impl.Tool.cdot` property is read at class load, and tlars has no
/// way to set it, so the incomplete implementation behind it is unreachable.
pub(crate) fn cdot_unsupported(ev: &mut Ev<'_>, pred: ExprId) -> EvalError {
    let graph = &ev.tool().graph;
    let reason = "The current version of TLC does not support action composition.  An incomplete implementation can be \
                  enabled via the tlc2.tool.impl.Tool.cdot=true java property."
        .to_string();
    EvalError::detailed(reason, Some(graph.loc(pred)))
}

impl<'t> Ev<'t> {
    /// `Tool.enabled(pred, acts, c, s0, s1)` (`:2977-3036`): `true` where TLC returns a state.
    pub(crate) fn enabled(&mut self, pred: ExprId, acts: Acts<'_>, c: &Ctx, s0: StateRef<'_>, s1: &FunState) -> EvalResult<bool> {
        self.enter(pred)?;
        let r = self.enabled_impl(pred, acts, c, s0, s1);
        self.leave(r.as_ref().err().map(|e| &e.kind));
        r
    }

    /// `enabledImpl`: dispatch on node kind.
    fn enabled_impl(&mut self, pred: ExprId, acts: Acts<'_>, c: &Ctx, s0: StateRef<'_>, s1: &FunState) -> EvalResult<bool> {
        let graph = self.tool().graph.clone();
        match graph.expr(pred) {
            ExprNode::OpAppl { .. } => self.enabled_appl(pred, acts, c, s0, s1),
            ExprNode::LetIn { defs, body, .. } => {
                let body = *body;
                let c1 = self.let_context(defs, c);
                self.enabled(body, acts, &c1, s0, s1)
            }
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                let body = *body;
                let c1 = self.subst_context(substs, c);
                self.enabled(body, acts, &c1, s0, s1)
            }
            ExprNode::Label { body, .. } => {
                let body = *body;
                self.enabled(body, acts, c, s0, s1)
            }
            _ => {
                let reason = format!("Attempted to compute ENABLED on a non-boolean expression.\n{}", graph.expr_tlc_text(pred));
                Err(EvalError::detailed(reason, Some(graph.loc(pred))))
            }
        }
    }

    /// `enabled(acts, s0, s1)` (`:3039-3069`): pop the next item, or answer "enabled".
    fn enabled_continue(&mut self, acts: Acts<'_>, s0: StateRef<'_>, s1: &FunState) -> EvalResult<bool> {
        let Acts::Cons(item) = acts else { return Ok(true) };
        match item.kind {
            ActKind::Conj(_) | ActKind::Pred => self.enabled(item.pred, item.next, item.ctx, s0, s1),
            ActKind::Unchanged => self.enabled_unchanged(item.pred, item.next, item.ctx, s0, s1),
            // `pred' # pred` of `<A>_e`. The second evaluation passes `Primed` alone, so a
            // cached `LazyValue` may be reused there (`Tool.java:3061-3064`).
            ActKind::Changed => {
                let v1 = self.eval(item.pred, item.ctx, s0, StateRef::Empty, EvalControl::ENABLED)?;
                let v2 = self.eval(item.pred, item.ctx, StateRef::Fun(s1), StateRef::Null, EvalControl::PRIMED)?;
                if v1.tlc_equals(self, &v2)? { Ok(false) } else { self.enabled_continue(item.next, s0, s1) }
            }
        }
    }

    /// `enabledApplImpl` (`Tool.java:3073-3504`).
    fn enabled_appl(&mut self, pred: ExprId, acts: Acts<'_>, c: &Ctx, s0: StateRef<'_>, s1: &FunState) -> EvalResult<bool> {
        let graph = self.tool().graph.clone();
        let ExprNode::OpAppl { op, args, .. } = graph.expr(pred) else { unreachable!() };
        let (op, args) = (*op, args.clone());
        let mut code = self.opcode(op);
        if code.is_none() {
            // A user-defined operator, unless a cfg override or INSTANCE put a built-in here.
            match self.lookup(op, c, s0, false) {
                Resolved::UserOp(d) | Resolved::Binding(Binding::OpDef(d)) => {
                    code = self.opcode(SymbolRef::OpDef(d));
                    if code.is_none() {
                        let body = graph.op_def(d).body.expect("a user definition has a body");
                        let c1 = self.op_context(d, &args, c, true);
                        return self.enabled(body, acts, &c1, s0, s1);
                    }
                }
                // Unlike `getNextStatesAppl`, ENABLED never reads a cached `LazyValue`: it
                // always expands the body (`Tool.java:3115-3118`).
                Resolved::Binding(Binding::Thunk(t)) => {
                    let (e, tc) = (t.expr, t.ctx.clone());
                    return self.enabled(e, acts, &tc, s0, s1);
                }
                other => {
                    let v = self.apply_resolved(other, op, pred, &args, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                    return self.enabled_filter(pred, acts, s0, s1, v);
                }
            }
        }
        let Some(code) = code else { unreachable!("resolved above") };
        let arg = |i: usize| args[i].expect_expr();
        let temporal = |ev: &mut Ev<'_>, what: &str| -> EvalError {
            let graph = ev.tool().graph.clone();
            let message = ev.out().text(Ec::TLC_ENABLED_WRONG_FORMULA, &[what, &graph.expr_tlc_text(pred)]);
            EvalError { loc: Some(graph.loc(pred)), ..EvalError::runtime(Ec::TLC_ENABLED_WRONG_FORMULA, message) }
        };
        let encountered = |ev: &mut Ev<'_>, what: &str| -> EvalError {
            let graph = ev.tool().graph.clone();
            let reason = format!("In computing ENABLED, TLC encountered {what}\n{}", graph.expr_tlc_text(pred));
            EvalError::detailed(reason, Some(graph.loc(pred)))
        };
        match code {
            Builtin::AngleAct => {
                let item = ActItem { pred: args[1].expect_expr(), ctx: c, kind: ActKind::Changed, next: acts };
                self.enabled(arg(0), Acts::Cons(&item), c, s0, s1)
            }
            Builtin::BoundedExists => {
                let ExprNode::OpAppl { bound, .. } = graph.expr(pred) else { unreachable!() };
                let bound = bound.as_deref().expect("a bounded quantifier has bounds");
                let body = arg(0);
                let mut contexts = self.contexts(bound, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                while let Some(c1) = contexts.next(self)? {
                    if self.enabled(body, acts, &c1, s0, s1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            // The first binding is evaluated, the rest become predicate items.
            Builtin::BoundedForall => {
                let ExprNode::OpAppl { bound, .. } = graph.expr(pred) else { unreachable!() };
                let bound = bound.as_deref().expect("a bounded quantifier has bounds");
                let body = arg(0);
                let mut contexts = self.contexts(bound, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                let Some(first) = contexts.next(self)? else {
                    return self.enabled_continue(acts, s0, s1);
                };
                let mut rest = Vec::new();
                while let Some(c2) = contexts.next(self)? {
                    rest.push(c2);
                }
                self.push_enabled_forall(body, &rest, acts, s0, s1, &first)
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
                            let v = self.eval(guard, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                            match v {
                                Value::Bool(true) => return self.enabled(pair_args[1].expect_expr(), acts, c, s0, s1),
                                Value::Bool(false) => {}
                                _ => {
                                    let arm = pair_args[1].expect_expr();
                                    let reason = format!(
                                        "In computing ENABLED, a non-boolean expression({}) was used as a guard condition of a CASE.\n{}",
                                        v.kind_string(),
                                        graph.expr_tlc_text(arm)
                                    );
                                    return Err(EvalError::detailed(reason, Some(graph.loc(arm))));
                                }
                            }
                        }
                    }
                }
                match other {
                    Some(e) => self.enabled(e, acts, c, s0, s1),
                    None => Err(encountered(self, "a CASE with no conditions true.")),
                }
            }
            Builtin::ConjList | Builtin::Land => {
                let Some((first, rest)) = args.split_first() else {
                    return self.enabled_continue(acts, s0, s1);
                };
                let first = first.expect_expr();
                self.push_enabled_conjuncts(rest, c, acts, &mut |ev, acts1| ev.enabled(first, acts1, c, s0, s1))
            }
            Builtin::DisjList | Builtin::Lor => {
                for a_i in args.iter() {
                    if self.enabled(a_i.expect_expr(), acts, c, s0, s1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            // `f[x]` on a lazy function expands the body at `x`.
            Builtin::FcnApply => {
                let fval = self.eval(arg(0), c, s0, StateRef::Fun(s1), EvalControl::ENABLED.with(EvalControl::KEEP_LAZY))?;
                let fval = match &fval {
                    Value::FcnLambda(l) if l.converted.get().is_none() => {
                        let c1 = self.fcn_context(l, &args, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                        let body = l.body;
                        return self.enabled(body, acts, &c1, s0, s1);
                    }
                    Value::FcnLambda(l) => Value::FcnRcd(l.converted.get().expect("just checked").clone()),
                    _ => fval.clone(),
                };
                if !fval.is_function() {
                    let reason = format!(
                        "In computing ENABLED, a non-function ({}) was applied as a function.\n{}",
                        fval.kind_string(),
                        graph.expr_tlc_text(pred)
                    );
                    return Err(EvalError::detailed(reason, Some(graph.loc(pred))));
                }
                let argv = self.eval(arg(1), c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                let bval = fval.apply(self, &argv, EvalControl::ENABLED)?;
                let Value::Bool(b) = bval else {
                    let message = self.out().text(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING2, &["ENABLED", "boolean", &graph.expr_tlc_text(pred)]);
                    return Err(EvalError {
                        loc: Some(graph.loc(arg(1))),
                        ..EvalError::runtime(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING2, message)
                    });
                };
                if !b {
                    return Ok(false);
                }
                self.enabled_continue(acts, s0, s1)
            }
            Builtin::IfThenElse => {
                let v = self.eval(arg(0), c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                match v {
                    Value::Bool(b) => self.enabled(if b { arg(1) } else { arg(2) }, acts, c, s0, s1),
                    _ => {
                        let reason = format!(
                            "In computing ENABLED, a non-boolean expression({}) was used as the guard condition of an IF.\n{}",
                            v.kind_string(),
                            graph.expr_tlc_text(pred)
                        );
                        Err(EvalError::detailed(reason, Some(graph.loc(pred))))
                    }
                }
            }
            // `[A]_e`: A, then the stuttering step.
            Builtin::SquareAct => {
                if self.enabled(arg(0), acts, c, s0, s1)? {
                    return Ok(true);
                }
                self.enabled_unchanged(args[1].expect_expr(), acts, c, s0, s1)
            }
            Builtin::TemporalExists | Builtin::TemporalForall => Err(encountered(self, "temporal quantifier.")),
            Builtin::UnboundedChoose => {
                Err(encountered(self, "unbounded CHOOSE. Make sure that the expression is of form CHOOSE x \\in S: P(x)."))
            }
            Builtin::UnboundedExists => {
                Err(encountered(self, "unbounded quantifier. Make sure that the expression is of form \\E x \\in S: P(x)."))
            }
            Builtin::UnboundedForall => {
                Err(encountered(self, "unbounded quantifier. Make sure that the expression is of form \\A x \\in S: P(x)."))
            }
            Builtin::Unimplemented("$SF") => Err(temporal(self, "SF")),
            Builtin::Unimplemented("$WF") => Err(temporal(self, "WF")),
            Builtin::Box => Err(temporal(self, "[]")),
            Builtin::Unimplemented("<>") => Err(temporal(self, "<>")),
            Builtin::Unchanged => self.enabled_unchanged(arg(0), acts, c, s0, s1),
            Builtin::Eq => {
                let Some(slot) = self.get_primed_var(args[0].expect_expr(), c, true) else {
                    // Not an assignment: TLC casts the value to a BoolValue without checking.
                    let v = self.eval(pred, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                    let Value::Bool(b) = v else { return Err(EvalError::class_cast(v.java_class(), "BoolValue")) };
                    if !b {
                        return Ok(false);
                    }
                    return self.enabled_continue(acts, s0, s1);
                };
                let lval = s1.lookup(slot).cloned();
                let rval = self.eval(args[1].expect_expr(), c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                match lval {
                    None => {
                        let s2 = s1.bind(slot, rval);
                        self.enabled_continue(acts, s0, &s2)
                    }
                    Some(lval) => {
                        if !lval.tlc_equals(self, &rval)? {
                            return Ok(false);
                        }
                        self.enabled_continue(acts, s0, s1)
                    }
                }
            }
            Builtin::Implies => {
                let v = self.eval(arg(0), c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                match v {
                    Value::Bool(true) => self.enabled(arg(1), acts, c, s0, s1),
                    Value::Bool(false) => self.enabled_continue(acts, s0, s1),
                    _ => {
                        let reason = format!(
                            "While computing ENABLED of an expression of the form P => Q, P was {}.\n{}",
                            v.kind_string(),
                            graph.expr_tlc_text(pred)
                        );
                        Err(EvalError::detailed(reason, Some(graph.loc(pred))))
                    }
                }
            }
            Builtin::Cdot => Err(cdot_unsupported(self, pred)),
            Builtin::Unimplemented("~>") => Err(encountered(self, "a temporal formula (a ~> b).")),
            Builtin::Unimplemented("-+->") => Err(encountered(self, "a temporal formula (a -+-> formula).")),
            Builtin::Subseteq => self.enabled_in(pred, &args, acts, c, s0, s1, true),
            Builtin::In => self.enabled_in(pred, &args, acts, c, s0, s1, false),
            Builtin::Nop => self.enabled(arg(0), acts, c, s0, s1),
            // Every other built-in operator is a boolean filter.
            _ => {
                let v = self.eval(pred, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
                self.enabled_filter(pred, acts, s0, s1, v)
            }
        }
    }

    /// The user-defined-operator and `default` cases: the value must be a boolean, and a true
    /// one continues with the remaining items.
    fn enabled_filter(&mut self, pred: ExprId, acts: Acts<'_>, s0: StateRef<'_>, s1: &FunState, v: Value) -> EvalResult<bool> {
        let Value::Bool(b) = v else {
            let graph = self.tool().graph.clone();
            let shown = v.to_tlc_string(self)?;
            let message =
                self.out().text(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, &["ENABLED", "boolean", &shown, &graph.expr_tlc_text(pred)]);
            return Err(EvalError {
                loc: Some(graph.loc(pred)),
                ..EvalError::runtime(Ec::TLC_EXPECTED_EXPRESSION_IN_COMPUTING, message)
            });
        };
        if b { self.enabled_continue(acts, s0, s1) } else { Ok(false) }
    }

    /// `x' \in S` and `x' \subseteq S`: bind the primed variable to each element in turn if it
    /// is still free, otherwise test what it is bound to.
    #[allow(clippy::too_many_arguments)]
    fn enabled_in(
        &mut self,
        pred: ExprId,
        args: &[ExprOrOpArg],
        acts: Acts<'_>,
        c: &Ctx,
        s0: StateRef<'_>,
        s1: &FunState,
        subset: bool,
    ) -> EvalResult<bool> {
        let graph = self.tool().graph.clone();
        let Some(slot) = self.get_primed_var(args[0].expect_expr(), c, true) else {
            let v = self.eval(pred, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
            let Value::Bool(b) = v else { return Err(EvalError::class_cast(v.java_class(), "BoolValue")) };
            if !b {
                return Ok(false);
            }
            return self.enabled_continue(acts, s0, s1);
        };
        let lval = s1.lookup(slot).cloned();
        let rval = self.eval(args[1].expect_expr(), c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
        let rval = if subset { self.set_source(args[1].expect_expr(), crate::value::subset(rval)) } else { rval };
        match lval {
            None => {
                // A `SubsetValue` is always enumerable, so only `\in` checks (`:3443-3446`).
                if !subset && !rval.is_enumerable() {
                    let reason = format!("The right side of \\IN is not enumerable.\n{}", graph.expr_tlc_text(pred));
                    return Err(EvalError::detailed(reason, Some(graph.loc(pred))));
                }
                let mut elems = rval.elements(self, crate::value::enumerate::EnumOrder::Native)?.expect("an enumerable value enumerates");
                while let Some(elem) = elems.next(self)? {
                    let s2 = s1.bind(slot, elem);
                    if self.enabled_continue(acts, s0, &s2)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Some(lval) => {
                if !rval.member(self, &lval)? {
                    return Ok(false);
                }
                self.enabled_continue(acts, s0, s1)
            }
        }
    }

    /// Conjunction: push `args[1..]` (so they run left to right after `args[0]`), then
    /// evaluate `args[0]`. TLC: `for i = alen-1 .. 1: acts = acts.cons(args[i], c, i)`.
    fn push_enabled_conjuncts(
        &mut self,
        rest: &[ExprOrOpArg],
        c: &Ctx,
        acts: Acts<'_>,
        k: &mut dyn FnMut(&mut Self, Acts<'_>) -> EvalResult<bool>,
    ) -> EvalResult<bool> {
        match rest.split_last() {
            None => k(self, acts),
            Some((last, init)) => {
                let item = ActItem { pred: last.expect_expr(), ctx: c, kind: ActKind::Conj(rest.len() as u32), next: acts };
                self.push_enabled_conjuncts(init, c, Acts::Cons(&item), k)
            }
        }
    }

    /// `\A x \in S : P`: the bindings after the first become predicate items, consed in
    /// enumeration order so the list holds them reversed, exactly as TLC's loop does.
    fn push_enabled_forall(
        &mut self,
        body: ExprId,
        rest: &[Ctx],
        acts: Acts<'_>,
        s0: StateRef<'_>,
        s1: &FunState,
        first: &Ctx,
    ) -> EvalResult<bool> {
        match rest.split_first() {
            None => self.enabled(body, acts, first, s0, s1),
            Some((next, tail)) => {
                let item = ActItem { pred: body, ctx: next, kind: ActKind::Pred, next: acts };
                self.push_enabled_forall(body, tail, Acts::Cons(&item), s0, s1, first)
            }
        }
    }

    /// `enabledUnchanged` (`:3509-3600`).
    fn enabled_unchanged(&mut self, expr: ExprId, acts: Acts<'_>, c: &Ctx, s0: StateRef<'_>, s1: &FunState) -> EvalResult<bool> {
        self.enter(expr)?;
        let r = self.enabled_unchanged_impl(expr, acts, c, s0, s1);
        self.leave(r.as_ref().err().map(|e| &e.kind));
        r
    }

    fn enabled_unchanged_impl(&mut self, expr: ExprId, acts: Acts<'_>, c: &Ctx, s0: StateRef<'_>, s1: &FunState) -> EvalResult<bool> {
        let graph = self.tool().graph.clone();
        // `getVar(expr, c, true)`: the cutoff keeps the search inside the ENABLED branch.
        if let Some(slot) = self.get_var(expr, c, true) {
            // Unlike `processUnchanged`, the old value comes from evaluating the expression,
            // not from `s0.lookup` (`Tool.java:3512-3515`).
            let v0 = self.eval(expr, c, s0, StateRef::Fun(s1), EvalControl::ENABLED)?;
            match s1.lookup(slot).cloned() {
                None => {
                    let s2 = s1.bind(slot, v0);
                    return self.enabled_continue(acts, s0, &s2);
                }
                Some(v1) => {
                    if v1.tlc_equals(self, &v0)? {
                        return self.enabled_continue(acts, s0, s1);
                    }
                    let name = graph.interner().text(self.tool().vars().names[slot.index()]).to_string();
                    crate::value::warn(self, Ec::TLC_UNCHANGED_VARIABLE_CHANGED, &[&name, &graph.expr_tlc_text(expr)])?;
                    return Ok(false);
                }
            }
        }
        if let ExprNode::OpAppl { op, args, .. } = graph.expr(expr) {
            let (op, args) = (*op, args.clone());
            if self.opcode(op) == Some(Builtin::Tuple) {
                let Some((first, rest)) = args.split_first() else {
                    return self.enabled_continue(acts, s0, s1);
                };
                let first = first.expect_expr();
                return self.push_enabled_unchanged(rest, c, acts, &mut |ev, acts1| ev.enabled_unchanged(first, acts1, c, s0, s1));
            }
            // A 0-arity user operator: follow it to its body. Anything else (a constant, say)
            // leaves the UNCHANGED unchecked, which is where `processUnchanged` instead falls
            // through to `verifyUnchanged` (`Tool.java:3556-3570`).
            if self.opcode(op).is_none() && args.is_empty() {
                match self.lookup(op, c, StateRef::Empty, false) {
                    Resolved::Binding(Binding::Thunk(t)) => {
                        let (e, tc) = (t.expr, t.ctx.clone());
                        return self.enabled_unchanged(e, acts, &tc, s0, s1);
                    }
                    Resolved::UserOp(d) | Resolved::Binding(Binding::OpDef(d)) => {
                        if let Some(body) = graph.op_def(d).body {
                            return self.enabled_unchanged(body, acts, c, s0, s1);
                        }
                    }
                    _ => {}
                }
                return self.enabled_continue(acts, s0, s1);
            }
        }
        // `verifyUnchanged`'s counterpart: both states are read with no successor, and the
        // second evaluation passes `Primed` alone (`Tool.java:3572-3599`).
        let v0 = self.eval(expr, c, s0, StateRef::Empty, EvalControl::ENABLED)?;
        let v1 = self.eval(expr, c, StateRef::Fun(s1), StateRef::Empty, EvalControl::PRIMED)?;
        if !v0.tlc_equals(self, &v1)? {
            return Ok(false);
        }
        self.enabled_continue(acts, s0, s1)
    }

    /// `UNCHANGED <<a, b, c>>`: the components after the first become UNCHANGED items, consed
    /// in *forward* order, so they are processed last to first. `processUnchanged` conses them
    /// the other way round (`Tool.java:3544-3550` against `:1738-1745`).
    fn push_enabled_unchanged(
        &mut self,
        rest: &[ExprOrOpArg],
        c: &Ctx,
        acts: Acts<'_>,
        k: &mut dyn FnMut(&mut Self, Acts<'_>) -> EvalResult<bool>,
    ) -> EvalResult<bool> {
        match rest.split_first() {
            None => k(self, acts),
            Some((first, tail)) => {
                let item = ActItem { pred: first.expect_expr(), ctx: c, kind: ActKind::Unchanged, next: acts };
                self.push_enabled_unchanged(tail, c, Acts::Cons(&item), k)
            }
        }
    }
}
