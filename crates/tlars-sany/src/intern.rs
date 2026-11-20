//! `util.UniqueString` + `util.InternTable`, session-scoped.
//!
//! Invariant: `Atom`'s `u32` *is* TLC's `tok`, assigned `++tokenCnt` on first sight.
//! `Ord for Atom` is therefore `UniqueString.compareTo` (`tok - other.tok`), which is what
//! orders strings, model values, record fields and string-domain functions everywhere.
//!
//! The intern *sequence* is reproduced by interning at the same program points TLC does
//! (cfg values, `SyntaxTreeNode` construction, runtime string creation) and by modelling
//! Java static initialisers as explicit [`JavaClass`] events, triggered where TLC first
//! touches the class. Nothing replays a reconstructed order.

use std::sync::{Arc, Mutex, RwLock};

/// An interned string. Copy, 4 bytes, compared by intern token.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Atom(u32);

impl Atom {
    /// The `tok` TLC writes into fingerprints of model values (`UniqueString.fingerPrint`).
    pub fn tok(self) -> i32 {
        self.0 as i32
    }
}

/// Java classes whose static initialisers create `UniqueString`s. Order of the variants is
/// irrelevant; *when* `ensure` is called is what matters. Each class's string list is
/// generated from the pinned sources into `fidelity::class_statics`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaClass {
    /// `Context.reInit()` + `BuiltInOperators` (SANY built-ins, re-created per parse).
    ContextReInit,
    /// `SyntaxTreeNode` statics (`***I do not exist***`, `--TLA+ BUILTINS--`).
    SyntaxTreeNode,
    /// `OpDefNode` statics (`Formal_0..2`).
    OpDefNode,
    /// `Operators` table and synonyms (incl. Unicode).
    Operators,
    /// `SyntaxTreeConstants.SyntaxNodeImage` (first touched after the third root token).
    SyntaxTreeConstants,
    /// `Location.unknown`.
    Location,
    /// TLC-side statics read at runtime (`TLCGetSet` keys, `Action.UNNAMED_ACTION`, ...).
    TlcRuntime,
}

impl JavaClass {
    pub const COUNT: usize = 7;
}

/// Session intern table. `Sync`: workers intern runtime strings (`\o`, `ToString`, JSON).
/// With N workers the token order of *runtime* strings is thread-dependent, exactly as in TLC.
pub struct Interner {
    table: RwLock<InternTable>,
    initialised: Mutex<[bool; JavaClass::COUNT]>,
}

struct InternTable {
    texts: Vec<Arc<str>>,
    // Lookup index only; never iterated, so its order cannot leak into output.
    #[allow(clippy::disallowed_types)]
    index: std::collections::HashMap<Arc<str>, Atom>,
}

impl Default for Interner {
    fn default() -> Self {
        Interner::new()
    }
}

impl Interner {
    /// Fresh table at TLC process start (no strings yet: the cfg is parsed first).
    pub fn new() -> Self {
        Interner {
            table: RwLock::new(InternTable { texts: Vec::new(), index: Default::default() }),
            initialised: Mutex::new([false; JavaClass::COUNT]),
        }
    }

    /// `UniqueString.uniqueStringOf`: the first string gets token 1 (`++tokenCnt`).
    pub fn intern(&self, s: &str) -> Atom {
        if let Some(a) = self.table.read().expect("intern table").index.get(s) {
            return *a;
        }
        let mut t = self.table.write().expect("intern table");
        if let Some(a) = t.index.get(s) {
            return *a;
        }
        let text: Arc<str> = Arc::from(s);
        t.texts.push(text.clone());
        let a = Atom(u32::try_from(t.texts.len()).expect("intern table overflow"));
        t.index.insert(text, a);
        a
    }

    /// `UniqueString.uidToUniqueString`: the atom with token `tok`, if handed out.
    pub fn uid_to_atom(&self, tok: i32) -> Option<Atom> {
        let n = self.len();
        usize::try_from(tok).ok().filter(|t| (1..=n).contains(t)).map(|t| Atom(t as u32))
    }

    pub fn text(&self, a: Atom) -> Arc<str> {
        self.table.read().expect("intern table").texts[a.0 as usize - 1].clone()
    }

    /// `UniqueString.hashCode()` == `String.hashCode()`; feeds `tlars_java::Hashtable`.
    pub fn java_hash(&self, a: Atom) -> i32 {
        tlars_java::string_hash_code(&self.text(a))
    }

    /// Run `class`'s static initialiser once: intern its static strings in source order. A
    /// string an earlier event already interned keeps the token it got then, exactly as
    /// `uniqueStringOf` hands back the existing `UniqueString`.
    pub fn ensure(&self, class: JavaClass) {
        {
            let mut done = self.initialised.lock().expect("class initialisers");
            if done[class as usize] {
                return;
            }
            done[class as usize] = true;
        }
        for s in crate::fidelity::class_statics(class) {
            self.intern(s);
        }
    }

    /// Number of tokens handed out; runtime strings created during search come after this.
    pub fn len(&self) -> usize {
        self.table.read().expect("intern table").texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
