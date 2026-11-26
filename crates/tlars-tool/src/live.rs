//! The formula side of liveness (`Liveness.processLiveness`): `astToLive`, negation pushed
//! to the atoms, `simplify`, DNF, the classification of each DNF branch into `<>[]A`,
//! `[]<>A`, `[]<>S` and a tableau formula, the grouping of branches with structurally equal
//! tableau formulas into one `OrderOfSolution`, and the `TBGraph` particle tableau. It lives
//! here, not in the checker, because every leaf is an expression plus a context; the
//! checker sees only check indices and bit vectors (grafted from candidate 2's `model::live`).
//!
//! The node classes are `LiveExprNode`'s (`LNBool`, `LNStateAST`, `LNStateEnabled`,
//! `LNAction`, `LNNeg`, `LNAll`, `LNEven`, `LNNext`, `LNConj`, `LNDisj`), and each method
//! follows the one of the same name. Children are shared, as the Java objects are when DNF
//! duplicates a subformula.
//!
//! Atoms compare by identity. TLC compares them by the tag `tagExpr` numbers them with; an
//! atom visited twice keeps its last tag, so two different atoms never share one, and an
//! atom made after tagging (none is) would be the only exception.

use crate::ctx::{Binding, Ctx};
use crate::error::{EvalError, EvalErrorKind, EvalResult};
use crate::eval::expr::{Builtin, Resolved};
use crate::eval::{Ev, EvalControl};
use crate::state::{State, StateRef};
use crate::value::Value;
use std::sync::Arc;
use tlars_msg::Ec;
use tlars_sany::Atom;
use tlars_sany::graph::{ExprId, ExprNode, SymbolRef};

type L = Arc<Live>;

/// `LiveExprNode`.
pub(crate) enum Live {
    Bool(bool),
    /// `LNStateAST` and `LNStateEnabled`, both `LNState`.
    State(Arc<StateAtom>),
    /// `LNAction`.
    Action(Arc<ActionAtom>),
    Neg(L),
    /// `LNAll`: `[]`.
    All(L),
    /// `LNEven`: `<>`.
    Even(L),
    /// `LNNext`: `()`, which only the tableau makes.
    Next(L),
    Conj(Box<[L]>),
    Disj(Box<[L]>),
}

pub(crate) enum StateAtom {
    Ast { expr: ExprId, ctx: Ctx },
    Enabled { pred: ExprId, ctx: Ctx, subscript: Option<ExprId>, is_box: bool },
}

pub(crate) struct ActionAtom {
    body: ExprId,
    ctx: Ctx,
    subscript: Option<ExprId>,
    is_box: bool,
}

const CONSTANT: u8 = 0;
const VARIABLE: u8 = 1;
const ACTION: u8 = 2;
const TEMPORAL: u8 = 3;

fn arc(l: Live) -> L {
    Arc::new(l)
}

/// `addConj`/`addDisj`: a junction of the same kind is spliced in, recursively.
fn add_conj(v: &mut Vec<L>, e: L) {
    if let Live::Conj(cs) = &*e {
        for c in cs.iter() {
            add_conj(v, c.clone());
        }
    } else {
        v.push(e);
    }
}

fn add_disj(v: &mut Vec<L>, e: L) {
    if let Live::Disj(ds) = &*e {
        for d in ds.iter() {
            add_disj(v, d.clone());
        }
    } else {
        v.push(e);
    }
}

fn conj_of(items: impl IntoIterator<Item = L>) -> L {
    let mut v = Vec::new();
    for e in items {
        add_conj(&mut v, e);
    }
    arc(Live::Conj(v.into()))
}

fn disj_of(items: impl IntoIterator<Item = L>) -> L {
    let mut v = Vec::new();
    for e in items {
        add_disj(&mut v, e);
    }
    arc(Live::Disj(v.into()))
}

/// The two-argument constructors, which do not splice.
fn conj2(a: L, b: L) -> L {
    arc(Live::Conj(Box::new([a, b])))
}

fn disj2(a: L, b: L) -> L {
    arc(Live::Disj(Box::new([a, b])))
}

impl Live {
    fn level(&self) -> u8 {
        match self {
            Live::Bool(_) => CONSTANT,
            Live::State(_) => VARIABLE,
            Live::Action(_) | Live::Next(_) => ACTION,
            Live::Neg(b) => b.level(),
            Live::All(_) | Live::Even(_) => TEMPORAL,
            Live::Conj(xs) | Live::Disj(xs) => xs.iter().map(|x| x.level()).max().unwrap_or(CONSTANT),
        }
    }

    fn contains_action(&self) -> bool {
        match self {
            Live::Bool(_) | Live::State(_) => false,
            Live::Action(_) => true,
            Live::Neg(b) | Live::All(b) | Live::Even(b) | Live::Next(b) => b.contains_action(),
            Live::Conj(xs) | Live::Disj(xs) => xs.iter().any(|x| x.contains_action()),
        }
    }

    /// `equals(LiveExprNode)`: structural, atoms by identity.
    fn equals(&self, other: &Live) -> bool {
        match (self, other) {
            (Live::Bool(a), Live::Bool(b)) => a == b,
            (Live::State(a), Live::State(b)) => Arc::ptr_eq(a, b),
            (Live::Action(a), Live::Action(b)) => Arc::ptr_eq(a, b),
            (Live::Neg(a), Live::Neg(b))
            | (Live::All(a), Live::All(b))
            | (Live::Even(a), Live::Even(b))
            | (Live::Next(a), Live::Next(b)) => a.equals(b),
            (Live::Conj(a), Live::Conj(b)) | (Live::Disj(a), Live::Disj(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.equals(y))
            }
            _ => false,
        }
    }

    /// `getAEBody`: `A` of `[]<>A`.
    fn ae_body(&self) -> Option<L> {
        match self {
            Live::All(b) => match &**b {
                Live::Even(x) => Some(x.clone()),
                _ => None,
            },
            _ => None,
        }
    }
}

/// `pushNeg()`: the negation of `l`, one level down.
fn push_neg(l: &L) -> L {
    match &**l {
        Live::Bool(b) => arc(Live::Bool(!b)),
        Live::Neg(b) => b.clone(),
        Live::All(b) => arc(Live::Even(push_neg(b))),
        Live::Even(b) => arc(Live::All(push_neg(b))),
        Live::Conj(xs) => disj_of(xs.iter().map(push_neg)),
        Live::Disj(xs) => conj_of(xs.iter().map(push_neg)),
        _ => arc(Live::Neg(l.clone())),
    }
}

/// `pushNeg(hasNeg)`: negation pushed all the way to the atoms.
fn push_neg_deep(l: &L, neg: bool) -> L {
    match &**l {
        Live::Bool(b) => {
            if neg {
                arc(Live::Bool(!b))
            } else {
                l.clone()
            }
        }
        Live::Neg(b) => push_neg_deep(b, !neg),
        Live::All(b) => {
            if neg {
                arc(Live::Even(push_neg_deep(b, true)))
            } else {
                arc(Live::All(push_neg_deep(b, false)))
            }
        }
        Live::Even(b) => {
            if neg {
                arc(Live::All(push_neg_deep(b, true)))
            } else {
                arc(Live::Even(push_neg_deep(b, false)))
            }
        }
        Live::Conj(xs) => {
            if neg {
                disj_of(xs.iter().map(|x| push_neg_deep(x, true)))
            } else {
                conj_of(xs.iter().map(|x| push_neg_deep(x, false)))
            }
        }
        Live::Disj(xs) => {
            if neg {
                conj_of(xs.iter().map(|x| push_neg_deep(x, true)))
            } else {
                disj_of(xs.iter().map(|x| push_neg_deep(x, false)))
            }
        }
        _ => {
            if neg {
                arc(Live::Neg(l.clone()))
            } else {
                l.clone()
            }
        }
    }
}

fn simplify(l: &L) -> L {
    match &**l {
        Live::Neg(b) => {
            let b1 = simplify(b);
            if let Live::Bool(v) = &*b1 { arc(Live::Bool(!v)) } else { arc(Live::Neg(b1)) }
        }
        Live::All(b) => {
            let b1 = simplify(b);
            let b1 = if let Live::All(x) = &*b1 { x.clone() } else { b1 };
            arc(Live::All(b1))
        }
        Live::Even(b) => {
            let b1 = simplify(b);
            let b1 = if let Live::Even(x) = &*b1 { x.clone() } else { b1 };
            arc(Live::Even(b1))
        }
        Live::Conj(xs) => {
            let mut v = Vec::new();
            for x in xs.iter() {
                let e = simplify(x);
                match &*e {
                    Live::Bool(false) => return arc(Live::Bool(false)),
                    Live::Bool(true) => {}
                    _ => add_conj(&mut v, e),
                }
            }
            match v.len() {
                0 => arc(Live::Bool(true)),
                1 => v.pop().expect("one"),
                _ => arc(Live::Conj(v.into())),
            }
        }
        Live::Disj(xs) => {
            let mut v = Vec::new();
            for x in xs.iter() {
                let e = simplify(x);
                match &*e {
                    Live::Bool(true) => return arc(Live::Bool(true)),
                    Live::Bool(false) => {}
                    _ => add_disj(&mut v, e),
                }
            }
            match v.len() {
                0 => arc(Live::Bool(false)),
                1 => v.pop().expect("one"),
                _ => arc(Live::Disj(v.into())),
            }
        }
        _ => l.clone(),
    }
}

/// `toDNF`.
fn to_dnf(l: &L) -> EvalResult<L> {
    match &**l {
        Live::Neg(b) => match &**b {
            Live::State(_) | Live::Action(_) => Ok(l.clone()),
            _ => to_dnf(&push_neg(b)),
        },
        Live::Conj(xs) => {
            let temp = xs.iter().map(to_dnf).collect::<EvalResult<Vec<_>>>()?;
            let mut nes: Vec<L> = Vec::new();
            let mut total: i32 = 1;
            for elem in temp {
                match &*elem {
                    Live::Disj(ds) => {
                        total = total.checked_mul(ds.len() as i32).ok_or_else(|| {
                            let message = "because it exceeds the maximum supported size in disjunctive normal form.";
                            EvalError::runtime(Ec::TLC_LIVE_CANNOT_HANDLE_FORMULA, message.to_string())
                                .with_params(vec![message.to_string()])
                        })?;
                        nes.push(elem.clone());
                    }
                    Live::Conj(cs) => nes.extend(cs.iter().cloned()),
                    _ => nes.push(elem.clone()),
                }
            }
            if total == 1 {
                return Ok(arc(Live::Conj(nes.into())));
            }
            let total = total as usize;
            let mut res: Vec<Vec<L>> = (0..total).map(|_| Vec::new()).collect();
            let mut num = 1;
            let mut r_count = total;
            for ln in nes.iter() {
                if let Live::Disj(ds) = &**ln {
                    r_count /= ds.len();
                    let mut idx = 0;
                    for _ in 0..num {
                        for elem in ds.iter() {
                            for _ in 0..r_count {
                                add_conj(&mut res[idx], elem.clone());
                                idx += 1;
                            }
                        }
                    }
                    num *= ds.len();
                } else {
                    for r in res.iter_mut() {
                        add_conj(r, ln.clone());
                    }
                }
            }
            Ok(arc(Live::Disj(res.into_iter().map(|c| arc(Live::Conj(c.into()))).collect())))
        }
        Live::Disj(xs) => {
            let mut ae: Vec<L> = Vec::new();
            let mut res: Vec<L> = Vec::new();
            let take = |e: L, ae: &mut Vec<L>, res: &mut Vec<L>| match e.ae_body() {
                None => add_disj(res, e),
                Some(b) => add_disj(ae, b),
            };
            for x in xs.iter() {
                let elem = to_dnf(x)?;
                if let Live::Disj(ds) = &*elem {
                    for d in ds.iter() {
                        take(d.clone(), &mut ae, &mut res);
                    }
                } else {
                    take(elem, &mut ae, &mut res);
                }
            }
            match ae.len() {
                0 => {}
                1 => add_disj(&mut res, arc(Live::All(arc(Live::Even(ae.pop().expect("one")))))),
                _ => add_disj(&mut res, arc(Live::All(arc(Live::Even(arc(Live::Disj(ae.into()))))))),
            }
            Ok(arc(Live::Disj(res.into())))
        }
        _ => Ok(l.clone()),
    }
}

fn flatten_single_junctions(l: &L) -> L {
    match &**l {
        Live::Neg(b) => {
            if let Live::Neg(x) = &**b {
                flatten_single_junctions(x)
            } else {
                arc(Live::Neg(flatten_single_junctions(b)))
            }
        }
        Live::All(b) => arc(Live::All(flatten_single_junctions(b))),
        Live::Even(b) => arc(Live::Even(flatten_single_junctions(b))),
        Live::Next(b) => arc(Live::Next(flatten_single_junctions(b))),
        Live::Conj(xs) if xs.len() == 1 => flatten_single_junctions(&xs[0]),
        Live::Conj(xs) => conj_of(xs.iter().map(flatten_single_junctions)),
        Live::Disj(xs) if xs.len() == 1 => flatten_single_junctions(&xs[0]),
        Live::Disj(xs) => disj_of(xs.iter().map(flatten_single_junctions)),
        _ => l.clone(),
    }
}

/// `makeBinary`: junctions of at most two members, split in the middle.
fn make_binary(l: &L) -> L {
    match &**l {
        Live::Neg(b) => arc(Live::Neg(make_binary(b))),
        Live::All(b) => arc(Live::All(make_binary(b))),
        Live::Even(b) => arc(Live::Even(make_binary(b))),
        Live::Next(b) => arc(Live::Next(make_binary(b))),
        Live::Conj(xs) | Live::Disj(xs) => {
            if xs.len() == 1 {
                return make_binary(&xs[0]);
            }
            let is_conj = matches!(&**l, Live::Conj(_));
            let mid = xs.len() / 2;
            let (left, right) = if is_conj {
                (conj_of(xs[..mid].iter().cloned()), conj_of(xs[mid..].iter().cloned()))
            } else {
                (disj_of(xs[..mid].iter().cloned()), disj_of(xs[mid..].iter().cloned()))
            };
            if is_conj { conj2(make_binary(&left), make_binary(&right)) } else { disj2(make_binary(&left), make_binary(&right)) }
        }
        // `LNAction.makeBinary` throws; `classifyExpr` keeps actions out of a tableau formula.
        Live::Action(_) => unreachable!("LiveExprNode.makeBinary: TLC encounters actions."),
        _ => l.clone(),
    }
}

fn member(par: &[L], e: &Live) -> bool {
    par.iter().any(|x| e.equals(x))
}

/// `extractPromises(TBPar)`: every `<>P`, outermost first, once each.
fn extract_promises(l: &L, promises: &mut Vec<L>) {
    match &**l {
        Live::Even(b) => {
            if !member(promises, l) {
                promises.push(l.clone());
            }
            extract_promises(b, promises);
        }
        Live::Neg(b) | Live::All(b) | Live::Next(b) => extract_promises(b, promises),
        // Binary by now (`makeBinary`).
        Live::Conj(xs) | Live::Disj(xs) => {
            extract_promises(&xs[0], promises);
            extract_promises(&xs[1], promises);
        }
        _ => {}
    }
}

/// `OrderOfSolution.containsBoxOperator`.
fn contains_box(l: &Live) -> bool {
    match l {
        Live::All(_) => true,
        Live::Even(b) | Live::Neg(b) | Live::Next(b) => contains_box(b),
        Live::Conj(xs) | Live::Disj(xs) => xs.iter().any(|x| contains_box(x)),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------
// The particle tableau (`TBPar`, `TBParVec`, `TBTriple`, `TBGraph`).

struct Triple {
    a: L,
    b: L,
    c: L,
}

/// `TBPar.equals`: the same members, in any order.
fn par_equals(a: &[L], b: &[L]) -> bool {
    a.iter().all(|x| member(b, x)) && b.iter().all(|x| member(a, x))
}

/// `TBParVec.union`: `self`'s particles missing from `ps`, then `ps`.
fn par_union(a: Vec<Vec<L>>, b: Vec<Vec<L>>) -> Vec<Vec<L>> {
    let mut res: Vec<Vec<L>> = a.into_iter().filter(|p| !b.iter().any(|q| par_equals(p, q))).collect();
    res.extend(b);
    res
}

fn is_locally_consistent(par: &[L]) -> bool {
    let mut pos: Vec<&Live> = Vec::new();
    let mut neg: Vec<&Live> = Vec::new();
    for ln in par {
        match &**ln {
            Live::State(_) | Live::Bool(_) => pos.push(ln),
            Live::Neg(b) if matches!(&**b, Live::State(_) | Live::Bool(_)) => neg.push(b),
            _ => {}
        }
    }
    !pos.iter().any(|p| neg.iter().any(|n| p.equals(n)))
}

/// `positiveClosure`.
fn positive_closure(par: &[L]) -> EvalResult<Vec<L>> {
    let mut tps: Vec<L> = par.to_vec();
    let mut result = Vec::new();
    while let Some(ln) = tps.pop() {
        match &*ln {
            Live::Neg(b) => tps.push(b.clone()),
            Live::Next(b) => {
                result.push(ln.clone());
                tps.push(b.clone());
            }
            Live::Even(b) | Live::All(b) => {
                result.push(ln.clone());
                result.push(arc(Live::Next(ln.clone())));
                tps.push(b.clone());
            }
            Live::Conj(xs) | Live::Disj(xs) => {
                tps.extend(xs.iter().cloned());
                result.push(ln.clone());
            }
            Live::State(_) | Live::Bool(_) => result.push(ln.clone()),
            Live::Action(_) => return Err(EvalError::runtime(Ec::TLC_LIVE_ENCOUNTERED_ACTIONS, String::new())),
        }
    }
    Ok(result)
}

fn alpha_triples(par: &[L]) -> Vec<Triple> {
    let mut ts = Vec::new();
    for ln in par {
        match &**ln {
            Live::All(b) => ts.push(Triple { a: ln.clone(), b: b.clone(), c: arc(Live::Next(ln.clone())) }),
            Live::Conj(xs) => ts.push(Triple { a: ln.clone(), b: xs[0].clone(), c: xs[1].clone() }),
            _ => {}
        }
    }
    ts
}

fn beta_triples(par: &[L]) -> Vec<Triple> {
    let mut ts = Vec::new();
    for ln in par {
        match &**ln {
            Live::Even(b) => ts.push(Triple { a: ln.clone(), b: b.clone(), c: arc(Live::Next(ln.clone())) }),
            Live::Disj(xs) => ts.push(Triple { a: ln.clone(), b: xs[0].clone(), c: xs[1].clone() }),
            _ => {}
        }
    }
    ts
}

/// `TBPar.particleClosure()`.
fn particle_closure(terms: Vec<L>) -> EvalResult<Vec<Vec<L>>> {
    let positive = positive_closure(&terms)?;
    let alphas = alpha_triples(&positive);
    let betas = beta_triples(&positive);
    Ok(particle_closure_with(terms, &alphas, &betas))
}

fn particle_closure_with(terms: Vec<L>, alphas: &[Triple], betas: &[Triple]) -> Vec<Vec<L>> {
    if !is_locally_consistent(&terms) {
        return Vec::new();
    }
    let original = terms.len();
    let mut terms1 = terms;
    // `terms1 = terms1.append(..)` makes a new particle; until then `terms1` *is* `terms`, so
    // the alpha closure below grows the caller's particle and the size test after it sees
    // no growth.
    let mut appended = false;
    let mut i = 0;
    while i < terms1.len() {
        let ln = terms1[i].clone();
        let kappas = match &*ln {
            Live::All(b) => Some((b.clone(), arc(Live::Next(ln.clone())))),
            Live::Conj(xs) => Some((xs[0].clone(), xs[1].clone())),
            _ => None,
        };
        if let Some((k1, k2)) = kappas {
            if member(&terms1, &k1) {
                if !member(&terms1, &k2) {
                    terms1.push(k2);
                    appended = true;
                }
            } else if member(&terms1, &k2) {
                terms1.push(k1);
                appended = true;
            } else {
                terms1.push(k1);
                terms1.push(k2);
                appended = true;
            }
        }
        i += 1;
    }
    loop {
        let mut done = true;
        for alpha in alphas {
            if member(&terms1, &alpha.b) && member(&terms1, &alpha.c) && !member(&terms1, &alpha.a) {
                terms1.push(alpha.a.clone());
                done = false;
            }
        }
        if done {
            break;
        }
    }
    let _ = original;
    if appended && !is_locally_consistent(&terms1) {
        return Vec::new();
    }
    particle_closure_beta(terms1, alphas, betas)
}

fn particle_closure_beta(terms: Vec<L>, alphas: &[Triple], betas: &[Triple]) -> Vec<Vec<L>> {
    for ln in terms.iter() {
        let kappas = match &**ln {
            Live::Even(b) => Some((b.clone(), arc(Live::Next(ln.clone())))),
            Live::Disj(xs) => Some((xs[0].clone(), xs[1].clone())),
            _ => None,
        };
        if let Some((k1, k2)) = kappas
            && !member(&terms, &k1)
            && !member(&terms, &k2)
        {
            let mut t1 = terms.clone();
            t1.push(k1);
            let mut t2 = terms.clone();
            t2.push(k2);
            let ps1 = particle_closure_with(t1, alphas, betas);
            let ps2 = particle_closure_with(t2, alphas, betas);
            return par_union(ps1, ps2);
        }
    }
    for beta in betas {
        if (member(&terms, &beta.b) || member(&terms, &beta.c)) && !member(&terms, &beta.a) {
            let mut t = terms.clone();
            t.push(beta.a.clone());
            return particle_closure_with(t, alphas, betas);
        }
    }
    vec![terms]
}

/// `TBGraphNode`.
pub struct TableauNode {
    particle: Box<[L]>,
    successors: Box<[u32]>,
    /// The particle's members of state level or below (`statePreds`).
    state_preds: Box<[L]>,
}

impl TableauNode {
    pub fn successors(&self) -> &[u32] {
        &self.successors
    }
    /// `isAccepting`: the empty particle that loops to itself.
    pub fn is_accepting(&self, index: u32) -> bool {
        self.particle.is_empty() && self.successors.len() == 1 && self.successors[0] == index
    }
}

/// `TBGraph`: the nodes in index order; the first `init_count` are the initial ones.
pub struct Tableau {
    nodes: Box<[TableauNode]>,
    init_count: usize,
}

impl Tableau {
    fn new(tf: L) -> EvalResult<Tableau> {
        let mut pars: Vec<Vec<L>> = particle_closure(vec![tf])?;
        let init_count = pars.len();
        let mut succs: Vec<Vec<u32>> = Vec::new();
        let mut i = 0;
        while i < pars.len() {
            let imps: Vec<L> = pars[i].iter().filter_map(|ln| if let Live::Next(b) = &**ln { Some(b.clone()) } else { None }).collect();
            let mut nexts = Vec::new();
            for par in particle_closure(imps)? {
                let found = pars.iter().position(|p| par_equals(&par, p));
                let at = match found {
                    Some(j) => j,
                    None => {
                        pars.push(par);
                        pars.len() - 1
                    }
                };
                nexts.push(at as u32);
            }
            succs.push(nexts);
            i += 1;
        }
        let nodes = pars
            .into_iter()
            .zip(succs)
            .map(|(par, nexts)| {
                let state_preds = par.iter().filter(|ln| ln.level() <= VARIABLE).cloned().collect();
                TableauNode { particle: par.into(), successors: nexts.into(), state_preds }
            })
            .collect();
        Ok(Tableau { nodes, init_count })
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn init_count(&self) -> usize {
        self.init_count
    }
    pub fn node(&self, i: u32) -> &TableauNode {
        &self.nodes[i as usize]
    }
}

/// `PossibleErrorModel`: indices into its solution's check bins.
pub struct Pem {
    /// `[]<>A`.
    pub ae_action: Box<[u32]>,
    /// `[]<>S`.
    pub ae_state: Box<[u32]>,
    /// `<>[]A`.
    pub ea_action: Box<[u32]>,
}

impl Pem {
    pub fn is_empty(&self) -> bool {
        self.ea_action.is_empty() && self.ae_state.is_empty() && self.ae_action.is_empty()
    }
}

/// One `OrderOfSolution`.
pub struct Solution {
    pub tableau: Option<Tableau>,
    promises: Box<[L]>,
    contains_box_in_promise: bool,
    pub pems: Box<[Pem]>,
    check_state: Box<[L]>,
    check_action: Box<[L]>,
}

impl Solution {
    pub fn check_state_len(&self) -> usize {
        self.check_state.len()
    }
    pub fn check_action_len(&self) -> usize {
        self.check_action.len()
    }
    pub fn promise_count(&self) -> usize {
        self.promises.len()
    }
    pub fn has_empty_pem(&self) -> bool {
        self.pems.iter().any(Pem::is_empty)
    }
    /// `hasEmptyPEMAndBoxFreePromises`.
    pub fn has_empty_pem_and_box_free_promises(&self) -> bool {
        !self.contains_box_in_promise && self.pems.iter().all(Pem::is_empty)
    }
    /// `TBPar.isFulfilling(promise)` for the particle of tableau node `node`.
    pub fn is_fulfilling(&self, node: u32, promise: usize) -> bool {
        let tableau = self.tableau.as_ref().expect("promises come from a tableau formula");
        let par = &tableau.node(node).particle;
        let Live::Even(body) = &*self.promises[promise] else { unreachable!("a promise is <>P") };
        !member(par, &self.promises[promise]) || member(par, body)
    }
}

/// Every liveness obligation of the spec, in TLC's solution order.
pub struct LiveSpec {
    pub solutions: Box<[Solution]>,
}

/// `OSExprPem`.
#[derive(Default)]
struct BranchPem {
    ea_action: Vec<L>,
    ae_state: Vec<L>,
    ae_action: Vec<L>,
    tfs: Vec<L>,
}

/// `addToBin`: the index of the first equal check, appending it when there is none.
fn add_to_bin(checks: &[L], bin: &mut Vec<L>) -> Box<[u32]> {
    checks
        .iter()
        .map(|c| match bin.iter().position(|b| c.equals(b)) {
            Some(i) => i as u32,
            None => {
                bin.push(c.clone());
                (bin.len() - 1) as u32
            }
        })
        .collect()
}

fn runtime(ev: &Ev<'_>, ec: Ec, params: &[&str]) -> EvalError {
    EvalError::runtime(ec, ev.out().text(ec, params)).with_params(params.iter().map(|p| p.to_string()).collect())
}

/// What `catch (Exception e)` catches: everything but a Java `Error`.
fn is_exception(e: &EvalError) -> bool {
    !matches!(e.kind, EvalErrorKind::StackOverflow | EvalErrorKind::Assertion)
}

impl<'t> Ev<'t> {
    /// `Liveness.processLiveness`; `None` when there is nothing to check. Prints
    /// "Implied-temporal checking--satisfiability problem has N branches".
    pub fn live_spec(&mut self) -> EvalResult<Option<LiveSpec>> {
        let Some(lexpr) = self.parse_liveness()? else { return Ok(None) };
        let lexpr = to_dnf(&simplify(&push_neg_deep(&lexpr, false)))?;
        if let Live::Bool(false) = &*lexpr {
            return Ok(Some(LiveSpec { solutions: Box::new([]) }));
        }
        let dnf: Vec<L> = match &*lexpr {
            Live::Disj(xs) => xs.to_vec(),
            _ => vec![lexpr.clone()],
        };
        let mut pems = Vec::with_capacity(dnf.len());
        let mut tfs: Vec<Option<L>> = Vec::with_capacity(dnf.len());
        for body in dnf.iter() {
            let ln = flatten_single_junctions(body);
            let mut pem = BranchPem::default();
            match &*ln {
                Live::Conj(xs) => {
                    for x in xs.iter() {
                        self.classify(x, &mut pem)?;
                    }
                }
                _ => self.classify(&ln, &mut pem)?,
            }
            tfs.push(match pem.tfs.len() {
                0 => None,
                1 => Some(pem.tfs[0].clone()),
                _ => Some(conj_of(pem.tfs.iter().cloned())),
            });
            pems.push(pem);
        }
        // Lump the branches whose tableau formulas are syntactically equal.
        let mut tf_bin: Vec<Option<L>> = Vec::new();
        let mut pem_bin: Vec<Vec<usize>> = Vec::new();
        for (i, tf) in tfs.iter().enumerate() {
            let found = tf_bin.iter().position(|t| match (t, tf) {
                (None, None) => true,
                (Some(a), Some(b)) => b.equals(a),
                _ => false,
            });
            let at = found.unwrap_or_else(|| {
                tf_bin.push(tf.clone());
                pem_bin.push(Vec::new());
                tf_bin.len() - 1
            });
            pem_bin[at].push(i);
        }
        let mut solutions = Vec::with_capacity(tf_bin.len());
        for (tf, members) in tf_bin.into_iter().zip(pem_bin) {
            let (tableau, promises) = match tf {
                None => (None, Vec::new()),
                Some(tf) => {
                    let tf1 = make_binary(&tf);
                    let tableau = Tableau::new(tf1.clone())?;
                    let mut promises = Vec::new();
                    extract_promises(&tf1, &mut promises);
                    (Some(tableau), promises)
                }
            };
            let contains_box_in_promise = promises.iter().any(|p| match &**p {
                Live::Even(b) => contains_box(b),
                _ => false,
            });
            let mut state_bin = Vec::new();
            let mut action_bin = Vec::new();
            let pems = members
                .into_iter()
                .map(|i| {
                    let p = &pems[i];
                    let ae_action = add_to_bin(&p.ae_action, &mut action_bin);
                    let ae_state = add_to_bin(&p.ae_state, &mut state_bin);
                    let ea_action = add_to_bin(&p.ea_action, &mut action_bin);
                    Pem { ae_action, ae_state, ea_action }
                })
                .collect();
            solutions.push(Solution {
                tableau,
                promises: promises.into(),
                contains_box_in_promise,
                pems,
                check_state: state_bin.into(),
                check_action: action_bin.into(),
            });
        }
        self.out().message(Ec::TLC_LIVE_IMPLIED, &[&solutions.len().to_string()]);
        Ok(Some(LiveSpec { solutions: solutions.into() }))
    }

    /// `classifyExpr`.
    fn classify(&mut self, ln: &L, pem: &mut BranchPem) -> EvalResult<()> {
        match &**ln {
            Live::Even(b) => {
                if let Live::All(x) = &**b
                    && x.level() < TEMPORAL
                {
                    pem.ea_action.push(x.clone());
                    return Ok(());
                }
            }
            Live::All(b) => {
                if let Live::Even(x) = &**b {
                    let level = x.level();
                    if level <= VARIABLE {
                        pem.ae_state.push(x.clone());
                        return Ok(());
                    }
                    if level == ACTION {
                        pem.ae_action.push(x.clone());
                        return Ok(());
                    }
                }
            }
            _ => {}
        }
        if ln.contains_action() {
            return Err(runtime(self, Ec::TLC_LIVE_WRONG_FORMULA_FORMAT, &[]));
        }
        pem.tfs.push(ln.clone());
        Ok(())
    }

    /// `parseLiveness`: `livespec /\ ~livecheck`, or `None` with neither.
    fn parse_liveness(&mut self) -> EvalResult<Option<L>> {
        let tool = self.tool();
        let mut fairs = Vec::new();
        for (expr, ctx) in tool.temporal().fairness.iter() {
            fairs.push(self.ast_to_live(*expr, ctx)?);
        }
        let checks = &tool.temporal().properties;
        let lnc = || conj_of(fairs.clone());
        Ok(match checks.len() {
            0 if fairs.is_empty() => None,
            0 => Some(lnc()),
            1 => {
                let ln = self.ast_to_live(checks[0].expr, &checks[0].ctx)?;
                let neg = arc(Live::Neg(ln));
                if fairs.is_empty() { Some(neg) } else { Some(conj_of(fairs.iter().cloned().chain([neg]))) }
            }
            _ => {
                let mut negs = Vec::new();
                for c in checks.iter() {
                    negs.push(arc(Live::Neg(self.ast_to_live(c.expr, &c.ctx)?)));
                }
                let lnd = disj_of(negs);
                if fairs.is_empty() { Some(lnd) } else { Some(conj_of(fairs.iter().cloned().chain([lnd]))) }
            }
        })
    }

    fn cannot_handle(&self, expr: ExprId) -> EvalError {
        let text = self.tool().graph().expr_tlc_text(expr);
        runtime(self, Ec::TLC_LIVE_CANNOT_HANDLE_FORMULA, &[&text])
    }

    /// `astToLive(tool, expr, con, level)`.
    fn ast_to_live_at(&mut self, expr: ExprId, c: &Ctx, level: u8) -> EvalResult<L> {
        let level = if level == CONSTANT { self.level_bound(expr, c)? } else { level };
        match level {
            CONSTANT => match self.eval(expr, c, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)? {
                Value::Bool(b) => Ok(arc(Live::Bool(b))),
                _ => {
                    let text = self.tool().graph().expr_tlc_text(expr);
                    Err(runtime(self, Ec::TLC_EXPECTED_VALUE, &["boolean", &text]))
                }
            },
            VARIABLE => Ok(arc(Live::State(Arc::new(StateAtom::Ast { expr, ctx: c.clone() })))),
            _ => Ok(arc(Live::Action(Arc::new(ActionAtom { body: expr, ctx: c.clone(), subscript: None, is_box: false })))),
        }
    }

    /// The level-based fallback: `Specs.getLevel`, then a failure above action level.
    fn ast_to_live_leveled(&mut self, expr: ExprId, c: &Ctx) -> EvalResult<L> {
        let level = self.specs_level(expr, c);
        if level > ACTION {
            return Err(self.cannot_handle(expr));
        }
        self.ast_to_live_at(expr, c, level)
    }

    /// `astToLive(tool, expr, con)`.
    /// The recursion shares the evaluator's depth guard: a RECURSIVE operator whose guard
    /// depends on a state never stops expanding, and TLC ends in a `StackOverflowError`.
    fn ast_to_live(&mut self, expr: ExprId, c: &Ctx) -> EvalResult<L> {
        self.enter(expr)?;
        let r = self.ast_to_live_inner(expr, c);
        self.leave(r.as_ref().err().map(|e| &e.kind));
        r
    }

    fn ast_to_live_inner(&mut self, expr: ExprId, c: &Ctx) -> EvalResult<L> {
        let graph = self.tool().graph();
        match graph.expr(expr) {
            ExprNode::OpAppl { .. } => self.ast_to_live_appl(expr, c),
            // The LET's definitions are not bound; a use of one resolves to the definition.
            ExprNode::LetIn { body, .. } => self.ast_to_live(*body, c),
            ExprNode::SubstIn { substs, body, .. } | ExprNode::APSubstIn { substs, body, .. } => {
                let body = *body;
                let c1 = self.subst_context(substs, c);
                self.ast_to_live(body, &c1)
            }
            ExprNode::Label { body, .. } => self.ast_to_live(*body, c),
            _ => self.ast_to_live_leveled(expr, c),
        }
    }

    fn ast_to_live_appl(&mut self, expr: ExprId, c: &Ctx) -> EvalResult<L> {
        let graph = self.tool().graph();
        let ExprNode::OpAppl { op, args, bound, .. } = graph.expr(expr) else { unreachable!() };
        let (op, args) = (*op, &**args);
        let mut code = self.opcode(op);
        if code.is_none() {
            match self.lookup(op, c, StateRef::Empty, false) {
                Resolved::UserOp(d) | Resolved::Binding(Binding::OpDef(d)) => {
                    code = self.opcode(SymbolRef::OpDef(d));
                    let recursive = code.is_none() && graph.op_def(d).recursive;
                    let expand = code.is_none() && (!recursive || self.level_bound(expr, c)? > ACTION);
                    if expand {
                        let expanded = (|| {
                            let params = graph.op_def(d).params.clone();
                            let mut c1 = c.clone();
                            for (&p, a) in params.iter().zip(args.iter()) {
                                let v = self.get_val(*a, c, false);
                                c1 = c1.cons(SymbolRef::Param(p), v);
                            }
                            let body = graph.op_def(d).body.expect("a user definition has a body");
                            let res = self.ast_to_live(body, &c1)?;
                            let level = res.level();
                            if level > ACTION {
                                return Ok(res);
                            }
                            self.ast_to_live_at(expr, c, level)
                        })();
                        match expanded {
                            Ok(l) => return Ok(l),
                            Err(e) if recursive || !is_exception(&e) => return Err(e),
                            Err(_) => {}
                        }
                    }
                }
                Resolved::Value(Value::Bool(b)) | Resolved::Binding(Binding::Value(Value::Bool(b))) => return Ok(arc(Live::Bool(b))),
                _ => {}
            }
            if code.is_none() {
                return self.ast_to_live_leveled(expr, c);
            }
        }
        let code = code.expect("a built-in by now");
        let arg = |i: usize| args[i].expect_expr();
        match code {
            Builtin::BoundedExists | Builtin::BoundedForall => {
                let forall = code == Builtin::BoundedForall;
                let bound = bound.as_deref().expect("a bounded quantifier has bounds");
                let body = arg(0);
                let junction = (|| {
                    let mut contexts = self.contexts(bound, c, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)?;
                    let mut kids = Vec::new();
                    while let Some(c1) = contexts.next(self)? {
                        kids.push(self.ast_to_live(body, &c1)?);
                    }
                    if kids.is_empty() {
                        // An empty domain: a contradiction or a tautology.
                        return Ok(arc(Live::Bool(forall)));
                    }
                    let res = if forall { conj_of(kids) } else { disj_of(kids) };
                    let level = res.level();
                    if level > ACTION {
                        return Ok(res);
                    }
                    self.ast_to_live_at(expr, c, level)
                })();
                match junction {
                    Ok(l) => Ok(l),
                    Err(e) if !is_exception(&e) => Err(e),
                    Err(e) => {
                        let level = self.specs_level(expr, c);
                        if level > ACTION {
                            let text = self.tool().graph().expr_tlc_text(expr);
                            // `\A` adds the exception's message when it is a `TLCRuntimeException`.
                            if forall && matches!(e.kind, EvalErrorKind::Runtime) {
                                return Err(runtime(self, Ec::TLC_LIVE_CANNOT_HANDLE_FORMULA, &[&text, &e.message]));
                            }
                            return Err(runtime(self, Ec::TLC_LIVE_CANNOT_HANDLE_FORMULA, &[&text]));
                        }
                        self.ast_to_live_at(expr, c, level)
                    }
                }
            }
            Builtin::ConjList | Builtin::Land | Builtin::DisjList | Builtin::Lor => {
                let conj = matches!(code, Builtin::ConjList | Builtin::Land);
                let mut kids = Vec::with_capacity(args.len());
                for a in args.iter() {
                    kids.push(self.ast_to_live(a.expect_expr(), c)?);
                }
                let res = if conj { conj_of(kids) } else { disj_of(kids) };
                let level = res.level();
                if level > ACTION {
                    return Ok(res);
                }
                self.ast_to_live_at(expr, c, level)
            }
            Builtin::FcnApply => {
                // A function of temporal formulas, `[i \in S |-> <>P(i)][k]`: the body
                // stands in, in the *outer* context (TLC builds the function's context and
                // does not use it). Any failure falls through to the level-based handling.
                let body = (|| {
                    if let Value::FcnLambda(f) = self.eval(arg(0), c, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)?
                        && f.converted.get().is_none()
                    {
                        self.fcn_context(&f, args, c, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR)?;
                        return self.ast_to_live(f.body, c).map(Some);
                    }
                    Ok(None)
                })();
                match body {
                    Ok(Some(l)) => return Ok(l),
                    Err(e) if !is_exception(&e) => return Err(e),
                    _ => {}
                }
                let level = self.tool().graph().level(expr) as u8;
                if level > ACTION {
                    return Err(self.cannot_handle(expr));
                }
                self.ast_to_live_at(expr, c, level)
            }
            Builtin::IfThenElse => {
                let chosen = match self.eval(arg(0), c, StateRef::Empty, StateRef::Empty, EvalControl::CLEAR) {
                    Ok(Value::Bool(b)) => Some(if b { arg(1) } else { arg(2) }),
                    Err(e) if !is_exception(&e) => return Err(e),
                    _ => None,
                };
                if let Some(e) = chosen {
                    return self.ast_to_live(e, c);
                }
                let guard = self.ast_to_live(arg(0), c)?;
                let e1 = self.ast_to_live(arg(1), c)?;
                let e2 = self.ast_to_live(arg(2), c)?;
                let res = disj2(conj2(guard.clone(), e1), conj2(arc(Live::Neg(guard)), e2));
                let level = res.level();
                if level > ACTION {
                    return Ok(res);
                }
                self.ast_to_live_at(expr, c, level)
            }
            Builtin::Lnot => {
                let a = self.ast_to_live(arg(0), c)?;
                let level = a.level();
                if level > ACTION {
                    return Ok(arc(Live::Neg(a)));
                }
                self.ast_to_live_at(expr, c, level)
            }
            Builtin::Implies => {
                let left = self.ast_to_live(arg(0), c)?;
                let right = self.ast_to_live(arg(1), c)?;
                let level = left.level().max(right.level());
                if level > ACTION {
                    return Ok(disj2(arc(Live::Neg(left)), right));
                }
                self.ast_to_live_at(expr, c, level)
            }
            Builtin::Prime => Ok(arc(Live::Action(Arc::new(ActionAtom { body: expr, ctx: c.clone(), subscript: None, is_box: false })))),
            // `SF_e(A)` is `<>[]~ENABLED <A>_e \/ []<><A>_e`, `WF_e(A)` is
            // `[]<>(~ENABLED <A>_e \/ <A>_e)`.
            Builtin::Unimplemented(op @ ("$SF" | "$WF")) => {
                let (subs, body) = (arg(0), arg(1));
                let en = arc(Live::Neg(arc(Live::State(Arc::new(StateAtom::Enabled {
                    pred: body,
                    ctx: c.clone(),
                    subscript: Some(subs),
                    is_box: false,
                })))));
                let act = arc(Live::Action(Arc::new(ActionAtom { body, ctx: c.clone(), subscript: Some(subs), is_box: false })));
                if op == "$SF" {
                    Ok(disj2(arc(Live::Even(arc(Live::All(en)))), arc(Live::All(arc(Live::Even(act))))))
                } else {
                    Ok(arc(Live::All(arc(Live::Even(disj2(en, act))))))
                }
            }
            // `F ~> G` is `[](~F \/ <>G)`.
            Builtin::Unimplemented("~>") => {
                let left = self.ast_to_live(arg(0), c)?;
                let right = self.ast_to_live(arg(1), c)?;
                Ok(arc(Live::All(disj2(arc(Live::Neg(left)), arc(Live::Even(right))))))
            }
            Builtin::Box => Ok(arc(Live::All(self.ast_to_live(arg(0), c)?))),
            Builtin::Unimplemented("<>") => Ok(arc(Live::Even(self.ast_to_live(arg(0), c)?))),
            Builtin::AngleAct => {
                Ok(arc(Live::Action(Arc::new(ActionAtom { body: arg(0), ctx: c.clone(), subscript: Some(arg(1)), is_box: false }))))
            }
            Builtin::Nop => self.ast_to_live(arg(0), c),
            // CASE included: TLC rejects it in a temporal formula rather than pick a branch.
            _ => self.ast_to_live_leveled(expr, c),
        }
    }

    /// `Specs.getLevel(expr, c)`: SANY's level, raised by the level of what `c` binds the
    /// expression's level parameters to.
    fn specs_level(&mut self, expr: ExprId, c: &Ctx) -> u8 {
        let graph = self.tool().graph();
        let mut level = graph.level(expr) as u8;
        for &p in graph.level_params(expr) {
            match c.lookup_cutoff(p, true) {
                Some(Binding::Thunk(t)) => {
                    let (e, tc) = (t.expr, t.ctx.clone());
                    level = level.max(self.specs_level(e, &tc));
                }
                Some(Binding::OpDef(d)) => level = level.max(graph.op_level(*d) as u8),
                _ => {}
            }
        }
        level
    }

    /// `LiveExprNode.eval(tool, s1, s2)`.
    fn live_eval(&mut self, l: &Live, s1: StateRef<'_>, s2: StateRef<'_>) -> EvalResult<bool> {
        match l {
            Live::Bool(b) => Ok(*b),
            Live::State(atom) => self.state_atom_eval(atom, s1),
            Live::Action(atom) => self.action_atom_eval(atom, s1, s2),
            Live::Neg(b) => Ok(!self.live_eval(b, s1, s2)?),
            Live::Conj(xs) => {
                for x in xs.iter() {
                    if !self.live_eval(x, s1, s2)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Live::Disj(xs) => {
                for x in xs.iter() {
                    if self.live_eval(x, s1, s2)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Live::All(_) => Err(runtime(self, Ec::TLC_LIVE_CANNOT_EVAL_FORMULA, &["[]"])),
            Live::Even(_) => Err(runtime(self, Ec::TLC_LIVE_CANNOT_EVAL_FORMULA, &["<>"])),
            Live::Next(b) => self.live_eval(b, s2, StateRef::Empty),
        }
    }

    fn state_atom_eval(&mut self, atom: &StateAtom, s: StateRef<'_>) -> EvalResult<bool> {
        match atom {
            StateAtom::Ast { expr, ctx } => match self.eval(*expr, ctx, s, StateRef::Empty, EvalControl::CLEAR)? {
                Value::Bool(b) => Ok(b),
                _ => Err(runtime(self, Ec::TLC_LIVE_STATE_PREDICATE_NON_BOOL, &[])),
            },
            StateAtom::Enabled { pred, ctx, subscript, is_box } => {
                if *is_box && subscript.is_some() {
                    return Ok(true);
                }
                let c1 = ctx.branch();
                match subscript {
                    Some(sub) => self.enabled_changed(*pred, &c1, s, *sub),
                    None => self.enabled_plain(*pred, &c1, s),
                }
            }
        }
    }

    fn action_atom_eval(&mut self, atom: &ActionAtom, s1: StateRef<'_>, s2: StateRef<'_>) -> EvalResult<bool> {
        if let Some(sub) = atom.subscript {
            let v1 = self.eval(sub, &atom.ctx, s1, StateRef::Empty, EvalControl::CLEAR)?;
            let v2 = self.eval(sub, &atom.ctx, s2, StateRef::Null, EvalControl::CLEAR)?;
            let stutters = v1.tlc_equals(self, &v2)?;
            if stutters {
                return Ok(atom.is_box);
            }
        }
        match self.eval(atom.body, &atom.ctx, s1, s2, EvalControl::CLEAR)? {
            Value::Bool(b) => Ok(b),
            _ => Err(runtime(self, Ec::TLC_LIVE_ENCOUNTERED_NONBOOL_PREDICATE, &[])),
        }
    }

    /// `OrderOfSolution.checkState(tool, s)`.
    pub fn live_state_bits(&mut self, sol: &Solution, s: &State) -> EvalResult<Box<[bool]>> {
        sol.check_state.iter().map(|l| self.live_eval(l, StateRef::Full(s), StateRef::Null)).collect()
    }

    /// `OrderOfSolution.checkAction(tool, s0, s1, ...)`.
    pub fn live_action_bits(&mut self, sol: &Solution, s0: &State, s1: &State) -> EvalResult<Box<[bool]>> {
        sol.check_action.iter().map(|l| self.live_eval(l, StateRef::Full(s0), StateRef::Full(s1))).collect()
    }

    /// `TBGraphNode.isConsistent(state, tool)`.
    pub fn tableau_consistent(&mut self, node: &TableauNode, s: &State) -> EvalResult<bool> {
        for p in node.state_preds.iter() {
            if !self.live_eval(p, StateRef::Full(s), StateRef::Null)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `Liveness.findViolatedProperties`: each PROPERTY's temporal formula evaluated on the
    /// lasso `states` whose cycle starts at `cycle_pos`; the names of those that fail.
    pub fn violated_properties(&mut self, states: &[State], cycle_pos: usize) -> EvalResult<Vec<Atom>> {
        let mut violated: Vec<Atom> = Vec::new();
        let n = self.tool().temporal().properties.len();
        for i in 0..n {
            let (name, expr, ctx) = {
                let t = &self.tool().temporal().properties[i];
                (t.name, t.expr, t.ctx.clone())
            };
            let held = self.ast_to_live(expr, &ctx).and_then(|prop| self.eval_on_lasso(&prop, states, cycle_pos, 0));
            match held {
                Ok(true) => {}
                Ok(false) => {
                    if !violated.contains(&name) {
                        violated.push(name);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(violated)
    }

    /// `LiveExprNode.evalOnLasso`.
    fn eval_on_lasso(&mut self, l: &Live, states: &[State], cycle: usize, pos: usize) -> EvalResult<bool> {
        let len = states.len();
        match l {
            Live::Bool(b) => Ok(*b),
            Live::State(atom) => self.state_atom_eval(atom, StateRef::Full(&states[pos])),
            Live::Action(atom) => {
                let s2 = if pos + 1 < len { &states[pos + 1] } else { &states[cycle] };
                self.action_atom_eval(atom, StateRef::Full(&states[pos]), StateRef::Full(s2))
            }
            Live::Neg(b) => Ok(!self.eval_on_lasso(b, states, cycle, pos)?),
            Live::Conj(xs) => {
                for x in xs.iter() {
                    if !self.eval_on_lasso(x, states, cycle, pos)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Live::Disj(xs) => {
                for x in xs.iter() {
                    if self.eval_on_lasso(x, states, cycle, pos)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Live::All(b) => {
                for i in pos..cycle {
                    if !self.eval_on_lasso(b, states, cycle, i)? {
                        return Ok(false);
                    }
                }
                for i in cycle..len {
                    if !self.eval_on_lasso(b, states, cycle, i)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Live::Even(b) => {
                for i in pos..cycle {
                    if self.eval_on_lasso(b, states, cycle, i)? {
                        return Ok(true);
                    }
                }
                for i in cycle..len {
                    if self.eval_on_lasso(b, states, cycle, i)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Live::Next(b) => {
                let next = if pos + 1 < len { pos + 1 } else { cycle };
                self.eval_on_lasso(b, states, cycle, next)
            }
        }
    }
}
