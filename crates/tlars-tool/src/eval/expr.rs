//! `eval*` / `evalAppl*` / `lookup` / `getOpContext` / `contexts`.
//!
//! `lookup` checks the `Ctx` chain first, then the spec's `Bindings` (TLC: context before
//! tool objects), so INSTANCE substitutions shadow the instantiated module's declarations.
//! Operator arguments and LET definitions become `Binding::Thunk` (call-by-name).

use super::{Ev, EvalControl};
use crate::ctx::{Binding, Ctx, Thunk};
use crate::error::{EvalError, EvalResult};
use crate::spec::OpBinding;
use crate::state::StateRef;
use crate::value::enumerate::{EnumOrder, ValueEnumeration};
use crate::value::{Closure, Except, Value};
use std::sync::Arc;
use tlars_msg::Ec;
use tlars_sany::Atom;
use tlars_sany::graph::{BoundSymbols, ExprId, ExprNode, ExprOrOpArg, OpDefId, OpDefKind, ParamId, SymbolRef};

/// What a name resolved to (`SymbolNodeValueLookupProvider.lookup`).
pub(crate) enum Resolved {
    Binding(Binding),
    Value(Value),
    UserOp(tlars_sany::graph::OpDefId),
    Native(crate::modules::NativeId),
    /// cfg `op(a, b) = v` (`OpRcdValue`, an `OpValue`).
    OpRcd(crate::spec::OpRcd),
    /// A state variable the state holds no value for, where `lookup` returns the
    /// `OpDeclNode` itself. It is the declaration, not a `vars[]` slot: ENABLED can reach one
    /// an instantiated module declares, which has no slot.
    Var(tlars_sany::graph::DeclId),
    /// A definition TLC overrides with a Java method tlars does not implement. Keeping it
    /// apart from `Undef` is what lets every caller stop as unimplemented instead of
    /// reporting an undefined operator. The level is the override's `minLevel`.
    Unimplemented {
        min_level: u8,
    },
    /// A theorem or assumption name, which `lookup` returns as the `ThmOrAssumpDefNode`
    /// itself; the evaluator expands its body like a definition's (`Tool.java:2050-2061`).
    Thm(tlars_sany::graph::ThmId),
}

impl<'t> Ev<'t> {
    /// `Tool.eval(expr, c, s0, s1, control)`.
    pub fn eval(&mut self, expr: ExprId, c: &Ctx, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> EvalResult<Value> {
        self.enter(expr)?;
        let r = self.eval_impl(expr, c, s0, s1, control);
        self.leave(r.as_ref().err().map(|e| &e.kind));
        r
    }

    /// `Tool.evalImpl` (`Tool.java:1795-1850`).
    fn eval_impl(&mut self, expr: ExprId, c: &Ctx, s0: StateRef<'_>, s1: StateRef<'_>, control: EvalControl) -> EvalResult<Value> {
        let graph = &self.tool().graph;
        match graph.expr(expr) {
            ExprNode::OpAppl { op, args, bound, .. } => self.eval_appl(expr, *op, args, bound.as_deref(), c, s0, s1, control),
            ExprNode::Label { body, .. } => self.eval(*body, c, s0, s1, control),
            ExprNode::LetIn { defs, body, .. } => {
                // `evalImplLetInKind`: 0-arity definitions become lazy values over the context
                // so far; the others are found through `lookup`.
                let mut c1 = c.clone();
                for &d in defs.iter() {
                    let def = graph.op_def(d);
                    if def.params.is_empty() {
                        let body = def.body.expect("a LET definition has a body");
                        c1 = c1.cons(SymbolRef::OpDef(d), Binding::Thunk(Thunk::new(body, c1.clone(), true)));
                    }
                }
                self.eval(*body, &c1, s0, s1, control)
            }
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                let mut c1 = c.clone();
                for sub in substs.iter() {
                    let val = self.get_val(sub.expr, c, true);
                    c1 = c1.cons(SymbolRef::Decl(sub.decl), val);
                }
                self.eval(*body, &c1, s0, s1, control)
            }
            ExprNode::Numeral { .. } | ExprNode::String { .. } => {
                Ok(self.tool().bindings.literal(expr).expect("processConstants bound every literal").clone())
            }
            ExprNode::At { .. } => Ok(c.except_at().expect("SANY builds an AtNode only inside an EXCEPT").clone()),
            ExprNode::Decimal { .. } => unreachable!("processConstants rejects decimals"),
        }
    }

    /// `Tool.eval(SemanticNode, ...)` on an argument node: an expression evaluates to a
    /// value, and an `OpArgNode` evaluates to what `lookup` finds for the operator it names,
    /// which TLC wraps in an `OpLambdaValue` (`evalImplOpArgKind`, `Tool.java:1888-1895`).
    /// tlars keeps an operator as a `Binding` instead, since no `Value` may hold one.
    pub(crate) fn eval_arg(
        &mut self,
        arg: ExprOrOpArg,
        c: &Ctx,
        s0: StateRef<'_>,
        s1: StateRef<'_>,
        control: EvalControl,
    ) -> EvalResult<Binding> {
        match arg {
            ExprOrOpArg::Expr(e) => Ok(Binding::Value(self.eval(e, c, s0, s1, control)?)),
            ExprOrOpArg::OpArg(_) => Ok(self.get_val(arg, c, false)),
            ExprOrOpArg::Absent => unreachable!("only CASE's OTHER guard is absent"),
        }
    }

    /// `getVal`: an expression argument is passed by name, an operator argument as what it
    /// names.
    fn get_val(&mut self, arg: ExprOrOpArg, c: &Ctx, cachable: bool) -> Binding {
        match arg {
            ExprOrOpArg::Expr(e) => Binding::Thunk(Thunk::new(e, c.clone(), cachable)),
            ExprOrOpArg::OpArg(sym) => match self.lookup(sym, c, StateRef::Empty, false) {
                Resolved::Binding(b) => b,
                Resolved::Value(v) => Binding::Value(v),
                Resolved::UserOp(d) => Binding::OpDef(d),
                Resolved::Native(id) => Binding::Native(id),
                Resolved::OpRcd(lines) => Binding::OpRcd(lines),
                // `lookup` returns the unimplemented `MethodValue` itself; passing it on as
                // a value makes the use site report it.
                Resolved::Var(_) | Resolved::Unimplemented { .. } => Binding::Value(Value::Undef),
                // TLC binds the `ThmOrAssumpDefNode` itself, which the use site then expands.
                // No binding carries a theorem, so the use site reports an undefined name.
                Resolved::Thm(_) => Binding::Value(Value::Undef),
            },
            ExprOrOpArg::Absent => unreachable!("only CASE's OTHER guard is absent"),
        }
    }

    /// `SymbolNodeValueLookupProvider.lookup` (`:87-121`) and `Spec.lookup` (`:450-466`): the
    /// context, then the symbol's tool object, then a definition body's (through INSTANCE
    /// substitutions).
    pub(crate) fn lookup(&mut self, sym: SymbolRef, c: &Ctx, s0: StateRef<'_>, cutoff: bool) -> Resolved {
        // `c.lookup(opNode, cutoff && isVarDecl)`: the cutoff applies to a variable only.
        let is_var_decl = matches!(sym, SymbolRef::Decl(d) if is_variable(self.tool(), d));
        if let Some(b) = c.lookup_cutoff(sym, cutoff && is_var_decl) {
            return Resolved::Binding(b.clone());
        }
        let tool = self.tool();
        match sym {
            SymbolRef::OpDef(d) => {
                if let Some(b) = tool.bindings.op_def(d) {
                    return resolved(b, d);
                }
                if tool.graph.op_def(d).kind == OpDefKind::User
                    && let Some(b) = tool.graph.op_def(d).body.and_then(|e| tool.bindings.body_chain(&tool.graph, e))
                {
                    return resolved(&b, d);
                }
                Resolved::UserOp(d)
            }
            SymbolRef::Decl(d) => match tool.bindings.decls[d.index()].as_ref() {
                Some(crate::spec::DeclBinding::Value(v)) => Resolved::Value(v.clone()),
                Some(crate::spec::DeclBinding::Table(lines)) => Resolved::OpRcd(lines.clone()),
                Some(crate::spec::DeclBinding::Op(op)) => Resolved::UserOp(*op),
                Some(crate::spec::DeclBinding::Native(id)) => Resolved::Native(*id),
                // `Spec.lookup(opNode, c, s, cutoff)` (`Spec.java:450-468`): a declaration
                // with no tool object is read from the state by name. An unbound one leaves
                // `lookup` returning the node itself.
                None => {
                    let name = tool.graph.decl(d).name;
                    match s0.get(name, tool.vars.slot_of(name)) {
                        Some(v) => Resolved::Value(v.clone()),
                        // `getVar` reads the node back only for a `VariableDeclKind` symbol
                        // (`SymbolNodeValueLookupProvider.java:66-81`), so a constant with no
                        // value stays what every other caller sees: nothing.
                        None if is_var_decl => Resolved::Var(d),
                        None => Resolved::Value(Value::Undef),
                    }
                }
            },
            // A formal parameter the context does not bind, which happens when TLC walks a
            // definition body without expanding its parameters (`processConfigSpec`'s level
            // check). `lookup` then returns the node itself, which resolves to nothing
            // (`SymbolNodeValueLookupProvider.java:86-118`).
            SymbolRef::Param(_) | SymbolRef::Module(_) => Resolved::Value(Value::Undef),
            SymbolRef::Thm(t) => Resolved::Thm(t),
        }
    }

    /// `Spec.getOpContext(ThmOrAssumpDefNode, ..)` (`Spec.java:485-496`): the same binding
    /// of parameters to thunks as a definition's.
    pub(crate) fn thm_context(&mut self, thm: tlars_sany::graph::ThmId, args: &[ExprOrOpArg], c: &Ctx) -> Ctx {
        let params = &self.tool().graph.thm(thm).params;
        let mut c1 = c.clone();
        for (&p, &arg) in params.iter().zip(args) {
            let val = self.get_val(arg, c, true);
            c1 = c1.cons(SymbolRef::Param(p), val);
        }
        c1
    }

    /// `getOpContext`: bind params to call-by-name thunks over `c`.
    pub(crate) fn op_context(&mut self, def: OpDefId, args: &[ExprOrOpArg], c: &Ctx, cachable: bool) -> Ctx {
        let params = &self.tool().graph.op_def(def).params;
        let mut c1 = c.clone();
        for (&p, &arg) in params.iter().zip(args) {
            let val = self.get_val(arg, c, cachable);
            c1 = c1.cons(SymbolRef::Param(p), val);
        }
        c1
    }

    /// `Tool.evalPure(opDef, args, ..)` (`Tool.java:1781-1785`): the definition's own body in
    /// the context its arguments bind, which is what an `@Evaluation` override that returned
    /// null falls back to. The `EvaluatingValue` holds the `OpDefNode`
    /// `processModuleOverrides` bound the method to, so a cfg `<-` override that points a
    /// declaration at such a method has no definition to fall back to.
    fn eval_pure(
        &mut self,
        op: SymbolRef,
        args: &[ExprOrOpArg],
        c: &Ctx,
        s0: StateRef<'_>,
        s1: StateRef<'_>,
        control: EvalControl,
    ) -> EvalResult<Value> {
        let SymbolRef::OpDef(d) = op else {
            let name = self.symbol_name(op);
            return Err(EvalError::unimplemented(format!("the TLA+ fallback of the Java method behind {name} (M3 unit 6)")));
        };
        let body = self.tool().graph.op_def(d).body.expect("an overridden definition has a body");
        let c1 = self.op_context(d, args, c, true);
        self.eval(body, &c1, s0, s1, control)
    }

    /// `Tool.evalApplImpl` (`Tool.java:1903-2870`) for the operators implemented so far.
    #[expect(clippy::too_many_arguments, reason = "the parameters of TLC's Tool.evalAppl*, plus the resolved operator")]
    fn eval_appl(
        &mut self,
        expr: ExprId,
        op: SymbolRef,
        args: &[ExprOrOpArg],
        bound: Option<&BoundSymbols>,
        c: &Ctx,
        s0: StateRef<'_>,
        s1: StateRef<'_>,
        control: EvalControl,
    ) -> EvalResult<Value> {
        let graph = &self.tool().graph;
        let mut builtin = self.builtin(op);
        if builtin.is_none() {
            match self.lookup(op, c, s0, control.has(EvalControl::PRIMED)) {
                Resolved::Binding(Binding::Thunk(t)) => return t.force(self, s0, s1, control),
                Resolved::Binding(Binding::Value(v)) | Resolved::Value(v) if !matches!(v, Value::Undef) => return Ok(v),
                Resolved::Binding(Binding::OpDef(d)) | Resolved::UserOp(d) => {
                    let def = graph.op_def(d);
                    if def.kind == OpDefKind::BuiltIn {
                        builtin = self.builtin(SymbolRef::OpDef(d));
                    } else {
                        let body = def.body.expect("a user definition has a body");
                        let c1 = self.op_context(d, args, c, true);
                        return self.eval(body, &c1, s0, s1, control);
                    }
                }
                Resolved::Native(id) | Resolved::Binding(Binding::Native(id)) => {
                    // A `MethodValue`'s method handle is invoked with exactly the Java
                    // method's parameters, so a cfg override that gives it another arity
                    // raises a `WrongMethodTypeException` whose text names the handle's
                    // erased type. tlars has no table of those types.
                    if args.len() != crate::modules::arity(id) {
                        let name = self.symbol_name(op);
                        return Err(EvalError::unimplemented(format!("the Java method behind {name} applied to {} arguments", args.len())));
                    }
                    // `MethodValue.eval(tool, args, ...)`: the arguments are evaluated first,
                    // except where the Java method takes an operator and tlars has to see the
                    // argument node to resolve it.
                    return match crate::modules::imp(id) {
                        crate::modules::NativeImpl::Lazy(f) => match f(self, args, c, s0, s1, control)? {
                            Some(v) => Ok(v),
                            // `EvaluatingValue.eval`'s null branch: `tool.evalPure(opDef, ..)`,
                            // the operator's own TLA+ body in the context its arguments bind.
                            // `opDef` is the definition `processModuleOverrides` bound the
                            // method to, which is the one this application names.
                            None => self.eval_pure(op, args, c, s0, s1, control),
                        },
                        crate::modules::NativeImpl::Strict(_) => {
                            let mut vals = Vec::with_capacity(args.len());
                            for &a in args {
                                vals.push(self.eval(a.expect_expr(), c, s0, s1, control)?);
                            }
                            crate::modules::apply(self, id, &vals)
                        }
                    };
                }
                // `OpValue.eval(tool, args, ...)`: evaluate the arguments, then match a line.
                Resolved::OpRcd(lines) | Resolved::Binding(Binding::OpRcd(lines)) => {
                    let mut vals = Vec::with_capacity(args.len());
                    for &a in args {
                        vals.push(self.eval(a.expect_expr(), c, s0, s1, control)?);
                    }
                    return self.apply_op_rcd(&lines, &vals);
                }
                Resolved::Binding(Binding::Lambda(_) | Binding::ExceptAt(_)) => {
                    return Err(EvalError::unimplemented("an operator argument bound to a LAMBDA (M2 unit 4)".into()));
                }
                Resolved::Unimplemented { .. } => {
                    let name = self.symbol_name(op);
                    return Err(EvalError::unimplemented(format!("the Java implementation of {name}")));
                }
                Resolved::Thm(t) => {
                    let body = graph.thm(t).body;
                    let c1 = self.thm_context(t, args, c);
                    return self.eval(body, &c1, s0, s1, control);
                }
                Resolved::Binding(Binding::Value(_)) | Resolved::Value(_) | Resolved::Var(_) => {
                    // Fairness on a sub-action that leaves a subscript variable unassigned
                    // (`Tool.java:2064-2090`).
                    if !control.has(EvalControl::ENABLED) && control.has(EvalControl::PRIMED) && matches!(op, SymbolRef::Decl(_)) {
                        let name = self.symbol_name(op);
                        let text = graph.expr_tlc_text(expr);
                        let message = self.out().text(Ec::TLC_STATE_NOT_COMPLETELY_SPECIFIED_LIVE, &[&name, &text]);
                        return Err(EvalError::runtime(Ec::TLC_STATE_NOT_COMPLETELY_SPECIFIED_LIVE, message));
                    }
                    // `Assert.fail(TLC_CONFIG_UNDEFINED_OR_NO_OPERATOR, ...)` (`Tool.java:2103`).
                    let name = self.symbol_name(op);
                    let text = graph.expr_tlc_text(expr);
                    let message = self.out().text(Ec::TLC_CONFIG_UNDEFINED_OR_NO_OPERATOR, &[&name, &text]);
                    return Err(EvalError::runtime(Ec::TLC_CONFIG_UNDEFINED_OR_NO_OPERATOR, message));
                }
            }
        }
        let Some(builtin) = builtin else { unreachable!("resolved above") };
        let text = |ev: &Ev<'_>, e: ExprId| ev.tool().graph.expr_tlc_text(e);
        let arg = |i: usize| args[i].expect_expr();
        let not_boolean = |ev: &mut Ev<'_>, reason: String, at: ExprId| -> EvalError {
            let loc = ev.tool().graph.loc(at);
            EvalError::detailed(reason, Some(loc))
        };
        match builtin {
            Builtin::ConjList | Builtin::DisjList => {
                let conj = builtin == Builtin::ConjList;
                for i in 0..args.len() {
                    let v = self.eval(arg(i), c, s0, s1, control)?;
                    let Value::Bool(b) = v else {
                        let what = if conj { "conjunction" } else { "disjunction" };
                        let reason = format!(
                            "A non-boolean expression ({}) was used as a formula in a {what}.\n{}",
                            v.kind_string(),
                            text(self, arg(i))
                        );
                        return Err(not_boolean(self, reason, arg(i)));
                    };
                    if b != conj {
                        return Ok(Value::Bool(b));
                    }
                }
                Ok(Value::Bool(conj))
            }
            Builtin::Land | Builtin::Lor | Builtin::Implies => {
                let form = match builtin {
                    Builtin::Land => "P /\\ Q",
                    Builtin::Lor => "P \\/ Q",
                    _ => "P => Q",
                };
                let v1 = self.eval(arg(0), c, s0, s1, control)?;
                let Value::Bool(b1) = v1 else {
                    let reason = format!(
                        "Attempted to evaluate an expression of form {form} when P was\n{}.\n{}",
                        v1.kind_string(),
                        text(self, expr)
                    );
                    return Err(not_boolean(self, reason, expr));
                };
                // `/\` and `=>` need Q only when P holds, `\/` only when it does not.
                let short = match builtin {
                    Builtin::Land => (!b1).then_some(false),
                    Builtin::Lor => b1.then_some(true),
                    _ => (!b1).then_some(true),
                };
                if let Some(r) = short {
                    return Ok(Value::Bool(r));
                }
                let v2 = self.eval(arg(1), c, s0, s1, control)?;
                if !matches!(v2, Value::Bool(_)) {
                    let reason = format!(
                        "Attempted to evaluate an expression of form {form} when Q was\n{}.\n{}",
                        v2.kind_string(),
                        text(self, expr)
                    );
                    return Err(not_boolean(self, reason, expr));
                }
                Ok(v2)
            }
            Builtin::Equiv => {
                let v1 = self.eval(arg(0), c, s0, s1, control)?;
                let v2 = self.eval(arg(1), c, s0, s1, control)?;
                match (v1, v2) {
                    (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
                    _ => {
                        let reason = format!(
                            "Attempted to evaluate an expression of form P <=> Q when P or Q was not a boolean.\n{}",
                            text(self, expr)
                        );
                        Err(not_boolean(self, reason, expr))
                    }
                }
            }
            Builtin::Lnot => {
                let v = self.eval(arg(0), c, s0, s1, control)?;
                match v {
                    Value::Bool(b) => Ok(Value::Bool(!b)),
                    _ => {
                        let reason =
                            format!("Attempted to apply the operator ~ to a non-boolean\n({})\n{}", v.kind_string(), text(self, expr));
                        Err(not_boolean(self, reason, arg(0)))
                    }
                }
            }
            Builtin::Eq | Builtin::Noteq => {
                let v1 = self.eval(arg(0), c, s0, s1, control)?;
                let v2 = self.eval(arg(1), c, s0, s1, control)?;
                let eq = v1.tlc_equals(self, &v2)?;
                Ok(Value::Bool(eq == (builtin == Builtin::Eq)))
            }
            Builtin::In | Builtin::Notin => {
                let v1 = self.eval(arg(0), c, s0, s1, control)?;
                let v2 = self.eval(arg(1), c, s0, s1, control)?;
                let member = v2.member(self, &v1)?;
                Ok(Value::Bool(member == (builtin == Builtin::In)))
            }
            Builtin::IfThenElse => {
                let v = self.eval(arg(0), c, s0, s1, control)?;
                match v {
                    Value::Bool(true) => self.eval(arg(1), c, s0, s1, control),
                    Value::Bool(false) => self.eval(arg(2), c, s0, s1, control),
                    _ => {
                        let reason = format!(
                            "A non-boolean expression ({}) was used as the condition of an IF.\n{}",
                            v.kind_string(),
                            text(self, expr)
                        );
                        Err(not_boolean(self, reason, expr))
                    }
                }
            }
            Builtin::Case => {
                let mut other = None;
                for a in args {
                    let pair = a.expect_expr();
                    let ExprNode::OpAppl { args: pair_args, .. } = graph.expr(pair) else { unreachable!("CASE arms are $Pair") };
                    match pair_args[0] {
                        ExprOrOpArg::Absent => other = Some(pair_args[1].expect_expr()),
                        guard => {
                            let guard = guard.expect_expr();
                            let v = self.eval(guard, c, s0, s1, control)?;
                            match v {
                                Value::Bool(true) => return self.eval(pair_args[1].expect_expr(), c, s0, s1, control),
                                Value::Bool(false) => {}
                                _ => {
                                    let reason = format!(
                                        "A non-boolean expression ({}) was used as a condition of a CASE. {}",
                                        v.kind_string(),
                                        text(self, guard)
                                    );
                                    return Err(not_boolean(self, reason, guard));
                                }
                            }
                        }
                    }
                }
                match other {
                    Some(e) => self.eval(e, c, s0, s1, control),
                    None => {
                        let reason = format!("Attempted to evaluate a CASE with no conditions true.\n{}", text(self, expr));
                        Err(not_boolean(self, reason, expr))
                    }
                }
            }
            Builtin::BoundedForall | Builtin::BoundedExists => {
                let forall = builtin == Builtin::BoundedForall;
                let bound = bound.expect("a bounded quantifier has bounds");
                let mut contexts = self.contexts(bound, c, s0, s1, control)?;
                let body = arg(0);
                while let Some(c1) = contexts.next(self)? {
                    let v = self.eval(body, &c1, s0, s1, control)?;
                    let Value::Bool(b) = v else {
                        // `Assert.fail(TLC_EXPECTED_VALUE, {"boolean", expr}, body, c1)`.
                        let message = self.out().text(Ec::TLC_EXPECTED_VALUE, &["boolean", &text(self, expr)]);
                        return Err(EvalError::runtime(Ec::TLC_EXPECTED_VALUE, message).with_loc(Some(graph.loc(body))));
                    };
                    if b != forall {
                        return Ok(Value::Bool(b));
                    }
                }
                Ok(Value::Bool(forall))
            }
            Builtin::BoundedChoose => {
                let bound = bound.expect("a bounded CHOOSE has a bound");
                let list = &bound.lists[0];
                let in_val = self.eval(list.domain, c, s0, s1, control)?;
                if !in_val.is_enumerable() {
                    let reason = format!(
                        "Attempted to compute the value of an expression of\nform CHOOSE x \\in S: P, but S was not enumerable.\n{}",
                        text(self, expr)
                    );
                    return Err(not_boolean(self, reason, expr));
                }
                in_val.normalize(self)?;
                let mut elems = in_val.elements(self, EnumOrder::Normalized)?.expect("an enumerable value enumerates");
                let pred = arg(0);
                while let Some(val) = elems.next(self)? {
                    let mut c1 = c.clone();
                    if list.tuple {
                        let cnt = list.vars.len();
                        let tuple = val.to_tuple(self)?.filter(|t| t.elems.len() == cnt);
                        let Some(tuple) = tuple else {
                            let reason = format!(
                                "Attempted to compute the value of an expression of form\nCHOOSE <<x1, ... , xN>> \\in S: P, but S was not a set\nof N-tuples.\n{}",
                                text(self, expr)
                            );
                            return Err(not_boolean(self, reason, expr));
                        };
                        for (&id, e) in list.vars.iter().zip(tuple.elems.iter()) {
                            c1 = c1.cons(SymbolRef::Param(id), Binding::Value(e.clone()));
                        }
                    } else {
                        c1 = c1.cons(SymbolRef::Param(list.vars[0]), Binding::Value(val.clone()));
                    }
                    let bval = self.eval(pred, &c1, s0, s1, control)?;
                    let Value::Bool(b) = bval else {
                        let message = self.out().text(Ec::TLC_EXPECTED_VALUE, &["boolean", &text(self, expr)]);
                        return Err(EvalError::runtime(Ec::TLC_EXPECTED_VALUE, message).with_loc(Some(graph.loc(pred))));
                    };
                    if b {
                        return Ok(val);
                    }
                }
                let reason = format!(
                    "Attempted to compute the value of an expression of form\nCHOOSE x \\in S: P, but no element of S satisfied P.\n{}",
                    text(self, expr)
                );
                Err(not_boolean(self, reason, expr))
            }
            Builtin::UnboundedChoose | Builtin::UnboundedExists | Builtin::UnboundedForall => {
                let what = match builtin {
                    Builtin::UnboundedChoose => ("CHOOSE", "CHOOSE x \\in S: P(x)"),
                    Builtin::UnboundedExists => ("\\E", "\\E x \\in S: P(x)"),
                    _ => ("\\A", "\\A x \\in S: P(x)"),
                };
                let reason = format!(
                    "TLC attempted to evaluate an unbounded {}.\nMake sure that the expression is of form {}.\n{}",
                    what.0,
                    what.1,
                    text(self, expr)
                );
                Err(not_boolean(self, reason, expr))
            }
            // `{x \in S : P}`: filtered eagerly over a Reducible S, kept as a SetPredValue
            // otherwise (`Tool.java:2466-2512`).
            Builtin::SubsetOf => {
                let bound = bound.expect("a set filter has a bound");
                let list = &bound.lists[0];
                let in_val = self.eval(list.domain, c, s0, s1, control)?;
                let pred = arg(0);
                if !in_val.is_reducible() {
                    // `CallStackTool.eval` replaces a `SetPredValue` with a fresh copy that
                    // carries no source, so sourcing it here would never be observed.
                    return Ok(Value::SetPred(Arc::new(crate::value::SetPredValue {
                        vars: list.vars.clone(),
                        tuple: list.tuple,
                        inner: in_val,
                        pred,
                        env: Closure::capture(c, s0, s1, control)?,
                        converted: std::sync::OnceLock::new(),
                        source: crate::value::Source::none(),
                    })));
                }
                let is_norm = in_val.is_normalized();
                let mut elems = in_val.elements(self, EnumOrder::Native)?.expect("a Reducible value enumerates");
                let mut vals = crate::value::ValueVec::new(self);
                while let Some(elem) = elems.next(self)? {
                    let mut c1 = c.clone();
                    if list.tuple {
                        // TLC casts the element to a tuple and reads its components without
                        // checking the length (`Tool.java:2475-2478`).
                        let Value::Tuple(t) = &elem else { return Err(EvalError::class_cast(elem.java_class(), "TupleValue")) };
                        for (i, &id) in list.vars.iter().enumerate() {
                            let Some(v) = t.elems.get(i) else { return Err(EvalError::index_out_of_bounds(i, t.elems.len())) };
                            c1 = c1.cons(SymbolRef::Param(id), Binding::Value(v.clone()));
                        }
                    } else {
                        c1 = c1.cons(SymbolRef::Param(list.vars[0]), Binding::Value(elem.clone()));
                    }
                    let bval = self.eval(pred, &c1, s0, s1, control)?;
                    let Value::Bool(b) = bval else {
                        let reason = format!(
                            "Attempted to evaluate an expression of form {{x \\in S : P(x)}} when P was {}.\n{}",
                            bval.kind_string(),
                            text(self, pred)
                        );
                        return Err(not_boolean(self, reason, pred));
                    };
                    if b {
                        vals.push(elem)?;
                    }
                }
                Ok(self.set_source(expr, Value::set_enum_of(vals.into_vec(), is_norm)))
            }
            // `{e : x \in S}`: always eager, and the result keeps the order the body
            // produced (`Tool.java:2436-2449`).
            Builtin::SetOfAll => {
                let bound = bound.expect("a set map has a bound");
                let mut contexts = self.contexts(bound, c, s0, s1, control)?;
                let body = arg(0);
                let mut vals = crate::value::ValueVec::new(self);
                while let Some(c1) = contexts.next(self)? {
                    let v = self.eval(body, &c1, s0, s1, control)?;
                    vals.push(v)?;
                }
                Ok(self.set_source(expr, Value::set_enum(vals.into_vec())))
            }
            // The eager form is the receiver's, so TLC swaps the operands when only the
            // right one reduces: `SUBSET {} \cup {1}` is built from `{1}`
            // (`Tool.java:2673-2719`).
            Builtin::Cup | Builtin::Cap | Builtin::Setdiff => {
                let a1 = self.eval(arg(0), c, s0, s1, control)?;
                let a2 = self.eval(arg(1), c, s0, s1, control)?;
                let result = match builtin {
                    Builtin::Cup if a1.is_reducible() => a1.cup(self, &a2),
                    Builtin::Cup if a2.is_reducible() => a2.cup(self, &a1),
                    Builtin::Cup => Ok(crate::value::lazy_cup(a1, a2)),
                    Builtin::Cap if a1.is_reducible() => a1.cap(self, &a2),
                    Builtin::Cap if a2.is_reducible() => a2.cap(self, &a1),
                    Builtin::Cap => Ok(crate::value::lazy_cap(a1, a2)),
                    // Only the left side of `\` may reduce: the right one is not enumerated.
                    _ if a1.is_reducible() => a1.diff(self, &a2),
                    _ => Ok(crate::value::lazy_diff(a1, a2)),
                };
                result.map(|v| self.set_source(expr, v))
            }
            Builtin::Subseteq => {
                let a1 = self.eval(arg(0), c, s0, s1, control)?;
                let a2 = self.eval(arg(1), c, s0, s1, control)?;
                if !a1.is_enumerable() {
                    let reason = format!(
                        "Attempted to evaluate an expression of form S \\subseteq T, but S was not enumerable.\n{}",
                        text(self, expr)
                    );
                    return Err(not_boolean(self, reason, expr));
                }
                Ok(Value::Bool(a1.is_subset_eq(self, &a2)?))
            }
            Builtin::Subset => {
                let v = self.eval(arg(0), c, s0, s1, control)?;
                Ok(self.set_source(expr, crate::value::subset(v)))
            }
            Builtin::Union => {
                let v = self.eval(arg(0), c, s0, s1, control)?;
                v.union(self).map(|u| self.set_source(expr, u))
            }
            Builtin::CartesianProd => {
                let mut sets = Vec::with_capacity(args.len());
                for i in 0..args.len() {
                    sets.push(self.eval(arg(i), c, s0, s1, control)?);
                }
                Ok(self.set_source(expr, crate::value::set_of_tuples(sets)))
            }
            Builtin::SetOfFcns => {
                let domain = self.eval(arg(0), c, s0, s1, control)?;
                let range = self.eval(arg(1), c, s0, s1, control)?;
                Ok(self.set_source(expr, crate::value::set_of_fcns(domain, range)))
            }
            Builtin::SetOfRcds => {
                let mut names = Vec::with_capacity(args.len());
                let mut sets = Vec::with_capacity(args.len());
                for i in 0..args.len() {
                    let (name, set) = pair(graph, arg(i));
                    names.push(self.field_name(name));
                    sets.push(self.eval(set, c, s0, s1, control)?);
                }
                crate::value::set_of_rcds(self, names, sets).map(|v| self.set_source(expr, v))
            }
            Builtin::SetEnumerate => {
                let mut elems = Vec::with_capacity(args.len());
                for i in 0..args.len() {
                    elems.push(self.eval(arg(i), c, s0, s1, control)?);
                }
                Ok(self.set_source(expr, Value::set_enum(elems)))
            }
            Builtin::Tuple => {
                let mut elems = Vec::with_capacity(args.len());
                for i in 0..args.len() {
                    elems.push(self.eval(arg(i), c, s0, s1, control)?);
                }
                Ok(self.set_source(expr, Value::tuple(elems)))
            }
            Builtin::RcdConstructor => {
                let mut names = Vec::with_capacity(args.len());
                let mut vals = Vec::with_capacity(args.len());
                for i in 0..args.len() {
                    let (name, value) = pair(graph, arg(i));
                    names.push(self.field_name(name));
                    vals.push(self.eval(value, c, s0, s1, control)?);
                }
                Ok(self.set_source(expr, Value::Record(Value::record_arc(names, vals, false))))
            }
            Builtin::RcdSelect => {
                let rval = self.eval(arg(0), c, s0, s1, control)?;
                let sval = Value::Str(self.field_name(arg(1)));
                // The field is only shown in the errors; a string prints without failing.
                if matches!(rval, Value::Record(_)) {
                    return match rval.select(self, &sval)? {
                        Some(v) => Ok(v),
                        None => {
                            let field = sval.to_tlc_string(self)?;
                            let shown = rval.ppr(self)?;
                            let reason =
                                format!("Attempted to select nonexistent field {field} from the record\n{shown}\n{}", text(self, expr));
                            Err(not_boolean(self, reason, expr))
                        }
                    };
                }
                let Some(fcn) = rval.to_fcn_rcd(self)? else {
                    let field = sval.to_tlc_string(self)?;
                    let shown = rval.ppr(self)?;
                    let reason = format!("Attempted to select field {field} from a non-record value {shown}\n{}", text(self, expr));
                    return Err(not_boolean(self, reason, expr));
                };
                Value::FcnRcd(fcn).apply(self, &sval, control)
            }
            Builtin::FcnApply => {
                // The function itself is evaluated with KeepLazy, so `[x \in S |-> e][a]`
                // evaluates the body at `a` only (`Tool.java:2328-2352`).
                let fval = self.eval(arg(0), c, s0, s1, control.with(EvalControl::KEEP_LAZY))?;
                if !fval.is_function() {
                    let reason = format!("A non-function ({}) was applied as a function.\n{}", fval.kind_string(), text(self, expr));
                    return Err(not_boolean(self, reason, expr));
                }
                let argv = self.eval(arg(1), c, s0, s1, control)?;
                fval.apply(self, &argv, control)
            }
            Builtin::Domain => {
                let v = self.eval(arg(0), c, s0, s1, control)?;
                if !v.is_function() {
                    let reason =
                        format!("Attempted to apply the operator DOMAIN to a non-function\n({})\n{}", v.kind_string(), text(self, expr));
                    return Err(not_boolean(self, reason, expr));
                }
                v.get_domain(self).map(|d| self.set_source(expr, d))
            }
            Builtin::FcnConstructor | Builtin::RecursiveFcnSpec => {
                let bound = bound.expect("a function constructor has bounds");
                let mut params = Vec::with_capacity(bound.lists.len());
                let mut is_fcn_rcd = true;
                for list in bound.lists.iter() {
                    let dval = self.eval(list.domain, c, s0, s1, control)?;
                    is_fcn_rcd = is_fcn_rcd && dval.is_reducible();
                    params.push((list.vars.clone(), list.tuple, dval));
                }
                // `makeRecursive` both binds the function's own name in its context and sets
                // KeepLazy, so a recursive definition is never materialised eagerly.
                let recursive = builtin == Builtin::RecursiveFcnSpec;
                let rec_name = recursive.then(|| SymbolRef::Param(bound.unbounded[0]));
                let lambda_control = if recursive { control.with(EvalControl::KEEP_LAZY) } else { control };
                let lambda = Arc::new(crate::value::FcnLambdaValue {
                    params: params.into_boxed_slice(),
                    body: arg(0),
                    env: Closure::capture(c, s0, s1, lambda_control)?,
                    rec_name,
                    excepts: Box::new([]),
                    converted: std::sync::OnceLock::new(),
                    source: crate::value::Source::none(),
                });
                // The lambda carries its source while `evalAppl` still holds it, so the
                // eager conversion below wraps a failure; `CallStackTool.eval` then hands
                // back a fresh copy, which has none.
                let value = self.set_source(expr, Value::FcnLambda(lambda.clone()));
                if is_fcn_rcd && !recursive && !control.has(EvalControl::KEEP_LAZY) {
                    let converted = value.to_fcn_rcd(self);
                    value.clear_source();
                    return Ok(Value::FcnRcd(converted?.expect("a function lambda converts")));
                }
                value.clear_source();
                Ok(value)
            }
            Builtin::Except => {
                let mut result = self.eval(arg(0), c, s0, s1, control)?;
                for i in 1..args.len() {
                    let (lhs_node, rhs_node) = pair(graph, arg(i));
                    let ExprNode::OpAppl { args: cmpts, .. } = graph.expr(lhs_node) else { unreachable!("an EXCEPT path is $Seq") };
                    let mut lhs = Vec::with_capacity(cmpts.len());
                    for &cmpt in cmpts.iter() {
                        lhs.push(self.eval(cmpt.expect_expr(), c, s0, s1, control)?);
                    }
                    match result.select_path(self, &lhs)? {
                        None => {
                            let shown = text(self, arg(0));
                            crate::value::warn(self, Ec::TLC_EXCEPT_APPLIED_TO_UNKNOWN_FIELD, &[&shown])?;
                        }
                        Some(at_val) => {
                            let c1 = c.cons_except_at(at_val);
                            let rhs = self.eval(rhs_node, &c1, s0, s1, control)?;
                            let mut ex = Except::new(lhs, rhs);
                            result = result.take_except(self, &mut ex)?;
                        }
                    }
                }
                Ok(result)
            }
            // `x'` reads the successor: s1 becomes the state, and there is no successor of a
            // successor (`Tool.java:2720-2736`).
            Builtin::Prime => self.eval(arg(0), c, s1, StateRef::Null, control.set_primed_if_enabled()),
            Builtin::Unchanged => {
                let v0 = self.eval(arg(0), c, s0, StateRef::Empty, control)?;
                let v1 = self.eval(arg(0), c, s1, StateRef::Null, control.set_primed_if_enabled())?;
                Ok(Value::Bool(v0.tlc_equals(self, &v1)?))
            }
            Builtin::AngleAct | Builtin::SquareAct => {
                let angle = builtin == Builtin::AngleAct;
                let form = if angle { "<A>_e" } else { "[A]_e" };
                let v = self.eval(arg(0), c, s0, s1, control)?;
                let Value::Bool(b) = v else {
                    let reason =
                        format!("Attempted to evaluate an expression of form {form}, but A was not a boolean.\n{}", text(self, expr));
                    return Err(not_boolean(self, reason, expr));
                };
                // `<A>_e` is false and `[A]_e` true as soon as A settles it.
                if b == angle {
                    return Ok(Value::Bool(angle));
                }
                let v0 = self.eval(arg(1), c, s0, StateRef::Empty, control)?;
                let v1 = self.eval(arg(1), c, s1, StateRef::Null, control.set_primed_if_enabled())?;
                let same = v0.tlc_equals(self, &v1)?;
                Ok(Value::Bool(same != angle))
            }
            Builtin::Enabled => {
                let c1 = c.branch();
                let enabled = self.enabled(arg(0), super::next::Acts::Empty, &c1, s0, &crate::state::FunState::EMPTY)?;
                Ok(Value::Bool(enabled))
            }
            Builtin::Nop => self.eval(arg(0), c, s0, s1, control),
            Builtin::Cdot => Err(super::enabled::cdot_unsupported(self, expr)),
            Builtin::Box | Builtin::TemporalExists | Builtin::TemporalForall => {
                Err(EvalError::unimplemented("a temporal operator outside PROPERTY (M4)".into()))
            }
            Builtin::Unimplemented(name) => Err(EvalError::unimplemented(format!("the operator {name} (M2 unit 4)"))),
        }
    }

    /// The `StringValue` `processConstants` left on a record field name or an `r.f` selector.
    fn field_name(&self, e: ExprId) -> Atom {
        match self.tool().bindings.literal(e) {
            Some(Value::Str(a)) => *a,
            _ => unreachable!("processConstants bound every field name"),
        }
    }

    /// `OpRcdValue.eval(args, control)` (`OpRcdValue.java:130-162`): the first line all of
    /// whose arguments are equal to the actual ones, compared with the *line's* value as the
    /// receiver.
    fn apply_op_rcd(&mut self, lines: &crate::spec::OpRcd, args: &[Value]) -> EvalResult<Value> {
        for (params, value) in lines.iter() {
            if params.len() != args.len() {
                let flat = crate::value::print::op_rcd_string(self, lines)?;
                let shown = crate::value::print::ppr(self, &flat);
                return Err(EvalError::detailed(format!("Attempted to apply the operator {shown}\nwith wrong number of arguments."), None));
            }
            let mut matched = true;
            for (p, a) in params.iter().zip(args) {
                matched = p.tlc_equals(self, a)?;
                if !matched {
                    break;
                }
            }
            if matched {
                return Ok(value.clone());
            }
        }
        let flat = crate::value::print::op_rcd_string(self, lines)?;
        let shown = crate::value::print::ppr(self, &flat);
        let mut args_text = String::new();
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                args_text.push_str(", ");
            }
            args_text.push_str(&a.to_tlc_string(self)?);
        }
        Err(EvalError::detailed(format!("Attempted to apply operator:\n{shown}\nto arguments ({args_text}), which is undefined."), None))
    }

    fn body_binding(&self, d: OpDefId) -> Option<OpBinding> {
        let tool = self.tool();
        tool.graph.op_def(d).body.and_then(|b| tool.bindings.body_chain(&tool.graph, b))
    }

    pub(crate) fn symbol_name(&self, sym: SymbolRef) -> String {
        let graph = &self.tool().graph;
        let name = match sym {
            SymbolRef::OpDef(d) => graph.op_def(d).name,
            SymbolRef::Decl(d) => graph.decl(d).name,
            SymbolRef::Param(p) => graph.param(p).name,
            SymbolRef::Thm(t) => graph.thm(t).name,
            SymbolRef::Module(m) => graph.module(m).name,
        };
        graph.interner().text(name).to_string()
    }

    /// `evalImplOpArgKind` (`Tool.java:1884-1892`): an operator argument evaluated as a
    /// value, which TLC wraps in an `OpLambdaValue` carrying the context of the *use* site.
    /// `None` = the argument does not name an operator.
    pub(crate) fn op_arg_value<'s>(
        &mut self,
        arg: ExprOrOpArg,
        c: &Ctx,
        s0: StateRef<'s>,
        s1: StateRef<'s>,
        control: EvalControl,
    ) -> EvalResult<Option<OpArgValue<'s>>> {
        let ExprOrOpArg::OpArg(sym) = arg else { return Ok(None) };
        Ok(match self.lookup(sym, c, StateRef::Empty, false) {
            Resolved::Binding(Binding::OpDef(d)) | Resolved::UserOp(d) => Some(OpArgValue::Op { def: d, ctx: c.clone(), s0, s1, control }),
            Resolved::Binding(Binding::Native(id)) | Resolved::Native(id) => Some(OpArgValue::Native(id)),
            _ => None,
        })
    }

    /// `evalImplOpArgKind`'s `return (Value) val` (`Tool.java:1888-1895`): an operator
    /// argument whose symbol does not resolve to a definition evaluates to whatever value
    /// `lookup` found. `lookup` reads the tool object of the definition's *body*, so
    /// `LAMBDA e : 1` resolves to the literal `1` and a module that wants an operator
    /// reports that value. `None` where the argument is an operator after all.
    pub(crate) fn op_arg_as_value(&mut self, arg: ExprOrOpArg, c: &Ctx) -> EvalResult<Option<Value>> {
        let ExprOrOpArg::OpArg(sym) = arg else { return Ok(None) };
        Ok(match self.lookup(sym, c, StateRef::Empty, false) {
            Resolved::Binding(Binding::Value(v)) | Resolved::Value(v) => Some(v),
            _ => None,
        })
    }

    /// `Values.ppr(op.toString())` of an operator argument: `<Operator name>` for a
    /// definition, which for a LAMBDA is named `LAMBDA`, and the Java signature for an
    /// override (`OpLambdaValue.java:189-193`, `MethodValue.java:249-252`).
    pub(crate) fn op_arg_text(&mut self, arg: ExprOrOpArg, op: Option<&OpArgValue<'_>>) -> EvalResult<String> {
        if let Some(OpArgValue::Native(id)) = op {
            return Ok(format!("<Java Method: {}>", id.op().signature));
        }
        let ExprOrOpArg::OpArg(sym) = arg else {
            return Err(EvalError::unimplemented("the printed form of a non-operator module argument".into()));
        };
        Ok(format!("<Operator {}>", self.symbol_name(sym)))
    }

    /// `OpLambdaValue.eval(args, control)` (`OpLambdaValue.java:100-119`) and
    /// `MethodValue.eval(args)`: the body runs in the captured context and states, and the
    /// captured control contributes only its Enabled bit.
    pub(crate) fn apply_operator(&mut self, op: &OpArgValue<'_>, args: &[Value], control: EvalControl) -> EvalResult<Value> {
        match op {
            OpArgValue::Native(id) => crate::modules::apply(self, *id, args),
            OpArgValue::Op { def, ctx, s0, s1, control: captured } => {
                let def = *def;
                let params = &self.tool().graph.op_def(def).params;
                let body = self.tool().graph.op_def(def).body.expect("a definition has a body");
                let mut c1 = ctx.clone();
                for (&p, v) in params.iter().zip(args) {
                    c1 = c1.cons(SymbolRef::Param(p), Binding::Value(v.clone()));
                }
                let control = if captured.has(EvalControl::ENABLED) { control.with(EvalControl::ENABLED) } else { control };
                self.eval(body, &c1, *s0, *s1, control)
            }
        }
    }

    /// `BuiltInOPs.getOpCode(name)` for a built-in operator; `None` for everything else
    /// (user definitions, and built-ins such as TRUE that TLC binds as values).
    #[inline]
    fn builtin(&self, op: SymbolRef) -> Option<Builtin> {
        let SymbolRef::OpDef(d) = op else { return None };
        self.tool().builtins[d.index()]
    }

    /// `Tool.contexts` (`Tool.java:4080-4110`): every domain is evaluated first; then the
    /// bindings run like an odometer with the first variable fastest.
    pub(crate) fn contexts(
        &mut self,
        bound: &BoundSymbols,
        c: &Ctx,
        s0: StateRef<'_>,
        s1: StateRef<'_>,
        control: EvalControl,
    ) -> EvalResult<Contexts> {
        let graph = &self.tool().graph;
        let mut vars = Vec::new();
        for list in bound.lists.iter() {
            let set = self.eval(list.domain, c, s0, s1, control)?;
            // One enumeration per bound variable, and a `<<x, y>> \in S` list takes one.
            let repeats = if list.tuple { 1 } else { list.vars.len() };
            for i in 0..repeats {
                match set.elements(self, EnumOrder::Normalized)? {
                    Some(e) => {
                        let bound = if list.tuple { Bound::Tuple(list.vars.clone()) } else { Bound::Var(list.vars[i]) };
                        vars.push((bound, e));
                    }
                    None => {
                        let reason = set.non_enumerable_message(self, &graph.expr_tlc_text(list.domain))?;
                        return Err(EvalError::detailed(reason, Some(graph.loc(list.domain))));
                    }
                }
            }
        }
        let mut current = Vec::with_capacity(vars.len());
        let mut done = false;
        for (_, e) in vars.iter_mut() {
            match e.next(self)? {
                Some(v) => current.push(v),
                None => {
                    done = true;
                    break;
                }
            }
        }
        Ok(Contexts { base: c.clone(), vars, current, done })
    }

    /// `BuiltInOPs.getOpCode(opNode.getName())` of an applied operator; `None` for a
    /// user-defined one.
    pub(crate) fn opcode(&self, op: SymbolRef) -> Option<Builtin> {
        self.builtin(op)
    }

    /// `getNextStatesApplEvalAppl` / the init and next `OpValue` branch: an `OpValue` is
    /// applied to the arguments, and anything else is the value itself. The variants
    /// `next_appl` and `init_appl` handle before this are not reached.
    #[expect(clippy::too_many_arguments, reason = "the parameters of TLC's Tool.evalAppl*, plus the resolved operator")]
    pub(crate) fn apply_resolved(
        &mut self,
        resolved: Resolved,
        op: SymbolRef,
        expr: ExprId,
        args: &[ExprOrOpArg],
        c: &Ctx,
        s0: StateRef<'_>,
        s1: StateRef<'_>,
        control: EvalControl,
    ) -> EvalResult<Value> {
        match resolved {
            Resolved::Unimplemented { .. } => {
                let name = self.symbol_name(op);
                Err(EvalError::unimplemented(format!("the Java implementation of {name}")))
            }
            // `OpLambdaValue` is an `OpValue`, so TLC applies it here (`OpLambdaValue.eval`).
            Resolved::Binding(Binding::Lambda(_)) => {
                Err(EvalError::unimplemented("an operator argument bound to a LAMBDA (M2 unit 4)".into()))
            }
            Resolved::Native(id) | Resolved::Binding(Binding::Native(id)) => {
                if args.len() != crate::modules::arity(id) {
                    let name = self.symbol_name(op);
                    return Err(EvalError::unimplemented(format!("the Java method behind {name} applied to {} arguments", args.len())));
                }
                match crate::modules::imp(id) {
                    crate::modules::NativeImpl::Lazy(f) => match f(self, args, c, s0, s1, control)? {
                        Some(v) => Ok(v),
                        None => self.eval_pure(op, args, c, s0, s1, control),
                    },
                    crate::modules::NativeImpl::Strict(_) => {
                        let mut vals = Vec::with_capacity(args.len());
                        for &a in args {
                            vals.push(self.eval(a.expect_expr(), c, s0, s1, control)?);
                        }
                        crate::modules::apply(self, id, &vals)
                    }
                }
            }
            Resolved::OpRcd(lines) | Resolved::Binding(Binding::OpRcd(lines)) => {
                let mut vals = Vec::with_capacity(args.len());
                for &a in args {
                    vals.push(self.eval(a.expect_expr(), c, s0, s1, control)?);
                }
                self.apply_op_rcd(&lines, &vals)
            }
            Resolved::Binding(Binding::Value(v)) | Resolved::Value(v) if !matches!(v, Value::Undef) => Ok(v),
            // The value is missing or not applicable: `evalApplImpl`'s last `else`.
            _ => {
                let name = self.symbol_name(op);
                let text = self.tool().graph.expr_tlc_text(expr);
                let message = self.out().text(Ec::TLC_CONFIG_UNDEFINED_OR_NO_OPERATOR, &[&name, &text]);
                Err(EvalError::runtime(Ec::TLC_CONFIG_UNDEFINED_OR_NO_OPERATOR, message))
            }
        }
    }

    /// `Tool.getFcnContext(fcn, args, ...)`: `args[1]` evaluated and bound to the formals.
    pub(crate) fn fcn_context(
        &mut self,
        fcn: &Arc<crate::value::FcnLambdaValue>,
        args: &[ExprOrOpArg],
        c: &Ctx,
        s0: StateRef<'_>,
        s1: StateRef<'_>,
        control: EvalControl,
    ) -> EvalResult<Ctx> {
        let arg = self.eval(args[1].expect_expr(), c, s0, s1, control)?;
        // `getFcnContext`'s own domain check (`Tool.java:3999-4004`), whose wording differs
        // from `FcnLambdaValue.apply`'s: the argument is followed straight by the sentence,
        // and the function expression's location is appended.
        if fcn.arg_len() == 1 {
            let domain = fcn.params[0].2.clone();
            if !domain.member(self, &arg)? {
                let f = Value::FcnLambda(fcn.clone()).ppr(self)?;
                let a = arg.ppr(self)?;
                let at = args[0].expect_expr();
                let graph = &self.tool().graph;
                let reason = format!(
                    "In applying the function\n{f},\nthe first argument is:\n{a}which is not in its domain.\n{}",
                    graph.expr_tlc_text(at)
                );
                return Err(EvalError::detailed(reason, Some(graph.loc(at))));
            }
        }
        fcn.fcn_context(self, &arg)
    }

    /// `getVar`, as the callers that bind a `TLCStateMut` use it: the `vars[]` slot of the
    /// declaration found. A declaration outside `vars[]` has no slot, and `TLCStateMut.lookup`
    /// answers `null` for its name; only ENABLED's cutoff reaches one, and there the state is
    /// a `TLCStateFun` keyed by name.
    pub(crate) fn get_var_slot(&mut self, expr: ExprId, c: &Ctx, cutoff: bool) -> Option<crate::state::VarSlot> {
        let decl = self.get_var(expr, c, cutoff)?;
        self.var_slot(decl)
    }

    /// `getPrimedVar`, likewise.
    pub(crate) fn get_primed_var_slot(&mut self, expr: ExprId, c: &Ctx, cutoff: bool) -> Option<crate::state::VarSlot> {
        let decl = self.get_primed_var(expr, c, cutoff)?;
        self.var_slot(decl)
    }

    /// `UniqueString.getVarLoc()` of a declaration's name.
    pub(crate) fn var_slot(&self, decl: tlars_sany::graph::DeclId) -> Option<crate::state::VarSlot> {
        let tool = self.tool();
        tool.vars.slot_of(tool.graph.decl(decl).name)
    }

    /// `SemanticNode.toString()` of a symbol `lookup` returned in place of a value: its
    /// location (`SemanticNode.java:481-489`). `None` for a theorem or assumption definition,
    /// whose location the graph does not keep; TLC expands one before any caller prints it.
    pub(crate) fn symbol_loc_text(&self, sym: SymbolRef) -> Option<String> {
        let graph = &self.tool().graph;
        let loc = match sym {
            SymbolRef::OpDef(d) => graph.op_def(d).loc,
            SymbolRef::Decl(d) => graph.decl(d).loc,
            SymbolRef::Param(p) => graph.param(p).loc,
            SymbolRef::Module(m) => graph.module(m).loc,
            SymbolRef::Thm(_) => return None,
        };
        Some(loc.tlc_text(graph.interner()))
    }

    /// `var.getName()`, which is how a `TLCStateFun` is keyed.
    pub(crate) fn var_name(&self, decl: tlars_sany::graph::DeclId) -> Atom {
        self.tool().graph.decl(decl).name
    }

    /// `Spec.getVar(expr, c, cutoff)` (`SymbolNodeValueLookupProvider.java:29-84`): the state
    /// variable `expr` denotes, if any.
    pub(crate) fn get_var(&mut self, expr: ExprId, c: &Ctx, cutoff: bool) -> Option<tlars_sany::graph::DeclId> {
        let graph = &self.tool().graph;
        match graph.expr(expr) {
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                let c1 = self.subst_context(substs, c);
                self.get_var(*body, &c1, cutoff)
            }
            ExprNode::LetIn { defs, body, .. } => {
                let c1 = self.let_context(defs, c);
                self.get_var(*body, &c1, cutoff)
            }
            ExprNode::Label { body, .. } => self.get_var(*body, c, cutoff),
            ExprNode::OpAppl { op, .. } if self.arity(*op) == 0 => match self.lookup(*op, c, StateRef::Empty, cutoff) {
                Resolved::Binding(Binding::Thunk(t)) => self.get_var(t.expr, &t.ctx.clone(), cutoff),
                Resolved::Binding(Binding::OpDef(d)) | Resolved::UserOp(d) => {
                    let body = self.tool().graph.op_def(d).body;
                    body.and_then(|b| self.get_var(b, c, cutoff))
                }
                Resolved::Var(d) => Some(d),
                _ => None,
            },
            _ => None,
        }
    }

    /// `Spec.getPrimedVar` (`Spec.java:138-205`): the variable `expr` primes, if any.
    pub(crate) fn get_primed_var(&mut self, expr: ExprId, c: &Ctx, cutoff: bool) -> Option<tlars_sany::graph::DeclId> {
        let graph = &self.tool().graph;
        match graph.expr(expr) {
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                let c1 = self.subst_context(substs, c);
                self.get_primed_var(*body, &c1, cutoff)
            }
            ExprNode::LetIn { defs, body, .. } => {
                let c1 = self.let_context(defs, c);
                self.get_primed_var(*body, &c1, cutoff)
            }
            ExprNode::Label { body, .. } => self.get_primed_var(*body, c, cutoff),
            ExprNode::OpAppl { op, args, .. } => {
                if self.opcode(*op) == Some(Builtin::Prime) {
                    return self.get_var(args[0].expect_expr(), c, cutoff);
                }
                if self.arity(*op) != 0 {
                    return None;
                }
                match self.lookup(*op, c, StateRef::Empty, cutoff) {
                    Resolved::Binding(Binding::Thunk(t)) => self.get_primed_var(t.expr, &t.ctx.clone(), cutoff),
                    Resolved::Binding(Binding::OpDef(d)) | Resolved::UserOp(d) => {
                        let body = self.tool().graph.op_def(d).body;
                        body.and_then(|b| self.get_primed_var(b, c, cutoff))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// `getVal` for every substitution, consed over the *outer* context.
    pub(crate) fn subst_context(&mut self, substs: &[tlars_sany::graph::Subst], c: &Ctx) -> Ctx {
        let mut c1 = c.clone();
        for sub in substs {
            let val = self.get_val(sub.expr, c, false);
            c1 = c1.cons(SymbolRef::Decl(sub.decl), val);
        }
        c1
    }

    /// The context `getVar` / `getLevelBound` build for a LET: every 0-arity definition
    /// becomes a thunk over the context so far.
    pub(crate) fn let_context(&mut self, defs: &[OpDefId], c: &Ctx) -> Ctx {
        let graph = &self.tool().graph;
        let mut c1 = c.clone();
        for &d in defs {
            let def = graph.op_def(d);
            if def.params.is_empty() {
                let body = def.body.expect("a LET definition has a body");
                c1 = c1.cons(SymbolRef::OpDef(d), Binding::Thunk(Thunk::new(body, c1.clone(), true)));
            }
        }
        c1
    }

    /// `opNode.getArity()`.
    fn arity(&self, op: SymbolRef) -> usize {
        let graph = &self.tool().graph;
        match op {
            SymbolRef::OpDef(d) => graph.op_def(d).params.len(),
            SymbolRef::Decl(d) => graph.decl(d).arity.max(0) as usize,
            SymbolRef::Param(p) => graph.param(p).arity.max(0) as usize,
            SymbolRef::Thm(_) | SymbolRef::Module(_) => 0,
        }
    }

    /// `Assert.fail("The next state relation is not a boolean expression.\n" + next, next, c)`.
    pub(crate) fn not_boolean_next(&mut self, pred: ExprId, _c: &Ctx) -> EvalError {
        let graph = &self.tool().graph;
        let reason = format!("The next state relation is not a boolean expression.\n{}", graph.expr_tlc_text(pred));
        let loc = graph.loc(pred);
        EvalError::detailed(reason, Some(loc))
    }

    /// `SymbolNodeValueLookupProvider.getLevelBound` (`:158-280`): the conservative level TLC
    /// uses to split SPECIFICATION into initial predicate, next-state relation and temporal
    /// formulas.
    pub(crate) fn level_bound(&mut self, expr: ExprId, c: &Ctx) -> u8 {
        let graph = &self.tool().graph;
        match graph.expr(expr) {
            ExprNode::OpAppl { op, args, bound, .. } => {
                if let Some(code) = self.opcode(*op) {
                    if code.is_temporal() {
                        return 3;
                    }
                    if code.is_action() {
                        return 2;
                    }
                    if code == Builtin::Enabled {
                        return 1;
                    }
                }
                let mut level = 0;
                for b in bound.iter().flat_map(|b| b.lists.iter()) {
                    level = level.max(self.level_bound(b.domain, c));
                }
                // A recursive function's own name is bound, so its body is not walked again.
                let c = match (self.opcode(*op), bound.as_deref()) {
                    (Some(Builtin::RecursiveFcnSpec), Some(b)) => c.cons(SymbolRef::Param(b.unbounded[0]), Binding::Value(Value::Int(1))),
                    _ => c.clone(),
                };
                for a in args.iter() {
                    if let ExprOrOpArg::Expr(e) = *a {
                        level = level.max(self.level_bound(e, &c));
                    }
                }
                if self.opcode(*op).is_some() {
                    return level;
                }
                // A user-defined operator: a variable is level 1, a definition contributes
                // its body's level, a thunk its expression's.
                if matches!(op, SymbolRef::Decl(d) if self.tool().vars.slot_of(graph.decl(*d).name).is_some()) {
                    return 1;
                }
                match self.lookup(*op, &c, StateRef::Empty, false) {
                    Resolved::Binding(Binding::OpDef(d)) | Resolved::UserOp(d) => {
                        let c1 = c.cons(*op, Binding::Value(Value::Int(1)));
                        if let Some(body) = graph.op_def(d).body {
                            level = level.max(self.level_bound(body, &c1));
                        }
                    }
                    Resolved::Binding(Binding::Thunk(t)) => {
                        let (e, tc) = (t.expr, t.ctx.clone());
                        level = level.max(self.level_bound(e, &tc));
                    }
                    // A Java override contributes its declared `minLevel`, which is how
                    // `TLCGet` stays out of constant folding (`:270-276`). An override tlars
                    // does not implement is a `MethodValue` to TLC all the same, so it
                    // contributes its own `minLevel` and not level 0.
                    Resolved::Native(id) | Resolved::Binding(Binding::Native(id)) => {
                        level = level.max(crate::modules::min_level(id));
                    }
                    Resolved::Unimplemented { min_level } => level = level.max(min_level),
                    _ => {}
                }
                level
            }
            ExprNode::LetIn { defs, body, .. } => {
                let mut level = 0;
                let mut c1 = c.clone();
                for &d in defs.iter() {
                    if let Some(b) = graph.op_def(d).body {
                        level = level.max(self.level_bound(b, &c1));
                    }
                    c1 = c1.cons(SymbolRef::OpDef(d), Binding::Value(Value::Int(1)));
                }
                level.max(self.level_bound(*body, &c1))
            }
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                let mut c1 = c.clone();
                for sub in substs.iter() {
                    let val = self.get_val(sub.expr, c, true);
                    c1 = c1.cons(SymbolRef::Decl(sub.decl), val);
                }
                self.level_bound(*body, &c1)
            }
            ExprNode::Label { body, .. } => self.level_bound(*body, c),
            _ => 0,
        }
    }
}

/// `opNode.getKind() == VariableDeclKind`.
fn is_variable(tool: &crate::tool::Tool, d: tlars_sany::graph::DeclId) -> bool {
    use tlars_sany::graph::DeclKind;
    matches!(tool.graph.decl(d).kind, DeclKind::Variable | DeclKind::NewVariable)
}

/// What an operator argument of a Java module resolves to: `OpLambdaValue` for a definition,
/// `MethodValue` for another module's override. An `OpLambdaValue` keeps the states it was
/// built in (`Tool.java:1888-1892`), so a test that reads a variable reads the caller's.
pub(crate) enum OpArgValue<'s> {
    Op { def: OpDefId, ctx: Ctx, s0: StateRef<'s>, s1: StateRef<'s>, control: EvalControl },
    Native(crate::modules::NativeId),
}

/// The two operands of a `$Pair` node, which SANY uses for record fields, CASE arms and
/// EXCEPT clauses.
fn pair(graph: &tlars_sany::graph::SemanticGraph, e: ExprId) -> (ExprId, ExprId) {
    let ExprNode::OpAppl { args, .. } = graph.expr(e) else { unreachable!("$Pair is an application") };
    (args[0].expect_expr(), args[1].expect_expr())
}

fn resolved(b: &OpBinding, d: OpDefId) -> Resolved {
    match b {
        OpBinding::Value(v) => Resolved::Value(v.clone()),
        OpBinding::Native(id) => Resolved::Native(*id),
        OpBinding::Replaced(r) => Resolved::UserOp(*r),
        OpBinding::Table(_) => {
            let _ = d;
            Resolved::Value(Value::Undef)
        }
        OpBinding::Unimplemented { min_level } => Resolved::Unimplemented { min_level: *min_level },
    }
}

/// One `ContextEnumerator` slot: a bound variable, or the formals a `<<x, y>> \in S` list
/// destructures each element into.
enum Bound {
    Var(ParamId),
    Tuple(Box<[ParamId]>),
}

/// `ContextEnumerator`.
pub(crate) struct Contexts {
    base: Ctx,
    vars: Vec<(Bound, ValueEnumeration)>,
    current: Vec<Value>,
    done: bool,
}

impl Contexts {
    pub(crate) fn next(&mut self, ev: &mut Ev<'_>) -> EvalResult<Option<Ctx>> {
        if self.done {
            return Ok(None);
        }
        let mut c1 = self.base.clone();
        for ((p, _), v) in self.vars.iter().zip(&self.current) {
            match p {
                Bound::Var(p) => c1 = c1.cons(SymbolRef::Param(*p), Binding::Value(v.clone())),
                Bound::Tuple(formals) => {
                    let elems = match v {
                        Value::Tuple(t) if t.elems.len() == formals.len() => t.elems.clone(),
                        // `Assert.fail(EC.TLC_ARGUMENT_MISMATCH, varList[0].toString())`
                        // (`ContextEnumerator.java:51, 55`): the parameter is a location, and
                        // the message's `%1` has no closing `%`, so it stays literal.
                        _ => {
                            let graph = ev.tool().graph();
                            let at = graph.param(formals[0]).loc.tlc_text(graph.interner());
                            let message = ev.out().text(Ec::TLC_ARGUMENT_MISMATCH, &[&at]);
                            return Err(EvalError::runtime(Ec::TLC_ARGUMENT_MISMATCH, message));
                        }
                    };
                    for (&id, e) in formals.iter().zip(elems.iter()) {
                        c1 = c1.cons(SymbolRef::Param(id), Binding::Value(e.clone()));
                    }
                }
            }
        }
        for i in 0..self.vars.len() {
            if let Some(v) = self.vars[i].1.next(ev)? {
                self.current[i] = v;
                return Ok(Some(c1));
            }
            if i == self.vars.len() - 1 {
                self.done = true;
                break;
            }
            self.vars[i].1.reset(ev)?;
            self.current[i] = self.vars[i].1.next(ev)?.expect("a reset enumeration of a non-empty set has an element");
        }
        if self.vars.is_empty() {
            self.done = true;
        }
        Ok(Some(c1))
    }
}

/// The opcodes of `BuiltInOPs` this evaluator implements; the others are named.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Builtin {
    ConjList,
    DisjList,
    Land,
    Lor,
    Lnot,
    Implies,
    Equiv,
    Eq,
    Noteq,
    In,
    Notin,
    IfThenElse,
    Case,
    BoundedForall,
    BoundedExists,
    BoundedChoose,
    SubsetOf,
    SetOfAll,
    Cup,
    Cap,
    Setdiff,
    Subseteq,
    Subset,
    Union,
    CartesianProd,
    SetOfFcns,
    SetOfRcds,
    UnboundedChoose,
    UnboundedExists,
    UnboundedForall,
    SetEnumerate,
    Tuple,
    RcdConstructor,
    RcdSelect,
    FcnApply,
    Domain,
    /// `$FcnConstructor` and `$NonRecursiveFcnSpec`, which differ only in where SANY puts them.
    FcnConstructor,
    RecursiveFcnSpec,
    Except,
    Prime,
    Unchanged,
    Enabled,
    /// `<A>_e`.
    AngleAct,
    /// `[A]_e`.
    SquareAct,
    Cdot,
    /// `$Nop`, which SANY wraps a subexpression name in.
    Nop,
    Box,
    TemporalExists,
    TemporalForall,
    Unimplemented(&'static str),
}

impl Builtin {
    /// `BuiltInOPs.isAction` (`BuiltInOPs.java:113-115`).
    pub(crate) fn is_action(self) -> bool {
        matches!(self, Builtin::Prime | Builtin::Unchanged | Builtin::AngleAct | Builtin::SquareAct | Builtin::Cdot)
    }

    /// `BuiltInOPs.isTemporal`: `$SF` through `<>`.
    pub(crate) fn is_temporal(self) -> bool {
        match self {
            Builtin::Box | Builtin::TemporalExists | Builtin::TemporalForall => true,
            Builtin::Unimplemented(name) => matches!(name, "$SF" | "$WF" | "~>" | "-+->" | "<>"),
            _ => false,
        }
    }
}

/// Every name `BuiltInOPs` registers (`BuiltInOPs.java:17-84`).
const BUILT_IN_OPS: &[&str] = &[
    "$AngleAct",
    "$BoundedChoose",
    "$BoundedExists",
    "$BoundedForall",
    "$Case",
    "$CartesianProd",
    "$ConjList",
    "$DisjList",
    "$Except",
    "$FcnApply",
    "$FcnConstructor",
    "$IfThenElse",
    "$NonRecursiveFcnSpec",
    "$Pair",
    "$RcdConstructor",
    "$RcdSelect",
    "$RecursiveFcnSpec",
    "$Seq",
    "$SquareAct",
    "$SetEnumerate",
    "$SF",
    "$SetOfAll",
    "$SetOfRcds",
    "$SetOfFcns",
    "$SubsetOf",
    "$Tuple",
    "$TemporalExists",
    "$TemporalForall",
    "$UnboundedChoose",
    "$UnboundedExists",
    "$UnboundedForall",
    "$WF",
    "\\lnot",
    "SUBSET",
    "UNION",
    "DOMAIN",
    "[]",
    "<>",
    "ENABLED",
    "UNCHANGED",
    "=",
    "\\land",
    "\\lor",
    "=>",
    "\\cdot",
    "\\equiv",
    "~>",
    "-+->",
    "/=",
    "\\subseteq",
    "\\in",
    "\\notin",
    "\\",
    "\\intersect",
    "$Nop",
    "\\union",
    "'",
];

impl Builtin {
    /// [`Tool::builtins`](crate::tool::Tool): the opcode of each definition SANY marks
    /// built-in, by name; `None` for every other definition.
    pub(crate) fn table(graph: &tlars_sany::graph::SemanticGraph) -> Box<[Option<Builtin>]> {
        graph
            .op_def_ids()
            .map(|d| {
                let def = graph.op_def(d);
                if def.kind == OpDefKind::BuiltIn { Builtin::of(&graph.interner().text(def.name)) } else { None }
            })
            .collect()
    }

    fn of(name: &str) -> Option<Builtin> {
        Some(match name {
            "$ConjList" => Builtin::ConjList,
            "$DisjList" => Builtin::DisjList,
            "\\land" => Builtin::Land,
            "\\lor" => Builtin::Lor,
            "\\lnot" => Builtin::Lnot,
            "=>" => Builtin::Implies,
            "\\equiv" => Builtin::Equiv,
            "=" => Builtin::Eq,
            "/=" => Builtin::Noteq,
            "\\in" => Builtin::In,
            "\\notin" => Builtin::Notin,
            "$IfThenElse" => Builtin::IfThenElse,
            "$Case" => Builtin::Case,
            "$BoundedForall" => Builtin::BoundedForall,
            "$BoundedExists" => Builtin::BoundedExists,
            "$BoundedChoose" => Builtin::BoundedChoose,
            "$SubsetOf" => Builtin::SubsetOf,
            "\\union" => Builtin::Cup,
            "\\intersect" => Builtin::Cap,
            "\\" => Builtin::Setdiff,
            "\\subseteq" => Builtin::Subseteq,
            "SUBSET" => Builtin::Subset,
            "UNION" => Builtin::Union,
            "$CartesianProd" => Builtin::CartesianProd,
            "$SetOfFcns" => Builtin::SetOfFcns,
            "$SetOfRcds" => Builtin::SetOfRcds,
            "$SetOfAll" => Builtin::SetOfAll,
            "$UnboundedChoose" => Builtin::UnboundedChoose,
            "$UnboundedExists" => Builtin::UnboundedExists,
            "$UnboundedForall" => Builtin::UnboundedForall,
            "$SetEnumerate" => Builtin::SetEnumerate,
            "$Tuple" => Builtin::Tuple,
            "$RcdConstructor" => Builtin::RcdConstructor,
            "$RcdSelect" => Builtin::RcdSelect,
            "$FcnApply" => Builtin::FcnApply,
            "DOMAIN" => Builtin::Domain,
            "$FcnConstructor" | "$NonRecursiveFcnSpec" => Builtin::FcnConstructor,
            "$RecursiveFcnSpec" => Builtin::RecursiveFcnSpec,
            "$Except" => Builtin::Except,
            "'" => Builtin::Prime,
            "UNCHANGED" => Builtin::Unchanged,
            "ENABLED" => Builtin::Enabled,
            "$AngleAct" => Builtin::AngleAct,
            "$SquareAct" => Builtin::SquareAct,
            "\\cdot" => Builtin::Cdot,
            "$Nop" => Builtin::Nop,
            "[]" => Builtin::Box,
            "$TemporalExists" => Builtin::TemporalExists,
            "$TemporalForall" => Builtin::TemporalForall,
            other => Builtin::Unimplemented(BUILT_IN_OPS.iter().copied().find(|n| *n == other)?),
        })
    }
}
