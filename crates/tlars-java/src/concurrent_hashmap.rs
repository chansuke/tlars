//! `java.util.concurrent.ConcurrentHashMap` (JDK 21) with Java's iteration order, for the
//! maps TLC iterates (`IdThread.namedRegisters`, whose key set decides the order
//! `AbstractChecker.getAllNamedRegisterValues` fills its result in). Keys bring their own
//! `hashCode` and `equals`, as in [`crate::JavaHashMap`].
//!
//! Modelled for a map one thread fills with `put`: lazy table allocation at
//! `DEFAULT_CAPACITY`, `spread = (h ^ (h >>> 16)) & HASH_BITS`, append within a bin,
//! order-preserving lo/hi split on `transfer`, the `addCount` resize at `sizeCtl`, and
//! `treeifyBin` growing a table below 64 through `tryPresize` instead of building a tree.
//! A bin that would become a tree is flagged by [`JavaConcurrentHashMap::needs_tree_bin`]
//! rather than guessed at, because a tree bin iterates in another order.

use crate::hashmap::table_size_for;

const DEFAULT_CAPACITY: usize = 16;
const MAXIMUM_CAPACITY: usize = 1 << 30;
const TREEIFY_THRESHOLD: usize = 8;
const MIN_TREEIFY_CAPACITY: usize = 64;
const HASH_BITS: i32 = 0x7fff_ffff;

pub struct JavaConcurrentHashMap<K, V> {
    /// `None` until the first insertion allocates the table (`initTable`).
    table: Option<Vec<Vec<(i32, K, V)>>>,
    /// `sizeCtl` once the table exists: the size at which `addCount` doubles it.
    size_ctl: usize,
    len: usize,
    tree_bin: bool,
}

/// `ConcurrentHashMap.spread`: the high half folded in, then the sign bit cleared.
fn spread(h: i32) -> i32 {
    let folded = ((h as u32) ^ ((h as u32) >> 16)) as i32;
    folded & HASH_BITS
}

impl<K, V> Default for JavaConcurrentHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> JavaConcurrentHashMap<K, V> {
    /// `new ConcurrentHashMap<>()`.
    pub fn new() -> Self {
        JavaConcurrentHashMap { table: None, size_ctl: 0, len: 0, tree_bin: false }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether a bin grew past `TREEIFY_THRESHOLD` in a table of at least
    /// `MIN_TREEIFY_CAPACITY`, where Java replaces the chain with a red-black tree. The
    /// iteration order of a tree bin is not modelled, so a caller whose output depends on the
    /// order has to stop here instead of printing a guess.
    pub fn needs_tree_bin(&self) -> bool {
        self.tree_bin
    }

    fn capacity(&self) -> usize {
        self.table.as_ref().map_or(0, Vec::len)
    }

    /// `initTable` with `sizeCtl == 0`: `DEFAULT_CAPACITY` bins and `sizeCtl = n - (n >>> 2)`.
    fn init_table(&mut self, capacity: usize) {
        let n = if capacity == 0 { DEFAULT_CAPACITY } else { capacity };
        self.table = Some((0..n).map(|_| Vec::new()).collect());
        self.size_ctl = n - (n >> 2);
    }

    /// `transfer(tab, null)`: a table of twice the size, each bin split into the entries that
    /// stay and the entries that move up by the old size.
    ///
    /// The split is not order preserving, unlike `java.util.HashMap`'s. `transfer` finds
    /// `lastRun`, the start of the bin's final run of entries that go the same way, reuses
    /// that run as it stands, and *prepends* every entry ahead of it one by one, so those come
    /// out reversed in front of the reused run.
    fn transfer(&mut self) {
        let old = self.table.take().expect("a table to transfer");
        let n = old.len();
        let mut new_tab: Vec<Vec<(i32, K, V)>> = (0..n << 1).map(|_| Vec::new()).collect();
        let moves_up = |h: i32| h as usize & n != 0;
        for (i, mut bin) in old.into_iter().enumerate() {
            if bin.is_empty() {
                continue;
            }
            let mut run_bit = moves_up(bin[0].0);
            let mut last_run = 0;
            for (k, e) in bin.iter().enumerate().skip(1) {
                if moves_up(e.0) != run_bit {
                    run_bit = moves_up(e.0);
                    last_run = k;
                }
            }
            let ahead: Vec<(i32, K, V)> = bin.drain(..last_run).collect();
            let (mut lo, mut hi) = if run_bit { (Vec::new(), bin) } else { (bin, Vec::new()) };
            for e in ahead {
                let side = if moves_up(e.0) { &mut hi } else { &mut lo };
                side.insert(0, e);
            }
            new_tab[i] = lo;
            new_tab[i + n] = hi;
        }
        self.table = Some(new_tab);
        // `sizeCtl = (n << 1) - (n >>> 1)` with `n` the old size, which is 0.75 of the new one.
        self.size_ctl = (n << 1) - (n >> 1);
    }

    /// `addCount`: double the table while the size has reached `sizeCtl`.
    fn add_count(&mut self) {
        while self.len >= self.size_ctl && self.capacity() > 0 && self.capacity() < MAXIMUM_CAPACITY {
            self.transfer();
        }
    }

    /// `tryPresize(size)`, which `treeifyBin` calls for a table below `MIN_TREEIFY_CAPACITY`.
    fn try_presize(&mut self, size: usize) {
        let c = if size >= MAXIMUM_CAPACITY >> 1 { MAXIMUM_CAPACITY } else { table_size_for(size + (size >> 1) + 1) };
        loop {
            let n = self.capacity();
            if n == 0 {
                self.init_table(c);
            } else if c <= self.size_ctl || n >= MAXIMUM_CAPACITY {
                return;
            } else {
                self.transfer();
            }
        }
    }

    /// `treeifyBin(tab, i)`: a small table grows instead of growing a tree.
    fn treeify_bin(&mut self) {
        let n = self.capacity();
        if n < MIN_TREEIFY_CAPACITY {
            self.try_presize(n << 1);
        } else {
            self.tree_bin = true;
        }
    }

    /// `put(key, value)`; returns the previous value. `hash` is the key's `hashCode`.
    pub fn put(&mut self, key: K, hash: i32, value: V, eq: impl Fn(&K, &K) -> bool) -> Option<V> {
        let h = spread(hash);
        if self.table.is_none() {
            self.init_table(0);
        }
        let i = (self.capacity() - 1) & h as usize;
        let bin = &mut self.table.as_mut().expect("table")[i];
        // `binCount` is how many nodes `putVal` walked: the position of an existing key, or
        // the length of the chain it appended to.
        let found = bin.iter().position(|e| e.0 == h && eq(&e.1, &key));
        let bin_count = found.map_or(bin.len(), |p| p + 1);
        let old = match found {
            Some(p) => Some(std::mem::replace(&mut bin[p].2, value)),
            None => {
                bin.push((h, key, value));
                self.len += 1;
                None
            }
        };
        if bin_count >= TREEIFY_THRESHOLD {
            self.treeify_bin();
        }
        if old.is_none() {
            self.add_count();
        }
        old
    }

    pub fn get(&self, key: &K, hash: i32, eq: impl Fn(&K, &K) -> bool) -> Option<&V> {
        let h = spread(hash);
        let table = self.table.as_ref()?;
        table[(table.len() - 1) & h as usize].iter().find(|e| e.0 == h && eq(&e.1, key)).map(|e| &e.2)
    }

    /// Entries in Java's iteration order: bin by bin, each chain from its head.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.table.iter().flatten().flatten().map(|e| (&e.1, &e.2))
    }
}

#[cfg(test)]
mod tests {
    use super::JavaConcurrentHashMap;
    use crate::string_hash_code;

    fn order(keys: &[&str]) -> Vec<String> {
        let mut map = JavaConcurrentHashMap::new();
        for k in keys {
            map.put((*k).to_string(), string_hash_code(k), (), |a, b| a == b);
        }
        assert!(!map.needs_tree_bin(), "no tree bin in these tests");
        map.iter().map(|(k, _)| k.clone()).collect()
    }

    /// Two keys in different bins come out in bin order, whichever order they were put in.
    /// Recorded from JDK 21: `String.hashCode("s:k21")` lands in bin 3 and `"s:k0"` in bin 9.
    #[test]
    fn bins_are_visited_in_index_order() {
        assert_eq!(order(&["s:k0", "s:k21"]), vec!["s:k21", "s:k0"]);
        assert_eq!(order(&["s:k21", "s:k0"]), vec!["s:k21", "s:k0"]);
    }

    /// Keys that share a bin keep the order they were put in, and a repeated put keeps the
    /// key's place. Recorded from JDK 21: these three all land in bin 11 of a table of 16.
    #[test]
    fn a_shared_bin_keeps_insertion_order() {
        assert_eq!(order(&["s:a", "s:q", "s:ah"]), vec!["s:a", "s:q", "s:ah"]);
        assert_eq!(order(&["s:ah", "s:q", "s:a"]), vec!["s:ah", "s:q", "s:a"]);
        assert_eq!(order(&["s:q", "s:a", "s:ah", "s:q"]), vec!["s:q", "s:a", "s:ah"]);
    }

    /// The twelfth entry reaches `sizeCtl` and doubles the table, which re-bins every key.
    #[test]
    fn the_table_doubles_at_three_quarters_full() {
        let keys: Vec<String> = (0..12).map(|i| format!("s:k{i}")).collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let mut map = JavaConcurrentHashMap::new();
        for k in &refs[..11] {
            map.put((*k).to_string(), string_hash_code(k), (), |a, b| a == b);
        }
        let before = map.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
        map.put(refs[11].to_string(), string_hash_code(refs[11]), (), |a, b| a == b);
        let after: Vec<String> = map.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(before.len(), 11);
        assert_eq!(after.len(), 12);
        assert_ne!(after[..11].to_vec(), before, "the resize re-bins the keys");
    }
}
