//! `java.util.Hashtable` with Java's exact layout and enumeration order. SANY's symbol
//! table is one of these, and TLC's variable order is its reversed enumeration.

/// Initial capacity 11, load factor 0.75, rehash to `2n+1` before inserting when
/// `count >= threshold`, new entries at the head of their bucket, enumeration from the last
/// bucket down. The key's hash is supplied by the caller because it must be *Java's* hash
/// of the key (`UniqueString.hashCode() == String.hashCode()`), not a Rust `Hash`.
pub struct Hashtable<K, V> {
    /// Bucket heads, indices into `entries`.
    buckets: Vec<Option<usize>>,
    /// Entry storage; a removed entry's slot stays `None` (it is unreachable from buckets).
    entries: Vec<Option<Entry<K, V>>>,
    count: usize,
    threshold: usize,
}

struct Entry<K, V> {
    hash: i32,
    key: K,
    value: V,
    next: Option<usize>,
}

const INITIAL_CAPACITY: usize = 11;
const LOAD_FACTOR: f32 = 0.75;

/// `(int) Math.min(capacity * loadFactor, MAX_ARRAY_SIZE + 1)`, computed in `float`.
fn threshold_for(capacity: usize) -> usize {
    (capacity as f32 * LOAD_FACTOR) as usize
}

fn index_for(hash: i32, len: usize) -> usize {
    (hash & 0x7FFF_FFFF) as usize % len
}

impl<K: Eq, V> Default for Hashtable<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq, V> Hashtable<K, V> {
    pub fn new() -> Self {
        Hashtable {
            buckets: vec![None; INITIAL_CAPACITY],
            entries: Vec::new(),
            count: 0,
            threshold: threshold_for(INITIAL_CAPACITY),
        }
    }

    /// `Hashtable.put`: replaces the value of an existing key in place (its position does
    /// not change) and returns the old value.
    pub fn put(&mut self, key: K, java_hash: i32, value: V) -> Option<V> {
        if let Some(i) = self.find(&key, java_hash) {
            return Some(std::mem::replace(&mut self.entry_mut(i).value, value));
        }
        if self.count >= self.threshold {
            self.rehash();
        }
        let index = index_for(java_hash, self.buckets.len());
        self.entries.push(Some(Entry { hash: java_hash, key, value, next: self.buckets[index] }));
        self.buckets[index] = Some(self.entries.len() - 1);
        self.count += 1;
        None
    }

    pub fn get(&self, key: &K, java_hash: i32) -> Option<&V> {
        self.find(key, java_hash).map(|i| &self.entry(i).value)
    }

    /// `Hashtable.remove`: unlinks the entry from its chain (the others keep their order).
    pub fn remove(&mut self, key: &K, java_hash: i32) -> Option<V> {
        let index = index_for(java_hash, self.buckets.len());
        let (mut prev, mut cur) = (None, self.buckets[index]);
        while let Some(i) = cur {
            let e = self.entry(i);
            if e.hash == java_hash && e.key == *key {
                let next = e.next;
                match prev {
                    Some(p) => self.entry_mut(p).next = next,
                    None => self.buckets[index] = next,
                }
                self.count -= 1;
                return self.entries[i].take().map(|e| e.value);
            }
            (prev, cur) = (Some(i), e.next);
        }
        None
    }

    fn entry(&self, i: usize) -> &Entry<K, V> {
        self.entries[i].as_ref().expect("live entry")
    }

    fn entry_mut(&mut self, i: usize) -> &mut Entry<K, V> {
        self.entries[i].as_mut().expect("live entry")
    }

    /// `Hashtable.keys()` / `elements()` enumeration order.
    pub fn elements(&self) -> Elements<'_, K, V> {
        Elements { table: self, bucket: self.buckets.len(), entry: None }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn find(&self, key: &K, java_hash: i32) -> Option<usize> {
        let mut cur = self.buckets[index_for(java_hash, self.buckets.len())];
        while let Some(i) = cur {
            let e = self.entry(i);
            if e.hash == java_hash && e.key == *key {
                return Some(i);
            }
            cur = e.next;
        }
        None
    }

    /// New capacity `(old << 1) + 1`; entries move walking old buckets from the last index
    /// down and each chain head-first, inserting at the new heads, which reverses chains.
    fn rehash(&mut self) {
        let new_capacity = (self.buckets.len() << 1) + 1;
        let old = std::mem::replace(&mut self.buckets, vec![None; new_capacity]);
        self.threshold = threshold_for(new_capacity);
        for head in old.into_iter().rev() {
            let mut cur = head;
            while let Some(i) = cur {
                cur = self.entry(i).next;
                let index = index_for(self.entry(i).hash, new_capacity);
                self.entry_mut(i).next = self.buckets[index];
                self.buckets[index] = Some(i);
            }
        }
    }
}

/// Iterator in Java `Enumeration` order.
pub struct Elements<'a, K, V> {
    table: &'a Hashtable<K, V>,
    bucket: usize,
    entry: Option<usize>,
}

impl<'a, K, V> Iterator for Elements<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while self.entry.is_none() && self.bucket > 0 {
            self.bucket -= 1;
            self.entry = self.table.buckets[self.bucket];
        }
        let e = self.table.entries[self.entry?].as_ref().expect("live entry");
        self.entry = e.next;
        Some((&e.key, &e.value))
    }
}
