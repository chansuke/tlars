//! The behaviour graph of one solution: `GraphNode`, `DiskGraph` (no tableau) and
//! `TableauDiskGraph`, with their pointer tables `NodePtrTable` and `TableauNodePtrTable`.
//!
//! TLC keeps the nodes in a file and maps each node to the file pointer of its latest
//! record. Here the "file" is a vector and a pointer is an index into it, but everything
//! that decides what is printed is TLC's: a node read from the graph is a copy that is
//! written back as a new record only when it gained an edge, the pointer tables hash and
//! grow as TLC's do, and the links of the SCC search and the parents of the path search
//! are stored in those tables exactly where TLC stores them.

/// `AbstractDiskGraph.MAX_PTR`: values below it are file pointers, values from it up are
/// links of the SCC search.
pub(super) const MAX_PTR: i64 = 0x4000_0000_0000_0000;
/// `AbstractDiskGraph.MAX_LINK`: the link of a node whose component has been checked.
pub(super) const MAX_LINK: i64 = 0x7FFF_FFFF_FFFF_FFFF;

pub(super) fn is_file_pointer(loc: i64) -> bool {
    loc < MAX_PTR
}

/// `((int) k & 0x7FFFFFFF) % length`.
fn slot(k: u64, length: usize) -> usize {
    ((k as u32) & 0x7FFF_FFFF) as usize % length
}

/// `tlc2.util.BitVector`: grows on `set`, and a bit beyond its end reads as unset.
#[derive(Clone, Default)]
pub(super) struct BitVector {
    words: Vec<u64>,
}

impl BitVector {
    pub(super) fn get(&self, i: usize) -> bool {
        self.words.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }
    pub(super) fn set(&mut self, i: usize) {
        if i / 64 >= self.words.len() {
            self.words.resize(i / 64 + 1, 0);
        }
        self.words[i / 64] |= 1 << (i % 64);
    }
}

/// `GraphNode`: a state (and tableau node) with its outgoing edges, and the check bits:
/// the state checks first, then `alen` action checks per edge.
#[derive(Clone)]
pub(super) struct GraphNode {
    pub(super) fp: u64,
    pub(super) tidx: i32,
    succs: Vec<(u64, i32)>,
    checks: BitVector,
}

impl GraphNode {
    pub(super) fn new(fp: u64, tidx: i32) -> GraphNode {
        GraphNode { fp, tidx, succs: Vec::new(), checks: BitVector::default() }
    }
    pub(super) fn succ_size(&self) -> usize {
        self.succs.len()
    }
    pub(super) fn state_fp(&self, i: usize) -> u64 {
        self.succs[i].0
    }
    pub(super) fn tidx_of(&self, i: usize) -> i32 {
        self.succs[i].1
    }
    /// `addTransition`: the action bits go at the position of the edge being added. TLC's
    /// allocation hint only sizes the edge array, which shows nowhere.
    pub(super) fn add_transition(&mut self, fp: u64, tidx: i32, slen: usize, alen: usize, acts: Option<&[bool]>) {
        if let Some(acts) = acts {
            let pos = slen + alen * self.succ_size();
            for (i, &b) in acts.iter().take(alen).enumerate() {
                if b {
                    self.checks.set(pos + i);
                }
            }
        }
        self.succs.push((fp, tidx));
    }
    pub(super) fn trans_exists(&self, fp: u64, tidx: i32) -> bool {
        self.succs.iter().any(|&(f, t)| f == fp && t == tidx)
    }
    pub(super) fn set_check_state(&mut self, vals: &[bool]) {
        for (i, &b) in vals.iter().enumerate() {
            if b {
                self.checks.set(i);
            }
        }
    }
    pub(super) fn check_state(&self, i: usize) -> bool {
        self.checks.get(i)
    }
    pub(super) fn check_action(&self, slen: usize, alen: usize, node_idx: usize, i: usize) -> bool {
        self.checks.get(slen + alen * node_idx + i)
    }
    /// `getCheckAction(slen, alen, nodeIdx, int[] is)`: every one of `is` holds on the edge.
    pub(super) fn check_actions(&self, slen: usize, alen: usize, node_idx: usize, is: &[u32]) -> bool {
        is.iter().all(|&i| self.check_action(slen, alen, node_idx, i as usize))
    }
}

/// The node file and the pointer file of `AbstractDiskGraph`: every record ever written,
/// and the `(fp, tidx, ptr)` entry each write appended.
#[derive(Default)]
struct Records {
    nodes: Vec<GraphNode>,
    ptrs: Vec<(u64, i32, i64)>,
}

impl Records {
    /// `addNode`'s writes; the pointer is the record's position.
    fn append(&mut self, node: GraphNode) -> i64 {
        let ptr = self.nodes.len() as i64;
        self.ptrs.push((node.fp, node.tidx, ptr));
        self.nodes.push(node);
        ptr
    }
    /// `getNodeFromDisk`: a fresh copy of the record. The node cache TLC keeps during the
    /// SCC search returns the same contents, as nothing is written while it exists.
    fn read(&self, fp: u64, tidx: i32, ptr: i64) -> GraphNode {
        let mut node = self.nodes[ptr as usize].clone();
        node.fp = fp;
        node.tidx = tidx;
        node
    }
}

/// `NodePtrTable`: open addressing on the fingerprint, `-1` marking a free slot.
pub(super) struct NodePtrTable {
    count: usize,
    thresh: usize,
    keys: Vec<u64>,
    elems: Vec<i64>,
}

impl NodePtrTable {
    pub(super) fn new(size: usize) -> NodePtrTable {
        NodePtrTable { count: 0, thresh: (size as f64 * 0.75) as usize, keys: vec![0; size], elems: vec![-1; size] }
    }
    pub(super) fn put(&mut self, k: u64, elem: i64) {
        if self.count >= self.thresh {
            self.grow();
        }
        let length = self.keys.len();
        let mut loc = slot(k, length);
        loop {
            if self.elems[loc] == -1 {
                self.keys[loc] = k;
                self.elems[loc] = elem;
                self.count += 1;
                return;
            }
            if self.keys[loc] == k {
                self.elems[loc] = elem;
                return;
            }
            loc = (loc + 1) % length;
        }
    }
    pub(super) fn get_loc(&mut self, k: u64) -> Option<usize> {
        if self.count >= self.thresh {
            self.grow();
        }
        let length = self.keys.len();
        let mut loc = slot(k, length);
        loop {
            if self.elems[loc] == -1 {
                return None;
            }
            if self.keys[loc] == k {
                return Some(loc);
            }
            loc = (loc + 1) % length;
        }
    }
    pub(super) fn get(&mut self, k: u64) -> i64 {
        match self.get_loc(k) {
            Some(loc) => self.elems[loc],
            None => -1,
        }
    }
    fn get_by_loc(&self, loc: usize) -> i64 {
        self.elems[loc]
    }
    fn key_by_loc(&self, loc: usize) -> u64 {
        self.keys[loc]
    }
    fn put_by_loc(&mut self, k: u64, elem: i64, loc: usize) {
        self.keys[loc] = k;
        self.elems[loc] = elem;
    }
    fn grow(&mut self) {
        let new_length = (2 * self.keys.len() + 1).min(i32::MAX as usize);
        let old_keys = std::mem::replace(&mut self.keys, vec![0; new_length]);
        let old_elems = std::mem::replace(&mut self.elems, vec![-1; new_length]);
        self.count = 0;
        for (k, elem) in old_keys.into_iter().zip(old_elems) {
            if elem == -1 {
                continue;
            }
            let mut loc = slot(k, new_length);
            loop {
                if self.elems[loc] == -1 {
                    self.keys[loc] = k;
                    self.elems[loc] = elem;
                    self.count += 1;
                    break;
                }
                if self.keys[loc] == k {
                    self.elems[loc] = elem;
                    break;
                }
                loc = (loc + 1) % new_length;
            }
        }
        self.thresh = (new_length as f64 * 0.75) as usize;
    }
    pub(super) fn size(&self) -> usize {
        self.count
    }
}

/// `TableauNodePtrTable.UNDONE`: a node the search recorded but has not written.
pub(super) const UNDONE: i64 = 0xFFFF_FFFE_0000_0000_u64 as i64;
/// `TableauNodePtrTable.DONE`.
const DONE: i64 = 0xFFFF_FFFD_0000_0000_u64 as i64;
/// `nodes[3] == -2`: the high word of the first element is `UNDONE`'s.
const UNDONE_HIGH: i32 = -2;

/// One element of a `TableauNodePtrTable` entry: a tableau index and its pointer, link or
/// marker, plus the predecessor's tableau index the reverse-traversable table keeps.
#[derive(Clone, Copy)]
pub(super) struct TElem {
    pub(super) tidx: i32,
    pub(super) elem: i64,
    pred_tidx: i32,
}

/// One slot of a `TableauNodePtrTable`: a fingerprint and its elements in insertion order.
/// `parent` is what `setParent` writes over the low word of the first element; only the
/// path search of a counterexample's cycle reads it, after writing it.
#[derive(Clone)]
pub(super) struct TNode {
    pub(super) key: u64,
    pub(super) elems: Vec<TElem>,
    parent: i32,
}

impl TNode {
    fn with_key(key: u64) -> TNode {
        TNode { key, elems: Vec::new(), parent: -1 }
    }
    /// `getIdx(node, tidx)`.
    pub(super) fn idx(&self, tidx: i32) -> Option<usize> {
        self.elems.iter().position(|e| e.tidx == tidx)
    }
    /// `isSeen(nodes, tloc)`: the most significant bit of the element.
    pub(super) fn is_seen_at(&self, i: usize) -> bool {
        self.elems[i].elem < 0
    }
    pub(super) fn set_seen_at(&mut self, i: usize) {
        self.elems[i].elem |= i64::MIN;
    }
    /// `isSeen(nodes)`: the first element's bit.
    pub(super) fn is_seen(&self) -> bool {
        self.is_seen_at(0)
    }
    pub(super) fn set_seen(&mut self) {
        self.set_seen_at(0);
    }
    /// `getPtr(getElem(nodes, tloc))`: the element without the seen bit.
    pub(super) fn ptr_at(&self, i: usize) -> i64 {
        self.elems[i].elem & i64::MAX
    }
}

/// `TableauNodePtrTable`.
pub(super) struct TableauNodePtrTable {
    count: usize,
    thresh: usize,
    nodes: Vec<Option<TNode>>,
}

impl TableauNodePtrTable {
    pub(super) fn new(size: usize) -> TableauNodePtrTable {
        TableauNodePtrTable { count: 0, thresh: (size as f64 * 0.75) as usize, nodes: vec![None; size] }
    }
    pub(super) fn size(&self) -> usize {
        self.count
    }
    /// `getSize()`: the number of slots.
    pub(super) fn slots(&self) -> usize {
        self.nodes.len()
    }
    fn grow_if_full(&mut self) {
        if self.count >= self.thresh {
            self.grow();
        }
    }
    /// The slot holding `k`, after growing as every lookup of TLC's does.
    fn find(&mut self, k: u64) -> Result<usize, usize> {
        self.grow_if_full();
        let length = self.nodes.len();
        let mut loc = slot(k, length);
        loop {
            match &self.nodes[loc] {
                None => return Err(loc),
                Some(node) if node.key == k => return Ok(loc),
                Some(_) => loc = (loc + 1) % length,
            }
        }
    }
    pub(super) fn get(&mut self, k: u64, tidx: i32) -> i64 {
        let Ok(loc) = self.find(k) else { return -1 };
        let node = self.nodes[loc].as_ref().expect("found");
        match node.idx(tidx) {
            Some(i) => node.elems[i].elem,
            None => -1,
        }
    }
    /// `put(k, tidx)`: a node the search has seen but not written.
    pub(super) fn record(&mut self, k: u64, tidx: i32) {
        self.put_with(k, tidx, UNDONE, DONE);
    }
    pub(super) fn put(&mut self, k: u64, tidx: i32, elem: i64) {
        self.put_with(k, tidx, elem, elem);
    }
    /// `put(k, tidx, addElem, newElem)`: a new key or an existing element takes `addElem`,
    /// a new element of an existing key takes `newElem`.
    fn put_with(&mut self, k: u64, tidx: i32, add_elem: i64, new_elem: i64) {
        self.grow_if_full();
        let length = self.nodes.len();
        let mut loc = slot(k, length);
        loop {
            match &mut self.nodes[loc] {
                None => {
                    let mut node = TNode::with_key(k);
                    node.elems.push(TElem { tidx, elem: add_elem, pred_tidx: -1 });
                    self.nodes[loc] = Some(node);
                    self.count += 1;
                    return;
                }
                Some(node) if node.key == k => {
                    match node.idx(tidx) {
                        None => node.elems.push(TElem { tidx, elem: new_elem, pred_tidx: -1 }),
                        Some(i) => node.elems[i].elem = add_elem,
                    }
                    return;
                }
                Some(_) => loc = (loc + 1) % length,
            }
        }
    }
    /// `getLoc(k, tidx)`: `k`'s slot if it has an element for `tidx`.
    pub(super) fn get_loc(&mut self, k: u64, tidx: i32) -> Option<usize> {
        let loc = self.find(k).ok()?;
        self.nodes[loc].as_ref().expect("found").idx(tidx).map(|_| loc)
    }
    /// `getNodesLoc(k)`.
    pub(super) fn nodes_loc(&mut self, k: u64) -> Option<usize> {
        self.find(k).ok()
    }
    pub(super) fn at(&self, loc: usize) -> Option<&TNode> {
        self.nodes[loc].as_ref()
    }
    pub(super) fn at_mut(&mut self, loc: usize) -> &mut TNode {
        self.nodes[loc].as_mut().expect("an occupied slot")
    }
    /// `getNodes(k)`, as the slot to read it from.
    pub(super) fn nodes_of(&mut self, k: u64) -> Option<usize> {
        self.find(k).ok()
    }
    pub(super) fn is_done(&mut self, k: u64) -> bool {
        let Some(loc) = self.nodes_of(k) else { return false };
        let node = self.nodes[loc].as_ref().expect("found");
        match node.elems.first() {
            None => true,
            Some(first) => (first.elem >> 32) as i32 != UNDONE_HIGH,
        }
    }
    /// `setDone(k)`: the slot of `k`, added as a bare key when it is new.
    pub(super) fn set_done(&mut self, k: u64) -> usize {
        match self.find(k) {
            Err(loc) => {
                self.nodes[loc] = Some(TNode::with_key(k));
                self.count += 1;
                loc
            }
            Ok(loc) => {
                let node = self.nodes[loc].as_mut().expect("found");
                if let Some(first) = node.elems.first_mut()
                    && (first.elem >> 32) as i32 == UNDONE_HIGH
                {
                    first.elem = (first.elem & 0xFFFF_FFFF) | (DONE & !0xFFFF_FFFF);
                }
                loc
            }
        }
    }
    /// `resetElems()`: every element's seen bit cleared.
    pub(super) fn reset_elems(&mut self) {
        for node in self.nodes.iter_mut().flatten() {
            for e in node.elems.iter_mut() {
                e.elem &= i64::MAX;
            }
        }
    }
    pub(super) fn set_parent(&mut self, loc: usize, parent: i32) {
        self.at_mut(loc).parent = parent;
    }
    pub(super) fn parent(&self, loc: usize) -> i32 {
        self.nodes[loc].as_ref().expect("an occupied slot").parent
    }
    fn grow(&mut self) {
        let length = (2 * self.nodes.len() + 1).min(i32::MAX as usize);
        self.thresh = (length as f64 * 0.75) as usize;
        let old = std::mem::replace(&mut self.nodes, vec![None; length]);
        for node in old.into_iter().flatten() {
            let mut loc = slot(node.key, length);
            while self.nodes[loc].is_some() {
                loc = (loc + 1) % length;
            }
            self.nodes[loc] = Some(node);
        }
    }
}

/// `DiskGraph`: the behaviour graph of a solution without a tableau.
pub(super) struct DiskGraph {
    records: Records,
    init_nodes: Vec<(u64, i32)>,
    tbl: NodePtrTable,
}

/// `TableauDiskGraph`.
pub(super) struct TableauDiskGraph {
    records: Records,
    init_nodes: Vec<(u64, i32)>,
    tbl: TableauNodePtrTable,
}

/// `AbstractDiskGraph`, as `LiveWorker` sees either kind.
pub(super) enum Graph {
    Plain(DiskGraph),
    Tableau(TableauDiskGraph),
}

/// `AbstractDiskGraph.getPath`'s failure.
pub(super) fn path_failure(fp: u64, tidx: i32) -> String {
    format!("Couldn't re-create liveness trace (path) starting at: {} and tidx: {tidx}", fp as i64)
}

impl DiskGraph {
    pub(super) fn new() -> DiskGraph {
        DiskGraph { records: Records::default(), init_nodes: Vec::new(), tbl: NodePtrTable::new(255) }
    }
    pub(super) fn add_init_node(&mut self, fp: u64, tidx: i32) {
        self.init_nodes.push((fp, tidx));
    }
    /// `getNode(fp)`: a new node when there is no record yet.
    pub(super) fn get_node(&mut self, fp: u64) -> GraphNode {
        let ptr = self.tbl.get(fp);
        if ptr < 0 {
            return GraphNode::new(fp, -1);
        }
        self.records.read(fp, -1, ptr)
    }
    pub(super) fn get_ptr(&mut self, fp: u64) -> i64 {
        self.tbl.get(fp)
    }
    pub(super) fn add_node(&mut self, node: GraphNode) {
        let fp = node.fp;
        let ptr = self.records.append(node);
        self.tbl.put(fp, ptr);
    }
    /// `makeNodePtrTbl`: every pointer entry put back, which turns links into pointers.
    fn make_node_ptr_tbl(&mut self) {
        for i in 0..self.records.ptrs.len() {
            let (fp, _, ptr) = self.records.ptrs[i];
            self.tbl.put(fp, ptr);
        }
    }
    /// `getPath(state, tidx)`: a breadth-first search from the initial nodes, the state
    /// first and an initial state last.
    fn get_path(&mut self, state: u64) -> Result<Vec<u64>, String> {
        if self.init_nodes.iter().any(|&(s, _)| s == state) {
            return Ok(vec![state]);
        }
        self.make_node_ptr_tbl();
        let offset = MAX_PTR + 1;
        let mut queue = std::collections::VecDeque::new();
        for i in 0..self.init_nodes.len() {
            let state0 = self.init_nodes[i].0;
            let ptr = self.tbl.get(state0);
            if ptr != -1 {
                queue.push_back((state0, ptr));
                self.tbl.put(state0, MAX_PTR);
            }
        }
        while let Some((mut cur_state, cur_ptr)) = queue.pop_front() {
            let cur_node = self.records.read(cur_state, -1, cur_ptr);
            for i in 0..cur_node.succ_size() {
                let next_state = cur_node.state_fp(i);
                if next_state == state {
                    let mut res = vec![next_state];
                    let mut cur_loc = self.tbl.get_loc(cur_state).expect("a visited state");
                    loop {
                        res.push(cur_state);
                        let ploc = self.tbl.get_by_loc(cur_loc);
                        if ploc == MAX_PTR {
                            break;
                        }
                        cur_loc = (ploc - offset) as usize;
                        cur_state = self.tbl.key_by_loc(cur_loc);
                    }
                    return Ok(res);
                }
                let Some(next_loc) = self.tbl.get_loc(next_state) else { continue };
                let next_ptr = self.tbl.get_by_loc(next_loc);
                if is_file_pointer(next_ptr) {
                    queue.push_back((next_state, next_ptr));
                    let cur_loc = self.tbl.get_loc(cur_state).expect("a visited state");
                    self.tbl.put_by_loc(next_state, offset + cur_loc as i64, next_loc);
                }
            }
        }
        Err(path_failure(state, -1))
    }
}

impl TableauDiskGraph {
    pub(super) fn new() -> TableauDiskGraph {
        TableauDiskGraph { records: Records::default(), init_nodes: Vec::new(), tbl: TableauNodePtrTable::new(255) }
    }
    pub(super) fn add_init_node(&mut self, fp: u64, tidx: i32) {
        self.init_nodes.push((fp, tidx));
    }
    pub(super) fn get_ptr(&mut self, fp: u64, tidx: i32) -> i64 {
        self.tbl.get(fp, tidx)
    }
    pub(super) fn is_done(&mut self, fp: u64) -> bool {
        self.tbl.is_done(fp)
    }
    /// `setDone(fp)` followed by `getNodesByLoc`: the tableau indices of the state's nodes
    /// as they stand now. TLC iterates the array it fetched, so elements appended while it
    /// iterates are not visited.
    pub(super) fn set_done(&mut self, fp: u64) -> Vec<i32> {
        let loc = self.tbl.set_done(fp);
        self.tbl.at(loc).map(|n| n.elems.iter().map(|e| e.tidx).collect()).unwrap_or_default()
    }
    pub(super) fn record_node(&mut self, fp: u64, tidx: i32) {
        self.tbl.record(fp, tidx);
    }
    pub(super) fn get_node(&mut self, fp: u64, tidx: i32) -> GraphNode {
        let ptr = self.tbl.get(fp, tidx);
        if ptr < 0 {
            return GraphNode::new(fp, tidx);
        }
        self.records.read(fp, tidx, ptr)
    }
    pub(super) fn add_node(&mut self, node: GraphNode) {
        let (fp, tidx) = (node.fp, node.tidx);
        let ptr = self.records.append(node);
        self.tbl.put(fp, tidx, ptr);
    }
    fn make_node_ptr_tbl_into(&self, table: &mut TableauNodePtrTable) {
        for &(fp, tidx, ptr) in self.records.ptrs.iter() {
            table.put(fp, tidx, ptr);
        }
    }
    /// `getPath(state, tidx)` over a fresh reverse-traversable table, which records each
    /// node's predecessor and the predecessor's tableau index.
    pub(super) fn get_path(&mut self, state: u64, tidx: i32) -> Result<Vec<u64>, String> {
        const INIT_STATE: i64 = MAX_PTR + 1;
        if self.init_nodes.iter().any(|&(s, t)| s == state && t == tidx) {
            return Ok(vec![state]);
        }
        let mut table = TableauNodePtrTable::new(255);
        self.make_node_ptr_tbl_into(&mut table);
        let mut queue = std::collections::VecDeque::new();
        for i in 0..self.init_nodes.len() {
            let (state0, tidx0) = self.init_nodes[i];
            let ptr = table.get(state0, tidx0);
            if ptr != -1 {
                queue.push_back((state0, tidx0, ptr));
                table.put(state0, tidx0, MAX_PTR);
            }
        }
        while let Some((cur_state, cur_tidx, cur_ptr)) = queue.pop_front() {
            let cur_node = self.records.read(cur_state, cur_tidx, cur_ptr);
            for i in 0..cur_node.succ_size() {
                let next_state = cur_node.state_fp(i);
                let next_tidx = cur_node.tidx_of(i);
                if next_state == cur_state && next_tidx == cur_tidx {
                    continue;
                }
                if next_state == state && next_tidx == tidx {
                    return reconstruct_reverse_path(&mut table, cur_state, cur_tidx, next_state);
                }
                let Some(next_loc) = table.nodes_loc(next_state) else { continue };
                let Some(cloc) = table.at(next_loc).expect("found").idx(next_tidx) else { continue };
                let next_ptr = table.at(next_loc).expect("found").elems[cloc].elem;
                if is_file_pointer(next_ptr) {
                    queue.push_back((next_state, next_tidx, next_ptr));
                    let cur_loc = table.nodes_loc(cur_state).expect("a visited state");
                    let e = &mut table.at_mut(next_loc).elems[cloc];
                    e.elem = INIT_STATE + cur_loc as i64;
                    e.pred_tidx = cur_tidx;
                }
            }
        }
        Err(path_failure(state, tidx))
    }
}

/// `reconstructReversePath`: from the node before the target back to an initial node. Of a
/// node's elements for the current tableau index, the *last* that holds a predecessor is
/// taken, unless one is an initial node.
fn reconstruct_reverse_path(
    table: &mut TableauNodePtrTable,
    start_state: u64,
    start_tidx: i32,
    final_state: u64,
) -> Result<Vec<u64>, String> {
    const INIT_STATE: i64 = MAX_PTR + 1;
    let mut res = vec![final_state];
    let mut current_state = start_state;
    let mut current_tidx = start_tidx;
    let mut loc = table.nodes_loc(current_state).expect("a visited state");
    loop {
        res.push(current_state);
        let mut predecessor_location = -1;
        let mut predecessor_tidx = -1;
        for e in table.at(loc).expect("a visited state").elems.iter() {
            if current_tidx == e.tidx && !is_file_pointer(e.elem) {
                predecessor_location = e.elem;
                predecessor_tidx = e.pred_tidx;
                if e.elem == MAX_PTR {
                    break;
                }
            }
        }
        if predecessor_location == MAX_PTR {
            break;
        }
        if predecessor_location < INIT_STATE {
            // TLC indexes the table with a negative slot here and fails.
            return Err(path_failure(current_state, current_tidx));
        }
        loc = (predecessor_location - INIT_STATE) as usize;
        let previous = (current_state, current_tidx);
        current_state = table.at(loc).expect("a visited state").key;
        current_tidx = predecessor_tidx;
        if (current_state, current_tidx) == previous {
            return Err("Self loop in trace path reconstruction".to_string());
        }
    }
    Ok(res)
}

impl Graph {
    pub(super) fn init_nodes(&self) -> &[(u64, i32)] {
        match self {
            Graph::Plain(g) => &g.init_nodes,
            Graph::Tableau(g) => &g.init_nodes,
        }
    }
    /// `size()`: the number of states (keys) in the pointer table.
    pub(super) fn size(&self) -> usize {
        match self {
            Graph::Plain(g) => g.tbl.size(),
            Graph::Tableau(g) => g.tbl.size(),
        }
    }
    pub(super) fn make_node_ptr_tbl(&mut self) {
        match self {
            Graph::Plain(g) => g.make_node_ptr_tbl(),
            Graph::Tableau(g) => {
                let mut tbl = std::mem::replace(&mut g.tbl, TableauNodePtrTable::new(0));
                g.make_node_ptr_tbl_into(&mut tbl);
                g.tbl = tbl;
            }
        }
    }
    /// `getNode(fp, tidx, ptr)`.
    pub(super) fn node_at(&self, fp: u64, tidx: i32, ptr: i64) -> GraphNode {
        match self {
            Graph::Plain(g) => g.records.read(fp, tidx, ptr),
            Graph::Tableau(g) => g.records.read(fp, tidx, ptr),
        }
    }
    pub(super) fn get_link(&mut self, fp: u64, tidx: i32) -> i64 {
        match self {
            Graph::Plain(g) => g.tbl.get(fp),
            Graph::Tableau(g) => g.tbl.get(fp, tidx),
        }
    }
    /// `putLink`: the link is stored unless the node already has one, which is returned.
    pub(super) fn put_link(&mut self, fp: u64, tidx: i32, link: i64) -> i64 {
        match self {
            Graph::Plain(g) => {
                let loc = g.tbl.get_loc(fp).expect("a node of the graph");
                let old = g.tbl.get_by_loc(loc);
                if !is_file_pointer(old) {
                    return old;
                }
                g.tbl.put_by_loc(fp, link, loc);
                -1
            }
            Graph::Tableau(g) => {
                let loc = g.tbl.nodes_of(fp).expect("a node of the graph");
                let node = g.tbl.at_mut(loc);
                let i = node.idx(tidx).expect("a node of the graph");
                let old = node.elems[i].elem;
                if !is_file_pointer(old) {
                    return old;
                }
                node.elems[i].elem = link;
                -1
            }
        }
    }
    pub(super) fn set_max_link(&mut self, fp: u64, tidx: i32) {
        match self {
            Graph::Plain(g) => g.tbl.put(fp, MAX_LINK),
            Graph::Tableau(g) => g.tbl.put(fp, tidx, MAX_LINK),
        }
    }
    pub(super) fn get_path(&mut self, fp: u64, tidx: i32) -> Result<Vec<u64>, String> {
        match self {
            Graph::Plain(g) => g.get_path(fp),
            Graph::Tableau(g) => g.get_path(fp, tidx),
        }
    }
}
