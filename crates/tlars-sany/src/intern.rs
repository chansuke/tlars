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

impl Interner {
    /// Fresh table at TLC process start (no strings yet: the cfg is parsed first).
    pub fn new() -> Self {
        todo!()
    }

    /// `UniqueString.uniqueStringOf`.
    pub fn intern(&self, s: &str) -> Atom {
        todo!()
    }

    pub fn text(&self, a: Atom) -> Arc<str> {
        todo!()
    }

    /// `UniqueString.hashCode()` == `String.hashCode()`; feeds `tlars_java::Hashtable`.
    pub fn java_hash(&self, a: Atom) -> i32 {
        tlars_java::string_hash_code(&self.text(a))
    }

    /// Run `class`'s static initialiser once: intern its static strings in source order.
    pub fn ensure(&self, class: JavaClass) {
        let _strings: &[&str] = crate::fidelity::class_statics(class);
        todo!()
    }

    /// Number of tokens handed out; runtime strings created during search come after this.
    pub fn len(&self) -> usize {
        todo!()
    }
}
