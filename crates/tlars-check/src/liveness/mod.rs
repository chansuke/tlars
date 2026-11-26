//! The graph side of `tlc2.tool.liveness`: `LiveCheck` (one behaviour graph per solution,
//! crossed with its tableau when it has one) and `LiveWorker` (the SCC search per PEM and
//! the lasso of a violation). The formula side (`Liveness.processLiveness`, PEMs, tableau)
//! is `tlars_tool::LiveSpec`; this module sees only check indices and bit vectors.
//! TLC keeps the graphs on disk; here they are in memory (DESIGN.md), in TLC's order.

mod graph;
mod set_of_states;
mod worker;

pub(crate) use set_of_states::SetOfStates;

use crate::queue::Queued;
use graph::{DiskGraph, Graph, TableauDiskGraph};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use tlars_msg::Ec;
use tlars_tool::{Ev, EvalError, EvalResult, Fp, LiveSpec, PartialState, Solution, State, StateInfo, StateRef};

/// One `ILiveChecker`: its graph, and the node `addNextState` found to close a violation
/// while it built the graph (`errorGraphNode`).
struct Checker {
    graph: Graph,
    error_node: Option<(u64, i32)>,
}

pub(crate) struct LiveCheck {
    spec: LiveSpec,
    checkers: Box<[Mutex<Checker>]>,
}

/// How `addNextState` ended.
pub(crate) enum Added {
    Done,
    /// `printErrorTrace` printed a violation that needs no cycle and threw
    /// `InvariantViolatedException`: the run stops with `TLC_INVARIANT_VIOLATED_BEHAVIOR`.
    Violated,
}

/// How `check0` ended.
pub(crate) enum CheckOutcome {
    /// `EC.NO_ERROR` or `EC.TLC_TEMPORAL_PROPERTY_VIOLATED`.
    Done(Ec),
    /// A `LiveWorker` failed; TLC prints the error and calls `System.exit(1)`.
    Failed(EvalError),
}

/// `Assert.check(TLCGlobals.mainChecker == null, EC.GENERAL)`: a state expanded twice.
fn assert_general(ev: &Ev<'_>) -> EvalError {
    EvalError::runtime(Ec::GENERAL, ev.out().text(Ec::GENERAL, &[]))
}

impl LiveCheck {
    /// One graph per solution; `Ev::live_spec` already printed the branch count.
    pub(crate) fn new(spec: LiveSpec) -> Self {
        let checkers = spec
            .solutions
            .iter()
            .map(|sol| {
                let graph = match sol.tableau {
                    None => Graph::Plain(DiskGraph::new()),
                    Some(_) => Graph::Tableau(TableauDiskGraph::new()),
                };
                Mutex::new(Checker { graph, error_node: None })
            })
            .collect();
        LiveCheck { spec, checkers }
    }

    /// `getNumChecker()`.
    pub(crate) fn num_checkers(&self) -> usize {
        self.checkers.len()
    }

    /// `addInitState`: the state's node, or one node per initial tableau node the state is
    /// consistent with.
    pub(crate) fn add_init_state(&self, ev: &mut Ev<'_>, state: &State, fp: Fp) -> EvalResult<()> {
        for (sol, checker) in self.spec.solutions.iter().zip(self.checkers.iter()) {
            match &sol.tableau {
                None => {
                    let mut c = checker.lock().expect("liveness checker");
                    let Graph::Plain(g) = &mut c.graph else { unreachable!("a solution without a tableau") };
                    g.add_init_node(fp.raw(), -1);
                }
                Some(tableau) => {
                    for i in 0..tableau.init_count() as u32 {
                        if ev.tableau_consistent(tableau.node(i), state)? {
                            let mut c = checker.lock().expect("liveness checker");
                            let Graph::Tableau(g) = &mut c.graph else { unreachable!("a solution with a tableau") };
                            g.add_init_node(fp.raw(), i as i32);
                            g.record_node(fp.raw(), i as i32);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// `addNextState`: `cur`'s edges to its successors (the stuttering step among them), in
    /// slot order. The checks are evaluated before the graph is locked.
    /// `printed` is `printedLivenessErrorStack`, which only the first violation sets.
    pub(crate) fn add_next_state(&self, ev: &mut Ev<'_>, cur: &Queued, succs: &SetOfStates, printed: &AtomicBool) -> EvalResult<Added> {
        let s0 = &cur.state;
        let fp0 = cur.fp.raw();
        let succ_list: Vec<&(Fp, State)> = succs.iter_slot_order().collect();
        let n = succ_list.len();
        for (sol, checker) in self.spec.solutions.iter().zip(self.checkers.iter()) {
            let alen = sol.check_action_len();
            let mut action_bits: Vec<bool> = Vec::with_capacity(alen * n);
            for (_, s1) in succ_list.iter() {
                action_bits.extend(ev.live_action_bits(sol, s0, s1)?.iter());
            }
            let state_bits = ev.live_state_bits(sol, s0)?;
            let slen = state_bits.len();
            match &sol.tableau {
                None => {
                    let mut c = checker.lock().expect("liveness checker");
                    let Graph::Plain(g) = &mut c.graph else { unreachable!("a solution without a tableau") };
                    let mut node0 = g.get_node(fp0);
                    let s = node0.succ_size();
                    node0.set_check_state(&state_bits);
                    for (sidx, (fp1, _)) in succ_list.iter().enumerate() {
                        let successor = fp1.raw();
                        let ptr1 = g.get_ptr(successor);
                        if ptr1 == -1 || !node0.trans_exists(successor, -1) {
                            node0.add_transition(successor, -1, slen, alen, Some(&action_bits[sidx * alen..(sidx + 1) * alen]));
                        }
                    }
                    if (s == 0 && s == node0.succ_size()) || s < node0.succ_size() {
                        g.add_node(node0);
                    } else {
                        return Err(assert_general(ev));
                    }
                }
                Some(tableau) => {
                    let mut consistency = vec![false; tableau.len() * n];
                    for t in 0..tableau.len() {
                        for (sidx, (_, s1)) in succ_list.iter().enumerate() {
                            if ev.tableau_consistent(tableau.node(t as u32), s1)? {
                                consistency[t * n + sidx] = true;
                            }
                        }
                    }
                    let mut c = checker.lock().expect("liveness checker");
                    let Checker { graph, error_node } = &mut *c;
                    let Graph::Tableau(g) = graph else { unreachable!("a solution with a tableau") };
                    let tidxs = g.set_done(fp0);
                    for tidx0 in tidxs {
                        let tnode0 = tableau.node(tidx0 as u32);
                        let mut node0 = g.get_node(fp0, tidx0);
                        let s = node0.succ_size();
                        node0.set_check_state(&state_bits);
                        for (sidx, (fp1, s1)) in succ_list.iter().enumerate() {
                            let successor = fp1.raw();
                            let is_done = g.is_done(successor);
                            for &t1 in tnode0.successors() {
                                let t1 = t1 as i32;
                                let ptr1 = g.get_ptr(successor, t1);
                                if consistency[t1 as usize * n + sidx] && (ptr1 == -1 || !node0.trans_exists(successor, t1)) {
                                    node0.add_transition(successor, t1, slen, alen, Some(&action_bits[sidx * alen..(sidx + 1) * alen]));
                                    if ptr1 == -1 {
                                        g.record_node(successor, t1);
                                        if is_done {
                                            late_expand(ev, g, sol, error_node, s1, successor, t1)?;
                                        }
                                    }
                                }
                            }
                        }
                        if (s == 0 && s == node0.succ_size()) || s < node0.succ_size() {
                            g.add_node(node0);
                        } else {
                            return Err(assert_general(ev));
                        }
                    }
                    let Some((efp, etidx)) = *error_node else { continue };
                    let prefix = g.get_path(efp, etidx).map_err(path_error)?;
                    drop(c);
                    // `printErrorTrace`: once a worker has printed a liveness error, the
                    // others carry on as if nothing had been found.
                    if printed.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        continue;
                    }
                    print_error_trace(ev, &prefix)?;
                    checker.lock().expect("liveness checker").error_node = None;
                    return Ok(Added::Violated);
                }
            }
        }
        Ok(Added::Done)
    }

    /// `check0(tool, true)`, the final check: 2192, the `LiveWorker`s, 2267.
    pub(crate) fn final_check(&self, ev: &mut Ev<'_>, clock: &crate::Clock) -> CheckOutcome {
        // `"off".equals(TLCGlobals.lnCheck)`: the graph was built, and only the checks made
        // while building it count.
        if ev.tool().globals().lncheck == tlars_tool::LnCheck::Off {
            return CheckOutcome::Done(Ec::NO_ERROR);
        }
        let out = ev.out();
        let sum: usize = self.checkers.iter().map(|c| c.lock().expect("liveness checker").graph.size()).sum();
        let branches = if self.checkers.len() == 1 { String::new() } else { format!("{} branches of ", self.checkers.len()) };
        out.message(Ec::TLC_CHECKING_TEMPORAL_PROPS, &["complete", &sum.to_string(), &branches]);
        let started = clock.now_millis();
        // One `LiveWorker` takes the checkers in order and stops at the first violation, as
        // `-workers 1` does.
        let mut violated = false;
        'checkers: for (sol, checker) in self.spec.solutions.iter().zip(self.checkers.iter()) {
            let mut c = checker.lock().expect("liveness checker");
            for pem in sol.pems.iter() {
                match worker::check_sccs(&mut c.graph, sol, pem) {
                    Ok(None) => {}
                    Ok(Some(lasso)) => {
                        drop(c);
                        if let Err(e) = print_trace(ev, sol, &lasso) {
                            return CheckOutcome::Failed(e);
                        }
                        violated = true;
                        break 'checkers;
                    }
                    Err(message) => return CheckOutcome::Failed(path_error(message)),
                }
            }
        }
        let took = crate::driver::human_readable_runtime(clock.now_millis() - started);
        ev.out().message(Ec::TLC_CHECKING_TEMPORAL_PROPS_END, &[&took]);
        CheckOutcome::Done(if violated { Ec::TLC_TEMPORAL_PROPERTY_VIOLATED } else { Ec::NO_ERROR })
    }
}

/// The `RuntimeException` `getPath` throws.
fn path_error(message: String) -> EvalError {
    let _ = message;
    EvalError::unimplemented("a liveness trace whose path cannot be re-created (M4)".to_string())
}

/// `TableauLiveChecker.addNextState(tool, s, fp, tnode, oos, dgraph)`: a node `(s, tnode)`
/// that appeared after `s` was expanded. Its edges are generated here, from the tableau and
/// from the next-state relation, and so on for the nodes that appear in turn.
fn late_expand(
    ev: &mut Ev<'_>,
    g: &mut TableauDiskGraph,
    sol: &Solution,
    error_node: &mut Option<(u64, i32)>,
    s: &State,
    fp: u64,
    tidx: i32,
) -> EvalResult<()> {
    let tableau = sol.tableau.as_ref().expect("a solution with a tableau");
    let check_state_res = ev.live_state_bits(sol, s)?;
    let slen = check_state_res.len();
    let alen = sol.check_action_len();
    let mut node = g.get_node(fp, tidx);
    let num_succ = node.succ_size();
    node.set_check_state(&check_state_res);
    let tnode = tableau.node(tidx as u32);
    let stutter_bits = if tnode.successors().is_empty() { None } else { Some(ev.live_action_bits(sol, s, s)?) };
    for &t1 in tnode.successors() {
        let tnode1 = tableau.node(t1);
        let t1 = t1 as i32;
        let ptr1 = g.get_ptr(fp, t1);
        if ev.tableau_consistent(tnode1, s)? {
            if tnode1.is_accepting(t1 as u32) && error_node.is_none() && sol.has_empty_pem() {
                *error_node = Some((node.fp, node.tidx));
                return Ok(());
            }
            if ptr1 == -1 || !node.trans_exists(fp, t1) {
                node.add_transition(fp, t1, slen, alen, stutter_bits.as_deref());
                if ptr1 == -1 {
                    g.record_node(fp, t1);
                    late_expand(ev, g, sol, error_node, s, fp, t1)?;
                }
            }
        }
    }
    let count = ev.tool().actions().len();
    for i in 0..count {
        let id = ev.tool().actions()[i].id();
        let mut next_states = Vec::new();
        match ev.next_states_of(s, id, &mut Collect { states: &mut next_states }) {
            Ok(()) | Err(tlars_tool::Halt::Stop) => {}
            Err(tlars_tool::Halt::Eval(e)) => return Err(e),
        }
        for s1 in next_states.iter() {
            if !(ev.is_in_model(s1)? && ev.is_in_actions(s, s1)?) {
                continue;
            }
            let fp1 = ev.fingerprint(s1)?.raw();
            let check_action_res = ev.live_action_bits(sol, s, s1)?;
            let is_done = g.is_done(fp1);
            for &t1 in tnode.successors() {
                let tnode1 = tableau.node(t1);
                let t1 = t1 as i32;
                let ptr1 = g.get_ptr(fp1, t1);
                if ev.tableau_consistent(tnode1, s1)? && (ptr1 == -1 || !node.trans_exists(fp1, t1)) {
                    node.add_transition(fp1, t1, slen, alen, Some(&check_action_res));
                    if ptr1 == -1 {
                        g.record_node(fp1, t1);
                        if is_done {
                            late_expand(ev, g, sol, error_node, s1, fp1, t1)?;
                        }
                    }
                }
            }
        }
    }
    if num_succ < node.succ_size() {
        g.add_node(node);
    }
    Ok(())
}

/// `tool.getNextStates(action, s)`: the complete successors in generation order.
struct Collect<'s> {
    states: &'s mut Vec<State>,
}

impl tlars_tool::NextSink for Collect<'_> {
    fn add_next(&mut self, ev: &mut Ev<'_>, _s0: &State, _action: tlars_tool::ActionId, s1: &PartialState) -> tlars_tool::Flow {
        if let Some(state) = ev.complete(s1) {
            self.states.push(state);
        }
        Ok(())
    }
}

/// A step of the counterexample: the state as `getState` found it and its number.
struct Step {
    info: StateInfo,
    number: usize,
}

/// `getState(fp, sinfo)`, which numbers the state after its predecessor and records the
/// fingerprint it was looked up by.
fn successor(ev: &mut Ev<'_>, pred: &Step, fp: u64) -> EvalResult<Step> {
    let pred_fp = pred.info.fp.expect("a step looked up by fingerprint");
    match ev.successor_from_fp(&pred.info.state, pred_fp, Fp::from_raw(fp))? {
        Some(mut info) => {
            info.fp = Some(Fp::from_raw(fp));
            Ok(Step { info, number: pred.number + 1 })
        }
        None => Err(EvalError::eval(Ec::TLC_FAILED_TO_RECOVER_NEXT, ev.out().text(Ec::TLC_FAILED_TO_RECOVER_NEXT, &[]), Vec::new())),
    }
}

/// One 2217 line before it is printed: the step and the ALIAS over it.
struct Printable {
    label: String,
    fp: Option<Fp>,
    aliased: tlars_tool::Aliased,
    number: usize,
}

fn printable(ev: &mut Ev<'_>, step: &Step, next: &State, trace: &[Step]) -> EvalResult<Printable> {
    let trace: Vec<PartialState> = trace.iter().map(|s| PartialState::from_state(&s.info.state)).collect();
    let aliased = ev.eval_alias(&PartialState::from_state(&step.info.state), StateRef::Full(next), Some(&trace))?;
    Ok(Printable { label: ev.state_label(&step.info), fp: step.info.fp, aliased, number: step.number })
}

/// The state half of `printTrace`: regenerate the prefix and the cycle, evaluate the ALIAS
/// over every step, name the violated properties, and print the counterexample.
fn print_trace(ev: &mut Ev<'_>, sol: &Solution, lasso: &worker::LassoFps) -> EvalResult<()> {
    let prefix = &lasso.prefix;
    let plen = prefix.len();
    let mut fp = prefix[plen - 1];
    let Some(init) = ev.state_from_fp(Fp::from_raw(fp))? else {
        return Err(EvalError::eval(Ec::TLC_FAILED_TO_RECOVER_INIT, ev.out().text(Ec::TLC_FAILED_TO_RECOVER_INIT, &[]), Vec::new()));
    };
    let mut states = vec![Step { info: init, number: 1 }];
    for i in (0..plen - 1).rev() {
        let cur_fp = prefix[i];
        if cur_fp != fp {
            let next = successor(ev, states.last().expect("the initial state"), cur_fp)?;
            states.push(next);
            fp = cur_fp;
        }
    }
    let mut printables = Vec::with_capacity(states.len() + lasso.postfix.len());
    for i in 0..states.len() - 1 {
        let p = printable(ev, &states[i], &states[i + 1].info.state, &states[..i])?;
        printables.push(p);
    }
    let cycle_pos = states.len() - 1;
    let cycle_fp = states[cycle_pos].info.fp.expect("a step looked up by fingerprint").raw();
    let cycle_number = states[cycle_pos].number;
    let mut postfix = lasso.postfix.clone();
    let mut last = cycle_pos;
    if postfix.is_empty() {
        let cycle_state = states[cycle_pos].info.state.clone();
        printables.push(printable(ev, &states[cycle_pos], &cycle_state, &states)?);
    } else {
        if postfix.last() == Some(&cycle_fp) {
            postfix.pop();
        }
        for i in (0..postfix.len()).rev() {
            let next = successor(ev, &states[last], postfix[i])?;
            printables.push(printable(ev, &states[last], &next.info.state, &states)?);
            states.push(next);
            last = states.len() - 1;
        }
        let cycle_state = states[cycle_pos].info.state.clone();
        printables.push(printable(ev, &states[last], &cycle_state, &states)?);
    }
    let all: Vec<State> = states.iter().map(|s| s.info.state.clone()).collect();
    let violated = ev.violated_properties(&all, cycle_pos)?;
    print_counterexample(ev, &violated, &printables)?;
    let last_fp = states[last].info.fp.expect("a step looked up by fingerprint").raw();
    let back_action = if last_fp == cycle_fp {
        ev.out().state(Ec::TLC_STATE_PRINT3, &[&(cycle_number + 1).to_string()]);
        if !sol.has_empty_pem_and_box_free_promises() {
            stuttering_warning(ev)?;
        }
        states[last].info.action
    } else {
        let back = successor(ev, &states[last], cycle_fp)?;
        let label = ev.state_label(&back.info);
        ev.out().state(Ec::TLC_BACK_TO_STATE, &[&cycle_number.to_string(), &label]);
        back.info.action
    };
    // `new CounterExample(states, sinfo.getAction(), stateNumber)` over the states as
    // regenerated, not as aliased.
    let mut steps = Vec::with_capacity(states.len());
    for s in states.iter() {
        let record = ev.aliased_record(&tlars_tool::Aliased::of_state(&s.info.state))?;
        steps.push(tlars_tool::CounterExampleStep { number: s.number as i32, record, action: s.info.action });
    }
    postconditions(ev, &steps, Some((back_action, cycle_number)))
}

/// `TableauLiveChecker.printErrorTrace`: the behaviour up to the node that made the
/// violation certain, with the ALIAS of each step (no `Trace` bound), as a counterexample
/// with no cycle.
fn print_error_trace(ev: &mut Ev<'_>, prefix: &[u64]) -> EvalResult<()> {
    let plen = prefix.len();
    let mut fp = prefix[plen - 1];
    let Some(init) = ev.state_from_fp(Fp::from_raw(fp))? else {
        return Err(EvalError::eval(Ec::TLC_FAILED_TO_RECOVER_INIT, ev.out().text(Ec::TLC_FAILED_TO_RECOVER_INIT, &[]), Vec::new()));
    };
    let mut states = vec![Step { info: init, number: 1 }];
    let mut printables: Vec<Printable> = Vec::with_capacity(plen);
    for i in (0..plen - 1).rev() {
        let cur_fp = prefix[i];
        if cur_fp != fp {
            let next = successor(ev, states.last().expect("the initial state"), cur_fp)?;
            let last = states.last().expect("the initial state");
            let aliased = ev.eval_alias(&PartialState::from_state(&last.info.state), StateRef::Full(&next.info.state), None)?;
            printables.push(Printable { label: ev.state_label(&last.info), fp: last.info.fp, aliased, number: last.number });
            states.push(next);
            fp = cur_fp;
        }
    }
    let last = states.last().expect("the initial state");
    let aliased = ev.eval_alias(&PartialState::from_state(&last.info.state), StateRef::Full(&last.info.state), None)?;
    printables.push(Printable { label: ev.state_label(&last.info), fp: last.info.fp, aliased, number: last.number });
    let all: Vec<State> = states.iter().map(|s| s.info.state.clone()).collect();
    let violated = ev.violated_properties(&all, all.len() - 1)?;
    print_counterexample(ev, &violated, &printables)?;
    // `new CounterExample(states)` over the aliased states, with no loop.
    let mut steps = Vec::with_capacity(printables.len());
    for (p, s) in printables.iter().zip(states.iter()) {
        let record = ev.aliased_record(&p.aliased)?;
        steps.push(tlars_tool::CounterExampleStep { number: p.number as i32, record, action: s.info.action });
    }
    postconditions(ev, &steps, None)
}

/// 2116 with the violated properties, 2264, and one 2217 per step.
fn print_counterexample(ev: &mut Ev<'_>, violated: &[tlars_sany::Atom], printables: &[Printable]) -> EvalResult<()> {
    let names: Vec<String> = violated.iter().map(|&a| ev.tool().graph().interner().text(a).to_string()).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    ev.out().error(Ec::TLC_TEMPORAL_PROPERTY_VIOLATED, &names);
    ev.out().error(Ec::TLC_COUNTER_EXAMPLE, &[]);
    let difftrace = ev.tool().globals().difftrace;
    for i in 0..printables.len() {
        let prev = if difftrace && i > 0 { Some(&printables[i - 1].aliased) } else { None };
        let p = &printables[i];
        let shown = ev.print_aliased(&p.aliased, prev)?;
        let fp = match ev.aliased_fingerprint(&p.aliased)? {
            Some(computed) => p.fp.unwrap_or(computed).raw() as i64,
            None => -1,
        };
        ev.out().state(Ec::TLC_STATE_PRINT2, &[&p.number.to_string(), &p.label, &shown, &fp.to_string()]);
    }
    Ok(())
}

/// `checkPostConditionWithCounterExample`, whose result the liveness checker ignores. `back`
/// is the edge that closes the lasso: its action and the number of the state it returns to.
fn postconditions(
    ev: &mut Ev<'_>,
    steps: &[tlars_tool::CounterExampleStep],
    back: Option<(Option<tlars_tool::ActionId>, usize)>,
) -> EvalResult<()> {
    if ev.postcondition_exception(true).is_some() || ev.postcondition_unimplemented(true).is_some() {
        return Err(EvalError::unimplemented("a POSTCONDITION that fails to resolve after a liveness violation (M4)".to_string()));
    }
    let counter_example = tlars_tool::counter_example_lasso(ev, steps, back, true)?;
    if let Err((ec, params)) = ev.check_postconditions(Some(counter_example)) {
        let params: Vec<&str> = params.iter().map(String::as_str).collect();
        ev.out().error(ec, &params);
    }
    Ok(())
}

/// `Liveness.printStutteringCounterExampleWarning`.
fn stuttering_warning(ev: &mut Ev<'_>) -> EvalResult<()> {
    let temporal = ev.tool().temporal();
    match &temporal.spec {
        Some((name, loc)) if temporal.fairness.is_empty() => {
            let (name, loc) = (name.clone(), loc.clone());
            ev.out().warning(Ec::TLC_CONFIG_NO_FAIRNESS_BUT_LIVE_PROPERTY, &[&name, &loc])
        }
        Some(_) => Ok(()),
        None => ev.out().warning(Ec::TLC_CONFIG_NO_SPEC_BUT_PROPERTY, &[""]),
    }
    .map_err(|w| EvalError::runtime(w.ec, w.message))
}
