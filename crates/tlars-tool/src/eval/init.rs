//! `getInitStates*`. Same continuation-passing shape as `next`, binding unprimed variables
//! in one `PartialState`: `x = e` binds or tests, `x \in S` enumerates `S.elements(Native)`,
//! `x \subseteq S` enumerates SUBSET S, `\E` iterates normalised contexts, IF/CASE take one
//! branch, SubstIn pushes thunks, and once every variable is bound the rest are filters.

use super::next::{ActItem, ActKind, Acts};
use super::{Ev, InitSink};
use crate::ctx::Ctx;
use crate::error::Flow;
use crate::state::PartialState;
use tlars_sany::graph::ExprId;

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
        todo!()
    }

    /// `getInitStates(ActionItemList, TLCState, IStateFunctor)`: pop or emit.
    pub(crate) fn init_continue(&mut self, acts: Acts<'_>, ps: &mut PartialState, sink: &mut dyn InitSink) -> Flow {
        let Acts::Cons(item) = acts else {
            return sink.add_init(self, ps);
        };
        self.init_impl(item.pred, item.next, item.ctx, ps, sink)
    }
}
