//! `tlc2.tool.Action` and `Tool.getActions` splitting. The split order is successor order.
//!
//! Splitting (`Tool.java:273-380`): `\/`/`$DisjList` recurse per disjunct; `\E x \in S`
//! recurses per normalised context (first bound variable fastest), falling back to a single
//! action if the domain cannot be evaluated or is empty; a user operator applied to
//! constant-level arguments recurses into its body (label = operator name with params);
//! LET recurses into the body; a `SubstIn` **with** substitutions is one action, no further
//! splitting; anything else is one action.

use crate::ctx::Ctx;
use crate::error::EvalResult;
use crate::eval::Ev;
use tlars_sany::Atom;
use tlars_sany::graph::{ExprId, OpDefId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionId(u32);

impl ActionId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

pub struct Action {
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

/// `getActions(next, Context.Empty)`.
pub(crate) fn split_next(ev: &mut Ev<'_>, next: ExprId, c: &Ctx, out: &mut Vec<Action>) -> EvalResult<()> {
    todo!()
}

/// Init predicates are split the same way and numbered before the next-state actions.
pub(crate) fn split_init(ev: &mut Ev<'_>, init: ExprId, c: &Ctx, out: &mut Vec<Action>) -> EvalResult<()> {
    todo!()
}
