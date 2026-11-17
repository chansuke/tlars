//! The formula side of liveness (`Liveness.processLiveness`): tagging, pushNeg, simplify,
//! DNF, classification into `<>[]A`, `[]<>A`, `[]<>S` and tableau formulas, grouping of DNF
//! branches with structurally equal tableau formulas into one `OrderOfSolution`, and the
//! `TBGraph` particle tableau. It lives here, not in the checker, because every leaf is an
//! expression plus a context; the checker sees only check indices and bit vectors
//! (grafted from candidate 2's `model::live`).

use crate::ctx::Ctx;
use crate::error::EvalResult;
use crate::eval::Ev;
use crate::state::State;
use tlars_sany::graph::ExprId;

/// `LiveExprNode` family (`LNAll`, `LNEven`, `LNConj`, `LNDisj`, `LNNeg`, `LNStateAST`,
/// `LNActionAST`, `LNStateEnabled`, ...).
pub(crate) enum LiveExpr {
    State { expr: ExprId, ctx: Ctx },
    Action { expr: ExprId, ctx: Ctx },
    Enabled { expr: ExprId, ctx: Ctx },
    Not(Box<LiveExpr>),
    And(Box<[LiveExpr]>),
    Or(Box<[LiveExpr]>),
    Always(Box<LiveExpr>),
    Eventually(Box<LiveExpr>),
    Bool(bool),
}

/// `PossibleErrorModel`: indices into its solution's check bins.
pub struct Pem {
    pub ae_state: Box<[u32]>,
    pub ae_action: Box<[u32]>,
    pub ea_action: Box<[u32]>,
}

/// `TBGraph`: tableau nodes with their state-level particles and promises.
pub struct Tableau {
    pub(crate) nodes: Box<[TableauNode]>,
}

pub(crate) struct TableauNode {
    pub(crate) particle: Box<[LiveExpr]>,
    pub(crate) successors: Box<[u32]>,
    pub(crate) accepting: bool,
}

/// One `OrderOfSolution`: its PEMs index `check_state`/`check_action`, which stay private.
pub struct Solution {
    pub tableau: Option<Tableau>,
    pub pems: Box<[Pem]>,
    pub(crate) check_state: Box<[LiveExpr]>,
    pub(crate) check_action: Box<[LiveExpr]>,
}

/// Every liveness obligation of the spec, in TLC's solution order.
pub struct LiveSpec {
    pub solutions: Box<[Solution]>,
}

/// Results of a solution's state or action checks, indexed like its bins.
pub struct CheckBits(pub Box<[bool]>);

impl<'t> Ev<'t> {
    /// `Liveness.processLiveness`; `None` when there is nothing to check. Prints
    /// "Implied-temporal checking--satisfiability problem has N branches".
    pub fn live_spec(&mut self) -> EvalResult<Option<LiveSpec>> {
        todo!()
    }
    /// `checkState` bins of one solution for state `s`.
    pub fn live_state_bits(&mut self, sol: &Solution, s: &State) -> EvalResult<CheckBits> {
        todo!()
    }
    /// `checkAction` bins of one solution for the edge `s0 -> s1` (pre-evaluated outside
    /// the graph lock, `LiveCheck.addNextState`).
    pub fn live_action_bits(&mut self, sol: &Solution, s0: &State, s1: &State) -> EvalResult<CheckBits> {
        todo!()
    }
    /// Tableau nodes of `tab` whose particle `s` satisfies (consistency test).
    pub fn tableau_matches(&mut self, tab: &Tableau, s: &State) -> EvalResult<Vec<u32>> {
        todo!()
    }
    /// `Liveness.findViolatedProperties`: re-evaluate each original property on the lasso.
    pub fn violated_properties(&mut self, prefix: &[State], cycle: &[State]) -> EvalResult<Vec<tlars_sany::Atom>> {
        todo!()
    }
}
