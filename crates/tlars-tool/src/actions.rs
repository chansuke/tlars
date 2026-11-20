//! `tlc2.tool.Action` and `Tool.getActions` splitting. The split order is successor order.
//!
//! Splitting (`Tool.java:273-380`): `\/`/`$DisjList` recurse per disjunct; `\E x \in S`
//! recurses per normalised context (first bound variable fastest), falling back to a single
//! action if the domain cannot be evaluated or is empty; a user operator applied to
//! constant-level arguments recurses into its body (label = operator name with params);
//! LET recurses into the body; a `SubstIn` **with** substitutions is one action, no further
//! splitting; anything else is one action.

use crate::ctx::{Binding, Ctx};
use crate::error::EvalResult;
use crate::eval::expr::{Builtin, Resolved};
use crate::eval::{EvalControl, Ev};
use crate::state::StateRef;
use tlars_sany::Atom;
use tlars_sany::graph::{ExprId, ExprNode, Level, OpDefId, SymbolRef};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionId(u32);

impl ActionId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

pub struct Action {
    /// Index in its own list. TLC numbers the initial predicates and the next-state actions
    /// in one sequence (`Tool.java:236-240`), which only `-coverage` and `-dump` read.
    pub(crate) id: ActionId,
    pub(crate) pred: ExprId,
    /// Constant bindings from splitting (`\E` contexts, constant operator args).
    pub(crate) ctx: Ctx,
    pub(crate) name: Option<Atom>,
    pub(crate) op_def: Option<OpDefId>,
    pub(crate) is_init: bool,
    pub(crate) is_internal: bool,
}

impl Action {
    pub fn id(&self) -> ActionId {
        self.id
    }
    pub fn name(&self) -> Option<Atom> {
        self.name
    }
}

/// `new Action(this, pred, con, opDef)`: the action's name is the definition's, if any.
fn action(ev: &Ev<'_>, pred: ExprId, c: &Ctx, op_def: Option<OpDefId>, is_init: bool, index: usize) -> Action {
    let name = op_def.map(|d| ev.tool().graph().op_def(d).name);
    Action { id: ActionId(index as u32), pred, ctx: c.clone(), name, op_def, is_init, is_internal: false }
}

/// `getActions(next)` (`Tool.java:269-271`), which threads the next-state *definition* down
/// as the name every action that splitting does not rename inherits.
pub(crate) fn split_next(ev: &mut Ev<'_>, next: ExprId, c: &Ctx, op_def: Option<OpDefId>, out: &mut Vec<Action>) -> EvalResult<()> {
    split(ev, next, c, op_def, out, false)
}

/// `getInitStateSpec()`: one action per initial predicate, unsplit. `new Action(pred, con,
/// opDef, ...)` names it after the definition the cfg's INIT gave, which `TLCGet("spec")`
/// reports; a conjunct of SPECIFICATION has no definition and stays unnamed.
pub(crate) fn init_action(graph: &tlars_sany::graph::SemanticGraph, index: usize, pred: ExprId, c: &Ctx, op_def: Option<OpDefId>) -> Action {
    let name = op_def.map(|d| graph.op_def(d).name);
    Action { id: ActionId(index as u32), pred, ctx: c.clone(), name, op_def, is_init: true, is_internal: false }
}

/// `(OpDefNode) opNode` on a symbol that is not a definition.
fn not_an_op_def(op: SymbolRef) -> crate::error::EvalError {
    let from = match op {
        SymbolRef::OpDef(_) => unreachable!("an OpDefNode casts"),
        SymbolRef::Decl(_) => "OpDeclNode",
        SymbolRef::Param(_) => "FormalParamNode",
        SymbolRef::Thm(_) => "ThmOrAssumpDefNode",
        SymbolRef::Module(_) => "ModuleNode",
    };
    let message = tlars_java::class_cast(&format!("tla2sany.semantic.{from}"), "tla2sany.semantic.OpDefNode");
    crate::error::EvalError {
        ec: tlars_msg::Ec::GENERAL,
        message,
        loc: None,
        kind: crate::error::EvalErrorKind::Jvm { class: "java.lang.ClassCastException" },
        params: Vec::new(),
    }
}

fn split(ev: &mut Ev<'_>, pred: ExprId, c: &Ctx, name: Option<OpDefId>, out: &mut Vec<Action>, is_init: bool) -> EvalResult<()> {
    let graph = ev.tool().graph.clone();
    match graph.expr(pred) {
        ExprNode::OpAppl { .. } => split_appl(ev, pred, c, name, out, is_init),
        ExprNode::LetIn { body, .. } | ExprNode::Label { body, .. } => split(ev, *body, c, name, out, is_init),
        // A substitution is not split further; without one the body is.
        ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
            if substs.is_empty() {
                return split(ev, *body, c, name, out, is_init);
            }
            let len = out.len();
            out.push(action(ev, pred, c, name, is_init, len));
            Ok(())
        }
        _ => Err(ev.not_boolean_next(pred, c)),
    }
}

fn split_appl(ev: &mut Ev<'_>, pred: ExprId, c: &Ctx, name: Option<OpDefId>, out: &mut Vec<Action>, is_init: bool) -> EvalResult<()> {
    let graph = ev.tool().graph.clone();
    let ExprNode::OpAppl { op, args, .. } = graph.expr(pred) else { unreachable!() };
    let (op, args) = (*op, args.clone());
    let mut code = ev.opcode(op);
    if code.is_none() {
        if let Resolved::UserOp(d) | Resolved::Binding(Binding::OpDef(d)) = ev.lookup(op, c, StateRef::Empty, false) {
            code = ev.opcode(SymbolRef::OpDef(d));
            if code.is_none() {
                // Recurse into the body only when every argument is constant level; the
                // arguments are then evaluated and bound, so the action carries its values.
                let constant_args = args.iter().all(|a| match *a {
                    tlars_sany::graph::ExprOrOpArg::Expr(e) => graph.level(e) == Level::Constant,
                    tlars_sany::graph::ExprOrOpArg::OpArg(SymbolRef::OpDef(d)) => graph.op_level(d) == Level::Constant,
                    _ => true,
                });
                if constant_args {
                    let params = graph.op_def(d).params.clone();
                    let body = graph.op_def(d).body.expect("a user definition has a body");
                    let evaluated = (|| {
                        let mut c1 = c.clone();
                        for (&p, a) in params.iter().zip(args.iter()) {
                            // `this.eval(args[i], con, TLCState.Empty, cm)`, which for an
                            // operator argument is the operator itself, not an expression.
                            let v = ev.eval_arg(*a, c, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)?;
                            c1 = c1.cons(SymbolRef::Param(p), v);
                        }
                        Ok::<Ctx, crate::error::EvalError>(c1)
                    })();
                    // `catch (Throwable e) { /*SKIP*/ }`: a failure falls through to one action.
                    if let Ok(c1) = evaluated {
                        return split(ev, body, &c1, Some(d), out, is_init);
                    }
                }
            }
        }
        if code.is_none() {
            // `new Action(this, next, con, (OpDefNode) opNode)`: the cast fails for anything
            // else, and nothing catches it (e.g. `Next == x` for a variable `x`).
            let SymbolRef::OpDef(d) = op else { return Err(not_an_op_def(op)) };
            let len = out.len();
            out.push(action(ev, pred, c, Some(d), is_init, len));
            return Ok(());
        }
    }
    match code {
        Some(Builtin::BoundedExists) => {
            let ExprNode::OpAppl { bound, .. } = graph.expr(pred) else { unreachable!() };
            let bound = bound.as_deref().expect("a bounded quantifier has bounds");
            let before = out.len();
            let body = args[0].expect_expr();
            let split_all = (|| {
                let mut contexts = ev.contexts(bound, c, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)?;
                let Some(first) = contexts.next(ev)? else { return Ok(false) };
                split(ev, body, &first, name, out, is_init)?;
                while let Some(c1) = contexts.next(ev)? {
                    split(ev, body, &c1, name, out, is_init)?;
                }
                Ok::<bool, crate::error::EvalError>(true)
            })();
            match split_all {
                // An empty domain gives one action over the quantifier itself.
                Ok(false) => {
                    let len = out.len();
                    out.push(action(ev, pred, c, name, is_init, len));
                }
                // A domain that fails to enumerate also gives one action, and whatever was
                // split before the failure is dropped.
                Err(_) => {
                    out.truncate(before);
                    let len = out.len();
                    out.push(action(ev, pred, c, name, is_init, len));
                }
                Ok(true) => {}
            }
            Ok(())
        }
        Some(Builtin::DisjList | Builtin::Lor) => {
            for a in args.iter() {
                split(ev, a.expect_expr(), c, name, out, is_init)?;
            }
            Ok(())
        }
        _ => {
            let len = out.len();
            out.push(action(ev, pred, c, name, is_init, len));
            Ok(())
        }
    }
}
