//! `java.util.HashMap` (JDK 21) with Java's iteration order, for `HashMap`/`HashSet` in
//! translated SANY code. Keys bring their own `hashCode` and `equals` (passed in), because
//! SANY's keys hash by translated methods (`SemanticNode.hashCode` is `kind` and a
//! sequential uid, so TLC's order is reproducible).
//!
//! Modelled: lazy table allocation with the initial capacity parked in `threshold`,
//! `hash = h ^ (h >>> 16)`, append within a bin, order-preserving lo/hi split on resize,
//! `treeifyBin` resizing tables below 64, `putMapEntries` pre-sizing. A bin that would become
//! a tree (9 entries in a table of at least 64) panics: tree bins iterate in a different
//! order and are not modelled.

const DEFAULT_INITIAL_CAPACITY: usize = 16;
const MAXIMUM_CAPACITY: usize = 1 << 30;
const LOAD_FACTOR: f32 = 0.75;
const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;

#[derive(Clone)]
pub struct JavaHashMap<K, V> {
    /// `None` until the first insertion.
    table: Option<Vec<Vec<(u32, K, V)>>>,
    threshold: usize,
    len: usize,
}

/// `HashMap.hash`: the key's `hashCode` with its high half folded in.
fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

/// `HashMap.tableSizeFor`.
pub fn table_size_for(cap: usize) -> usize {
    if cap <= 1 { 1 } else { cap.next_power_of_two().min(MAXIMUM_CAPACITY) }
}

/// `HashMap.calculateHashMapCapacity`: `ceil(n / 0.75)`.
pub fn capacity_for_mappings(n: usize) -> usize {
    (n as f64 / f64::from(LOAD_FACTOR)).ceil() as usize
}

impl<K, V> Default for JavaHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> JavaHashMap<K, V> {
    /// `new HashMap<>()`.
    pub fn new() -> Self {
        JavaHashMap { table: None, threshold: 0, len: 0 }
    }

    /// `new HashMap<>(initialCapacity)`.
    pub fn with_capacity(initial_capacity: usize) -> Self {
        JavaHashMap { table: None, threshold: table_size_for(initial_capacity), len: 0 }
    }

    /// `HashMap.newHashMap(n)`, as `new HashSet<>(collection)` sizes its map.
    pub fn for_mappings(n: usize) -> Self {
        Self::with_capacity(capacity_for_mappings(n))
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn resize(&mut self) {
        let old_cap = self.table.as_ref().map_or(0, Vec::len);
        let old_thr = self.threshold;
        let mut new_thr = 0;
        let new_cap;
        if old_cap > 0 {
            if old_cap >= MAXIMUM_CAPACITY {
                self.threshold = i32::MAX as usize;
                return;
            }
            new_cap = old_cap << 1;
            if new_cap < MAXIMUM_CAPACITY && old_cap >= DEFAULT_INITIAL_CAPACITY {
                new_thr = old_thr << 1;
            }
        } else if old_thr > 0 {
            new_cap = old_thr;
        } else {
            new_cap = DEFAULT_INITIAL_CAPACITY;
            new_thr = (LOAD_FACTOR * DEFAULT_INITIAL_CAPACITY as f32) as usize;
        }
        if new_thr == 0 {
            let ft = new_cap as f32 * LOAD_FACTOR;
            new_thr = if new_cap < MAXIMUM_CAPACITY && ft < MAXIMUM_CAPACITY as f32 { ft as usize } else { i32::MAX as usize };
        }
        self.threshold = new_thr;
        let mut new_tab: Vec<Vec<(u32, K, V)>> = (0..new_cap).map(|_| Vec::new()).collect();
        if let Some(old) = self.table.take() {
            for (j, bin) in old.into_iter().enumerate() {
                for e in bin {
                    // The split keeps each half in its original order.
                    let to = if e.0 as usize & old_cap == 0 { j } else { j + old_cap };
                    new_tab[to].push(e);
                }
            }
        }
        self.table = Some(new_tab);
    }

    fn index(&self, h: u32) -> usize {
        let n = self.table.as_ref().map_or(0, Vec::len);
        (n - 1) & h as usize
    }

    /// `put(key, value)`; returns the previous value. `hash` is the key's `hashCode`.
    pub fn put(&mut self, key: K, hash: i32, value: V, eq: impl Fn(&K, &K) -> bool) -> Option<V> {
        let h = spread(hash);
        if self.table.is_none() {
            self.resize();
        }
        let i = self.index(h);
        let bin = &mut self.table.as_mut().expect("table")[i];
        if let Some(e) = bin.iter_mut().find(|e| e.0 == h && eq(&e.1, &key)) {
            return Some(std::mem::replace(&mut e.2, value));
        }
        bin.push((h, key, value));
        if bin.len() > TREEIFY_THRESHOLD {
            // `treeifyBin`: small tables grow instead.
            assert!(
                self.table.as_ref().map_or(0, Vec::len) < MIN_TREEIFY_CAPACITY,
                "java.util.HashMap would treeify a bin; tree-bin order is not modelled"
            );
            self.resize();
        }
        self.len += 1;
        if self.len > self.threshold {
            self.resize();
        }
        None
    }

    pub fn get(&self, key: &K, hash: i32, eq: impl Fn(&K, &K) -> bool) -> Option<&V> {
        let h = spread(hash);
        let table = self.table.as_ref()?;
        table[self.index(h)].iter().find(|e| e.0 == h && eq(&e.1, key)).map(|e| &e.2)
    }

    pub fn remove(&mut self, key: &K, hash: i32, eq: impl Fn(&K, &K) -> bool) -> Option<V> {
        let h = spread(hash);
        let i = self.index_if_allocated(h)?;
        let bin = &mut self.table.as_mut().expect("table")[i];
        let pos = bin.iter().position(|e| e.0 == h && eq(&e.1, key))?;
        self.len -= 1;
        Some(bin.remove(pos).2)
    }

    fn index_if_allocated(&self, h: u32) -> Option<usize> {
        self.table.as_ref().map(|_| self.index(h))
    }

    /// `putMapEntries`' sizing step before inserting `incoming` mappings in bulk (`putAll`).
    pub fn reserve_for_put_all(&mut self, incoming: usize) {
        if incoming == 0 {
            return;
        }
        if self.table.is_none() {
            let t = capacity_for_mappings(incoming).min(MAXIMUM_CAPACITY);
            if t > self.threshold {
                self.threshold = table_size_for(t);
            }
        } else {
            while incoming > self.threshold && self.table.as_ref().map_or(0, Vec::len) < MAXIMUM_CAPACITY {
                self.resize();
            }
        }
    }

    pub fn clear(&mut self) {
        if let Some(t) = self.table.as_mut() {
            t.iter_mut().for_each(Vec::clear);
        }
        self.len = 0;
    }

    /// Entries in Java's iteration order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.table.iter().flatten().flatten().map(|e| (&e.1, &e.2))
    }
}
