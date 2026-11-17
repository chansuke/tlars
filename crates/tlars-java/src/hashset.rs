//! `java.util.HashSet<Integer>` iteration order, for messages that print such a set with
//! `AbstractCollection.toString` (e.g. TLC's `-suppressMessages`/`-messagesAsErrors`
//! overlap error). Models `HashMap`'s power-of-two table, `hash = h ^ (h >>> 16)`, append
//! within a bucket, and order-preserving lo/hi splits on resize. Bins never reach the
//! treeify threshold at the sizes TLC prints.

#[derive(Clone)]
pub struct JavaIntHashSet {
    buckets: Vec<Vec<i32>>,
    len: usize,
}

fn spread(v: i32) -> u32 {
    let h = v as u32;
    h ^ (h >> 16)
}

/// `HashMap.tableSizeFor`: the smallest power of two `>= cap` (at least 1).
fn table_size_for(cap: usize) -> usize {
    cap.max(1).next_power_of_two()
}

impl Default for JavaIntHashSet {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaIntHashSet {
    /// `new HashSet<>()`: capacity 16 on first insert.
    pub fn new() -> Self {
        Self::with_table(16)
    }

    fn with_table(capacity: usize) -> Self {
        JavaIntHashSet { buckets: vec![Vec::new(); capacity], len: 0 }
    }

    /// `new HashSet<>(other)`: `HashMap.newHashMap(max(size, 12))` (JDK 19+), then `addAll`
    /// in `other`'s iteration order.
    pub fn copy_of(other: &JavaIntHashSet) -> Self {
        let n = other.len.max(12);
        let mut set = Self::with_table(table_size_for((n as f64 / 0.75).ceil() as usize));
        for v in other.iter() {
            set.add(v);
        }
        set
    }

    /// `add`; `false` if already present.
    pub fn add(&mut self, v: i32) -> bool {
        let mask = self.buckets.len() - 1;
        let bucket = &mut self.buckets[spread(v) as usize & mask];
        if bucket.contains(&v) {
            return false;
        }
        bucket.push(v);
        self.len += 1;
        if self.len > self.buckets.len() * 3 / 4 {
            self.resize();
        }
        true
    }

    pub fn contains(&self, v: i32) -> bool {
        self.buckets[spread(v) as usize & (self.buckets.len() - 1)].contains(&v)
    }

    /// `retainAll`/`removeIf`: removal never reorders the rest.
    pub fn retain(&mut self, mut keep: impl FnMut(i32) -> bool) {
        for bucket in &mut self.buckets {
            bucket.retain(|v| keep(*v));
        }
        self.len = self.buckets.iter().map(Vec::len).sum();
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Iteration order: buckets ascending, insertion order within a bucket.
    pub fn iter(&self) -> impl Iterator<Item = i32> + '_ {
        self.buckets.iter().flatten().copied()
    }

    /// `AbstractCollection.toString`: `[a, b, c]`.
    pub fn to_java_string(&self) -> String {
        let items: Vec<String> = self.iter().map(|v| v.to_string()).collect();
        format!("[{}]", items.join(", "))
    }

    /// Doubling preserves relative order: each old bucket splits into `lo` (same index) and
    /// `hi` (index + old capacity), in order.
    fn resize(&mut self) {
        let old_cap = self.buckets.len();
        let mut next = vec![Vec::new(); old_cap * 2];
        for (i, bucket) in std::mem::take(&mut self.buckets).into_iter().enumerate() {
            for v in bucket {
                let index = if spread(v) as usize & old_cap == 0 { i } else { i + old_cap };
                next[index].push(v);
            }
        }
        self.buckets = next;
    }
}
