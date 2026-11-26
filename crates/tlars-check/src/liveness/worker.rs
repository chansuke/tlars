//! `LiveWorker`: the strongly connected components of one solution's behaviour graph, per
//! possible error model, and the lasso of a component that violates it. Everything here
//! works on fingerprints and check bits; turning the lasso into states is the caller's.

use super::graph::{Graph, GraphNode, MAX_PTR, TableauNodePtrTable, is_file_pointer};
use std::collections::VecDeque;
use tlars_tool::{Pem, Solution};

/// `LiveWorker.SCC_MARKER`.
const SCC_MARKER: i64 = -42;

/// The fingerprints of a counterexample, as `printTrace` holds them before it regenerates
/// any state.
pub(super) struct LassoFps {
    /// `getPath(state, tidx)`: the state that closes the cycle first, an initial state last.
    pub(super) prefix: Vec<u64>,
    /// The cycle after that state, from `bfsPostFix` and the `dfsPostFix` stack, last
    /// state first.
    pub(super) postfix: Vec<u64>,
}

/// `getPath`'s `RuntimeException`, which ends the check.
pub(super) type PathFailure = String;

/// `IntStack` as `checkSccs` uses it: longs and ints on one stack, its size counted in
/// ints, which is what the loop condition reads.
#[derive(Default)]
struct DfsStack {
    items: Vec<i64>,
    ints: usize,
}

impl DfsStack {
    fn push_long(&mut self, v: i64) {
        self.items.push(v);
        self.ints += 2;
    }
    fn push_int(&mut self, v: i32) {
        self.items.push(v as i64);
        self.ints += 1;
    }
    fn pop_long(&mut self) -> i64 {
        self.ints -= 2;
        self.items.pop().expect("a long on the DFS stack")
    }
    fn pop_int(&mut self) -> i32 {
        self.ints -= 1;
        self.items.pop().expect("an int on the DFS stack") as i32
    }
    fn reset(&mut self) {
        self.items.clear();
        self.ints = 0;
    }
}

/// `checkSccs` for one PEM: Tarjan's algorithm, iterative, with a node's links kept in the
/// pointer table. An edge that does not satisfy every `<>[]A` check is not followed; its
/// target starts a search of its own later. `Ok(Some)` is the first violating component's
/// lasso.
pub(super) fn check_sccs(graph: &mut Graph, sol: &Solution, pem: &Pem) -> Result<Option<LassoFps>, PathFailure> {
    graph.make_node_ptr_tbl();
    let mut node_queue: VecDeque<(u64, i32, i64)> = VecDeque::new();
    for &(state, tidx) in graph.init_nodes().to_vec().iter() {
        let ptr = graph.get_link(state, tidx);
        if ptr >= 0 {
            node_queue.push_back((state, tidx, ptr));
        }
    }
    let ea = &pem.ea_action;
    let (slen, alen) = (sol.check_state_len(), sol.check_action_len());
    let mut dfs = DfsStack::default();
    let mut com: Vec<(i64, i32, u64)> = Vec::new();
    while let Some((state, tidx, loc)) = node_queue.pop_front() {
        dfs.reset();
        dfs.push_long(state as i64);
        dfs.push_int(tidx);
        dfs.push_long(loc);
        dfs.push_long(MAX_PTR);
        let mut new_link = MAX_PTR;
        while dfs.ints >= 7 {
            let low_link = dfs.pop_long();
            let cur_loc = dfs.pop_long();
            let cur_tidx = dfs.pop_int();
            let cur_state = dfs.pop_long() as u64;
            if cur_loc == SCC_MARKER {
                let cur_link = graph.get_link(cur_state, cur_tidx);
                if cur_link == low_link
                    && let Some(lasso) = check_component(graph, sol, pem, cur_state, cur_tidx, &mut com)?
                {
                    return Ok(Some(lasso));
                }
                let plow_link = dfs.pop_long();
                dfs.push_long(plow_link.min(low_link));
            } else {
                let link = graph.put_link(cur_state, cur_tidx, new_link);
                if link == -1 {
                    dfs.push_long(low_link);
                    dfs.push_long(cur_state as i64);
                    dfs.push_int(cur_tidx);
                    dfs.push_long(SCC_MARKER);
                    com.push((cur_loc, cur_tidx, cur_state));
                    let gnode = graph.node_at(cur_state, cur_tidx, cur_loc);
                    let mut next_low_link = new_link;
                    new_link += 1;
                    for i in 0..gnode.succ_size() {
                        let next_state = gnode.state_fp(i);
                        let next_tidx = gnode.tidx_of(i);
                        let next_link = graph.get_link(next_state, next_tidx);
                        if next_link < 0 {
                            continue;
                        }
                        if gnode.check_actions(slen, alen, i, ea) {
                            if is_file_pointer(next_link) {
                                dfs.push_long(next_state as i64);
                                dfs.push_int(next_tidx);
                                dfs.push_long(next_link);
                            } else {
                                next_low_link = next_low_link.min(next_link);
                            }
                        } else if is_file_pointer(next_link) {
                            node_queue.push_back((next_state, next_tidx, next_link));
                        }
                    }
                    dfs.push_long(next_low_link);
                } else {
                    dfs.push_long(low_link.min(link));
                }
            }
        }
    }
    Ok(None)
}

/// `checkComponent`: the component whose root is `(state, tidx)`, popped off `com`. A
/// single node counts only if it stutters under the `<>[]A` checks. The component violates
/// the PEM if it meets every `[]<>S`, every `[]<>A` on an edge inside it that satisfies the
/// `<>[]A` checks, and fulfils every promise.
fn check_component(
    graph: &mut Graph,
    sol: &Solution,
    pem: &Pem,
    state: u64,
    tidx: i32,
    com_stack: &mut Vec<(i64, i32, u64)>,
) -> Result<Option<LassoFps>, PathFailure> {
    let (mut loc1, mut tidx1, mut state1) = com_stack.pop().expect("the component's root is on the stack");
    if state1 == state && tidx1 == tidx && !is_stuttering(graph, sol, pem, state1, tidx1, loc1) {
        graph.set_max_link(state, tidx);
        return Ok(None);
    }
    let mut com = TableauNodePtrTable::new(128);
    loop {
        com.put(state1, tidx1, loc1);
        graph.set_max_link(state1, tidx1);
        if state == state1 && tidx == tidx1 {
            break;
        }
        (loc1, tidx1, state1) = com_stack.pop().expect("the component's root is on the stack");
    }
    let (slen, alen) = (sol.check_state_len(), sol.check_action_len());
    let mut ae_state_res = vec![false; pem.ae_state.len()];
    let mut ae_action_res = vec![false; pem.ae_action.len()];
    let mut promise_res = vec![false; sol.promise_count()];
    let ea = &pem.ea_action;
    for ci in 0..com.slots() {
        let Some(node) = com.at(ci) else { continue };
        let node = node.clone();
        for e in node.elems.iter() {
            let cur = graph.node_at(node.key, e.tidx, e.elem);
            for (i, &idx) in pem.ae_state.iter().enumerate() {
                if !ae_state_res[i] {
                    ae_state_res[i] = cur.check_state(idx as usize);
                }
            }
            let succ_cnt = if pem.ae_action.is_empty() { 0 } else { cur.succ_size() };
            for i in 0..succ_cnt {
                if com.get_loc(cur.state_fp(i), cur.tidx_of(i)).is_none() {
                    continue;
                }
                if !cur.check_actions(slen, alen, i, ea) {
                    continue;
                }
                for (j, &idx) in pem.ae_action.iter().enumerate() {
                    if !ae_action_res[j] {
                        ae_action_res[j] = cur.check_action(slen, alen, i, idx as usize);
                    }
                }
            }
            for (i, res) in promise_res.iter_mut().enumerate() {
                if sol.is_fulfilling(cur.tidx as u32, i) {
                    *res = true;
                }
            }
        }
    }
    if ae_state_res.iter().chain(ae_action_res.iter()).chain(promise_res.iter()).any(|&b| !b) {
        return Ok(None);
    }
    print_trace_fps(graph, sol, pem, state, tidx, &mut com).map(Some)
}

/// `isStuttering`: the node's edge to itself satisfies every `<>[]A` check.
fn is_stuttering(graph: &Graph, sol: &Solution, pem: &Pem, state: u64, tidx: i32, loc: i64) -> bool {
    let gnode = graph.node_at(state, tidx, loc);
    for i in 0..gnode.succ_size() {
        if gnode.state_fp(i) == state && gnode.tidx_of(i) == tidx {
            return gnode.check_actions(sol.check_state_len(), sol.check_action_len(), i, &pem.ea_action);
        }
    }
    false
}

/// The graph half of `printTrace`: the prefix (`getPath`), then the cycle from
/// `dfsPostFix`'s stack and `bfsPostFix`.
fn print_trace_fps(
    graph: &mut Graph,
    sol: &Solution,
    pem: &Pem,
    state: u64,
    tidx: i32,
    com: &mut TableauNodePtrTable,
) -> Result<LassoFps, PathFailure> {
    let prefix = graph.get_path(state, tidx)?;
    let mut cycle: Vec<(i32, u64)> = Vec::new();
    let cur = dfs_post_fix(graph, sol, pem, state, tidx, com, &mut cycle);
    let mut postfix = bfs_post_fix(graph, sol, pem, state, tidx, com, &cur);
    while let Some((_, fp)) = cycle.pop() {
        if postfix.last() != Some(&fp) {
            postfix.push(fp);
        }
    }
    Ok(LassoFps { prefix, postfix })
}

/// The slot of `fp` in the component table and the element index of `tidx` in it.
fn elem_of(com: &mut TableauNodePtrTable, fp: u64, tidx: i32) -> Option<(usize, usize)> {
    let loc = com.nodes_of(fp)?;
    let i = com.at(loc).expect("found").idx(tidx)?;
    Some((loc, i))
}

/// `dfsPostFix`: a walk through the component from `(state, tidx)` that stops once it has
/// met every `[]<>S`, `[]<>A` and promise, greedily taking an edge that meets one more.
/// The nodes it left are on `cycle`; the node it ends on is returned.
fn dfs_post_fix(
    graph: &Graph,
    sol: &Solution,
    pem: &Pem,
    state: u64,
    tidx: i32,
    com: &mut TableauNodePtrTable,
    cycle: &mut Vec<(i32, u64)>,
) -> GraphNode {
    let (slen, alen) = (sol.check_state_len(), sol.check_action_len());
    let mut ae_state_res = vec![false; pem.ae_state.len()];
    let mut ae_action_res = vec![false; pem.ae_action.len()];
    let mut promise_res = vec![false; sol.promise_count()];
    let ea = &pem.ea_action;
    let mut cnt = ae_state_res.len() + ae_action_res.len() + promise_res.len();
    let (loc, tloc) = elem_of(com, state, tidx).expect("the root is in its component");
    let ptr = com.at(loc).expect("found").elems[tloc].elem;
    com.at_mut(loc).set_seen_at(tloc);
    let mut cur = graph.node_at(state, tidx, ptr);
    while cnt > 0 {
        let cnt0 = cnt;
        'next: loop {
            for (i, &idx) in pem.ae_state.iter().enumerate() {
                if !ae_state_res[i] && cur.check_state(idx as usize) {
                    ae_state_res[i] = true;
                    cnt -= 1;
                }
            }
            for (i, res) in promise_res.iter_mut().enumerate() {
                if !*res && sol.is_fulfilling(cur.tidx as u32, i) {
                    *res = true;
                    cnt -= 1;
                }
            }
            if cnt == 0 {
                break 'next;
            }
            // `nextState1` and friends: the last successor in the component that satisfies
            // the `<>[]A` checks; `nextState2`: the last such one not yet visited.
            let mut next1: Option<(u64, i32, usize, usize)> = None;
            let mut next2: Option<(u64, i32, usize, usize)> = None;
            let cnt1 = cnt;
            for i in 0..cur.succ_size() {
                let next_state = cur.state_fp(i);
                let next_tidx = cur.tidx_of(i);
                let Some(nloc) = com.nodes_of(next_state) else { continue };
                let Some(t) = com.at(nloc).expect("found").idx(next_tidx) else { continue };
                if !cur.check_actions(slen, alen, i, ea) {
                    continue;
                }
                next1 = Some((next_state, next_tidx, nloc, t));
                for (j, &idx) in pem.ae_action.iter().enumerate() {
                    if !ae_action_res[j] && cur.check_action(slen, alen, i, idx as usize) {
                        ae_action_res[j] = true;
                        cnt -= 1;
                    }
                }
                if cnt < cnt1 {
                    cycle.push((cur.tidx, cur.fp));
                    let next_ptr = com.at(nloc).expect("found").ptr_at(t);
                    cur = graph.node_at(next_state, next_tidx, next_ptr);
                    com.reset_elems();
                    break 'next;
                }
                if !com.at(nloc).expect("found").is_seen_at(t) {
                    next2 = Some((next_state, next_tidx, nloc, t));
                }
            }
            if cnt < cnt0 {
                let (next_state, next_tidx, nloc, t) = next1.expect("a successor inside the component");
                cycle.push((cur.tidx, cur.fp));
                let next_ptr = com.at(nloc).expect("found").ptr_at(t);
                cur = graph.node_at(next_state, next_tidx, next_ptr);
                com.reset_elems();
                break 'next;
            }
            // Back up along the walk until a node has an unvisited successor in the
            // component; this search does not look at the `<>[]A` checks.
            while next2.is_none() {
                let (cur_tidx, cur_state) = cycle.pop().expect("a node to back up to");
                let cur_ptr = com.get(cur_state, cur_tidx) & i64::MAX;
                cur = graph.node_at(cur_state, cur_tidx, cur_ptr);
                for i in 0..cur.succ_size() {
                    let (ns, nt) = (cur.state_fp(i), cur.tidx_of(i));
                    if let Some((nloc, t)) = elem_of(com, ns, nt)
                        && !com.at(nloc).expect("found").is_seen_at(t)
                    {
                        next2 = Some((ns, nt, nloc, t));
                        break;
                    }
                }
            }
            let (next_state, next_tidx, nloc, t) = next2.expect("set above");
            cycle.push((cur.tidx, cur.fp));
            let next_ptr = com.at(nloc).expect("found").ptr_at(t);
            cur = graph.node_at(next_state, next_tidx, next_ptr);
            com.at_mut(nloc).set_seen_at(t);
        }
    }
    com.reset_elems();
    cur
}

/// `bfsPostFix`: the shortest way back from where `dfsPostFix` ended to `(state, tidx)`
/// over edges that satisfy the `<>[]A` checks, as states, the last first. Empty when the
/// walk ended where it began.
fn bfs_post_fix(
    graph: &Graph,
    sol: &Solution,
    pem: &Pem,
    state: u64,
    tidx: i32,
    com: &mut TableauNodePtrTable,
    cur: &GraphNode,
) -> Vec<u64> {
    let (slen, alen) = (sol.check_state_len(), sol.check_action_len());
    let ea = &pem.ea_action;
    let mut postfix = Vec::new();
    let (start_state, start_tidx) = (cur.fp, cur.tidx);
    if start_state == state && start_tidx == tidx {
        return postfix;
    }
    let mut queue: VecDeque<(u64, i32)> = VecDeque::new();
    let mut cur_state = start_state;
    let mut ploc: i32 = -1;
    let mut cur_loc = com.nodes_of(cur_state).expect("the walk ended inside the component");
    com.at_mut(cur_loc).set_seen();
    'done: loop {
        let elems = com.at(cur_loc).expect("found").elems.clone();
        for (tloc, e) in elems.iter().enumerate() {
            let cur_tidx = e.tidx;
            let cur_ptr = com.at(cur_loc).expect("found").ptr_at(tloc);
            let node = graph.node_at(cur_state, cur_tidx, cur_ptr);
            for j in 0..node.succ_size() {
                let (next_state, next_tidx) = (node.state_fp(j), node.tidx_of(j));
                if cur_state == next_state && cur_tidx == next_tidx {
                    continue;
                }
                if !node.check_actions(slen, alen, j, ea) {
                    continue;
                }
                if next_state == state && next_tidx == tidx {
                    while cur_state != start_state {
                        postfix.push(cur_state);
                        let at = ploc as usize;
                        cur_state = com.at(at).expect("a visited state").key;
                        ploc = com.parent(at);
                    }
                    postfix.push(start_state);
                    break 'done;
                }
                if let Some(l1) = com.nodes_of(next_state)
                    && !com.at(l1).expect("found").is_seen()
                {
                    com.at_mut(l1).set_seen();
                    queue.push_back((next_state, cur_loc as i32));
                }
            }
        }
        com.set_parent(cur_loc, ploc);
        let Some((next, parent)) = queue.pop_front() else { break };
        cur_state = next;
        ploc = parent;
        cur_loc = com.nodes_of(cur_state).expect("an enqueued state");
    }
    postfix
}
