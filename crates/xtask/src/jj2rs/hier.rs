//! The class hierarchy of the translated classes.
//!
//! A translated class with translated subclasses, and those subclasses, form a *family*
//! named after its root class `R`. A family is one Rust type: `enum RObj` with one variant
//! per concrete class, each holding a struct with every field from `R` down to that class.
//! References to any family member are `J<RObj>`, so upcasts and downcasts cost nothing;
//! `instanceof` and virtual calls `match` on the variant. A class without translated
//! subclasses or superclass stays a plain struct (`J<C>`).
//!
//! Interfaces whose implementors are all in one family stand for that family's type.
//! Translated exceptions (classes whose superclass chain ends in a library throwable) are
//! variants of the generated `JThrow`, one per family (or class).

use std::collections::{BTreeMap, BTreeSet};

/// Library exception classes and their superclasses, as far as `catch` needs them.
pub(crate) const EXTERN_THROWABLES: &[(&str, &str)] = &[
    ("Throwable", ""),
    ("Exception", "Throwable"),
    ("Error", "Throwable"),
    ("RuntimeException", "Exception"),
    ("IllegalArgumentException", "RuntimeException"),
    ("NumberFormatException", "IllegalArgumentException"),
    ("IllegalStateException", "RuntimeException"),
    ("UnsupportedOperationException", "RuntimeException"),
    ("NoSuchElementException", "RuntimeException"),
    ("IOException", "Exception"),
    ("UnsupportedEncodingException", "IOException"),
    ("FileNotFoundException", "IOException"),
    ("WrongInvocationException", "RuntimeException"),
    ("NullPointerException", "RuntimeException"),
    ("IndexOutOfBoundsException", "RuntimeException"),
    ("ArrayIndexOutOfBoundsException", "IndexOutOfBoundsException"),
];

pub(crate) fn is_extern_throwable(c: &str) -> bool {
    EXTERN_THROWABLES.iter().any(|(n, _)| *n == c)
}

/// The library superclass chain of `c`, `c` first.
pub(crate) fn extern_throwable_chain(c: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut cur = EXTERN_THROWABLES.iter().find(|(n, _)| *n == c);
    while let Some((n, p)) = cur {
        out.push(*n);
        cur = EXTERN_THROWABLES.iter().find(|(m, _)| m == p);
    }
    out
}

/// What emission needs to know about one translated class's place in the hierarchy.
#[derive(Clone, Debug, Default)]
pub(crate) struct Place {
    /// Translated superclass.
    pub(crate) parent: Option<String>,
    /// Library superclass (`Exception`, `Error`, ...) of the family root.
    pub(crate) extern_parent: Option<String>,
    pub(crate) interfaces: Vec<String>,
    pub(crate) is_interface: bool,
    pub(crate) is_abstract: bool,
    /// The enclosing class of a nested class.
    pub(crate) outer: Option<String>,
    /// A non-static nested class: holds its enclosing instance in `this__0`.
    pub(crate) is_inner: bool,
    /// Type parameters of a generic class, erased to `Object` in its members.
    pub(crate) type_params: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Hier {
    pub(crate) places: BTreeMap<String, Place>,
    /// Class -> family root, for classes in a family.
    pub(crate) root: BTreeMap<String, String>,
    /// Root -> members in declaration order.
    pub(crate) families: BTreeMap<String, Vec<String>>,
    /// Interface -> the family all its implementors are in.
    pub(crate) iface_root: BTreeMap<String, String>,
    /// Roots (family or plain class) that are exceptions.
    pub(crate) throwable_roots: BTreeSet<String>,
}

impl Hier {
    /// `classes` in declaration order.
    pub(crate) fn new(classes: Vec<(String, Place)>) -> Hier {
        let order: Vec<String> = classes.iter().map(|(c, _)| c.clone()).collect();
        let places: BTreeMap<String, Place> = classes.into_iter().collect();
        let mut h = Hier { places, ..Hier::default() };
        let has_child: BTreeSet<String> = h.places.values().filter_map(|p| p.parent.clone()).collect();
        for c in &order {
            if h.places[c].is_interface {
                continue;
            }
            let r = h.top(c);
            if r != *c || has_child.contains(c) {
                h.root.insert(c.clone(), r.clone());
                h.families.entry(r).or_default().push(c.clone());
            }
        }
        for c in &order {
            if !h.places[c].is_interface {
                continue;
            }
            let roots: BTreeSet<String> =
                order.iter().filter(|k| !h.places[*k].is_interface && h.implements(k, c)).map(|k| h.root_of(k)).collect();
            if roots.len() == 1 {
                h.iface_root.insert(c.clone(), roots.into_iter().next().expect("one"));
            }
        }
        for c in &order {
            let top = h.top(c);
            if h.places[&top].extern_parent.as_deref().is_some_and(is_extern_throwable) {
                h.throwable_roots.insert(h.root_of(c));
            }
        }
        h
    }

    /// The topmost translated superclass of `c` (or `c`).
    fn top(&self, c: &str) -> String {
        let mut cur = c.to_string();
        while let Some(p) = self.places.get(&cur).and_then(|p| p.parent.clone()) {
            cur = p;
        }
        cur
    }

    /// The family root, or the class itself when it is not in a family.
    pub(crate) fn root_of(&self, c: &str) -> String {
        self.root.get(c).cloned().unwrap_or_else(|| c.to_string())
    }

    pub(crate) fn in_family(&self, c: &str) -> bool {
        self.root.contains_key(c)
    }

    /// `c`, its superclass, ... up to the root.
    pub(crate) fn ancestors(&self, c: &str) -> Vec<String> {
        let mut out = vec![c.to_string()];
        while let Some(p) = self.places.get(out.last().expect("nonempty")).and_then(|p| p.parent.clone()) {
            out.push(p);
        }
        out
    }

    /// Whether class `k` is `c` or a subclass, or implements interface `c`.
    pub(crate) fn implements(&self, k: &str, c: &str) -> bool {
        self.ancestors(k).iter().any(|a| a == c || self.iface_extends(a, c))
    }

    /// Whether `a`'s interfaces (transitively) include `c`.
    fn iface_extends(&self, a: &str, c: &str) -> bool {
        let mut todo = vec![a.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(x) = todo.pop() {
            if !seen.insert(x.clone()) {
                continue;
            }
            for i in self.places.get(&x).map(|p| p.interfaces.clone()).unwrap_or_default() {
                if i == c {
                    return true;
                }
                todo.push(i);
            }
        }
        false
    }

    /// The concrete classes of `c`'s family that are `c` or below it (or implement it).
    pub(crate) fn concrete_below(&self, c: &str) -> Vec<String> {
        let root = self.iface_root.get(c).cloned().unwrap_or_else(|| self.root_of(c));
        self.families
            .get(&root)
            .map(|m| m.iter().filter(|k| !self.places[*k].is_abstract && self.implements(k, c)).cloned().collect())
            .unwrap_or_default()
    }

    /// The Rust type behind references to class or interface `c`.
    pub(crate) fn object_type(&self, c: &str) -> Option<String> {
        if let Some(r) = self.iface_root.get(c) {
            return Some(if self.in_family(r) { format!("{r}Obj") } else { r.clone() });
        }
        self.root.get(c).map(|r| format!("{r}Obj"))
    }
}

thread_local! {
    pub(crate) static HIER: std::cell::RefCell<Hier> = Default::default();
}

/// The `JThrow` variant carrying exceptions of class `c` (its family root).
pub(crate) fn throw_variant(c: &str) -> Option<String> {
    HIER.with(|h| {
        let h = h.borrow();
        let r = h.root_of(c);
        h.throwable_roots.contains(&r).then_some(r)
    })
}
