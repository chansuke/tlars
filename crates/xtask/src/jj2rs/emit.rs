//! Rust emission for the Java subset. The output targets `tlars-sany`'s `parser::java`
//! runtime: every class `C` becomes `struct C` plus `impl J<C>` (instance methods take the
//! receiver as `&J<C>`), statics live in a lazily initialised `CStatics`, and every
//! translated method returns `JResult<_>` so Java exceptions propagate with `?`.
//!
//! Field reads go through `J::jread(|o| ...)` so no `RefCell` borrow outlives the read: a
//! callee may mutate the object the caller read an argument from.

use super::ast::{Expr, Method, Stmt, SwitchArm, Type, Var};
use super::hier::{self, HIER};
use super::parse::{Member, MemberKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// Classes whose Rust type is a `Copy` value rather than `J<_>`: `UniqueString` is interned,
/// so Java's identity comparison is value comparison; `LogLevel` is an enum.
const VALUE_CLASSES: &[&str] = &["UniqueString", "LogLevel", "ErrorCode", "ErrorLevel", "AtomicInteger", "BigInteger", "BigDecimal"];

/// A Java type, as far as emission needs to know it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum T {
    Bool,
    Int,
    Long,
    Char,
    Double,
    Str,
    /// A class with its type arguments (empty when not generic, raw, or `<>`).
    Obj(String, Vec<T>),
    Arr(Box<T>),
    /// Type parameter `i` of the receiver's class (extern signatures only).
    Param(usize),
    /// An extern parameter that takes any argument by reference.
    Any,
    Void,
    Null,
}

/// Library generics whose Rust type is erased as in Java (`J<JavaVector>` for every
/// `Vector<E>`): their sources mix raw and parameterized uses freely. Their methods that
/// return an element take the expected element type as a turbofish.
pub(crate) const ERASED_LIBRARY_GENERICS: &[&str] = &["JavaVector"];

thread_local! {
    /// The number of type parameters of each generic library class.
    pub(crate) static EXTERN_ARITY: std::cell::RefCell<BTreeMap<String, usize>> = Default::default();
    /// `Outer.Inner` -> the Rust name of a nested class renamed to avoid a clash.
    pub(crate) static NESTED_RENAMES: std::cell::RefCell<BTreeMap<String, String>> = Default::default();
}

/// A regex that is an alternation of literal strings (`\\{|\\}|"`), as those strings.
fn literal_alternatives(re: &str) -> Option<Vec<String>> {
    let mut alts = vec![String::new()];
    let mut chars = re.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let e = chars.next()?;
                if e.is_ascii_alphanumeric() {
                    return None;
                }
                alts.last_mut().expect("one").push(e);
            }
            '|' => alts.push(String::new()),
            '.' | '[' | ']' | '{' | '}' | '(' | ')' | '*' | '+' | '?' | '^' | '$' => return None,
            c => alts.last_mut().expect("one").push(c),
        }
    }
    alts.iter().all(|a| !a.is_empty()).then_some(alts)
}

/// The simple class name, with names that share one Rust type unified.
pub(crate) fn class_name(name: &str) -> String {
    let mut segs = name.rsplit('.');
    if let (Some(inner), Some(outer)) = (segs.next(), segs.next())
        && let Some(r) = NESTED_RENAMES.with(|m| m.borrow().get(&format!("{outer}.{inner}")).cloned())
    {
        return r;
    }
    match name.rsplit('.').next().unwrap_or(name) {
        // The parser treats tree nodes through both names; one Rust type serves both.
        "TreeNode" => "SyntaxTreeNode".into(),
        "ArrayDeque" => "Deque".into(),
        "StringBuilder" => "StringBuffer".into(),
        "SilentSanyOutput" => "SanyOutput".into(),
        // `ParseUnit` holds its parser as the `ParseTree` interface, which only it implements.
        "ParseTree" => "TLAplusParser".into(),
        "Set" => "HashSet".into(),
        "List" | "Collection" => "ArrayList".into(),
        "Map" => "HashMap".into(),
        other => other.into(),
    }
}

impl T {
    pub(crate) fn of(ty: &Type) -> T {
        T::of_with_params(ty, &[])
    }

    pub(crate) fn of_with_params(ty: &Type, params: &[String]) -> T {
        let base = match ty.name.rsplit('.').next().unwrap_or(&ty.name) {
            "int" | "short" | "byte" => T::Int,
            "long" => T::Long,
            "char" => T::Char,
            "boolean" => T::Bool,
            "double" | "float" => T::Double,
            "void" => T::Void,
            "String" => T::Str,
            // `Boolean`/`Long`/`Character` are their primitives: the sources never compare those
            // boxes with `null` or by identity (either would fail to compile). `Integer` is a
            // real box (`JInteger`): SANY tests level maps for `null`.
            "Long" => T::Long,
            "Boolean" => T::Bool,
            "Character" => T::Char,
            "Any" => T::Any,
            p if params.iter().any(|q| q == p) => T::Param(params.iter().position(|q| q == p).expect("param")),
            _ => T::Obj(class_name(&ty.name), ty.args.iter().map(|a| T::of_with_params(a, params)).collect()),
        };
        (0..ty.dims).fold(base, |t, _| T::Arr(Box::new(t)))
    }

    pub(crate) fn obj(c: &str) -> T {
        T::Obj(c.into(), Vec::new())
    }

    fn is_numeric(&self) -> bool {
        matches!(self, T::Int | T::Long | T::Char | T::Double)
    }

    fn is_ref(&self) -> bool {
        matches!(self, T::Str | T::Obj(..) | T::Arr(_) | T::Null)
    }

    fn class(&self) -> Option<&str> {
        match self {
            T::Obj(c, _) => Some(c),
            _ => None,
        }
    }

    /// The Rust type.
    pub(crate) fn rust(&self) -> String {
        match self {
            T::Bool => "bool".into(),
            T::Int => "i32".into(),
            T::Long => "i64".into(),
            T::Char => "u16".into(),
            T::Double => "f64".into(),
            T::Str => "JStr".into(),
            T::Obj(c, _) if VALUE_CLASSES.contains(&c.as_str()) => c.clone(),
            T::Obj(c, _) if c == "Class" => "JClass".into(),
            T::Obj(c, _) if c == "Integer" => "JInteger".into(),
            // A library exception is held as the `JThrow` it travels in.
            T::Obj(c, _) if hier::is_extern_throwable(c) => "JThrow".into(),
            T::Obj(c, _) if HIER.with(|h| h.borrow().object_type(c).is_some()) => {
                format!("J<{}>", HIER.with(|h| h.borrow().object_type(c).expect("family")))
            }
            // Translated generic classes are erased: no type arguments in Rust; so are the
            // library classes that hold `Object`s (`ERASED_LIBRARY_GENERICS`).
            T::Obj(c, _) if HIER.with(|h| h.borrow().places.contains_key(c)) || ERASED_LIBRARY_GENERICS.contains(&c.as_str()) => {
                format!("J<{c}>")
            }
            // A raw library generic (`Vector`) holds `Object`s.
            T::Obj(c, args) if args.is_empty() => match EXTERN_ARITY.with(|a| a.borrow().get(c).copied()).unwrap_or(0) {
                0 => format!("J<{c}>"),
                n => format!("J<{c}<{}>>", vec!["J<Object>"; n].join(", ")),
            },
            T::Obj(c, args) => format!("J<{c}<{}>>", args.iter().map(T::rust).collect::<Vec<_>>().join(", ")),
            T::Arr(e) => format!("JArr<{}>", e.rust()),
            T::Param(i) => format!("__Param{i}"),
            T::Any => "__Any".into(),
            T::Void => "()".into(),
            T::Null => "J<()>".into(),
        }
    }

    /// Java's default value for a field or array element of this type.
    pub(crate) fn default_value(&self) -> String {
        match self {
            T::Bool => "false".into(),
            T::Int | T::Long | T::Char => "0".into(),
            T::Double => "0.0".into(),
            T::Void => "()".into(),
            _ => "JNull::null()".into(),
        }
    }

    /// Whether a type parameter occurs in this type.
    pub(crate) fn mentions_param(&self) -> bool {
        match self {
            T::Param(_) => true,
            T::Obj(_, a) => a.iter().any(T::mentions_param),
            T::Arr(e) => e.mentions_param(),
            _ => false,
        }
    }

    /// Replaces the receiver's type parameters with its type arguments.
    fn subst(&self, args: &[T]) -> T {
        match self {
            T::Param(i) => args.get(*i).cloned().unwrap_or_else(|| T::obj("Object")),
            T::Obj(c, a) => T::Obj(c.clone(), a.iter().map(|t| t.subst(args)).collect()),
            T::Arr(e) => T::Arr(Box::new(e.subst(args))),
            other => other.clone(),
        }
    }
}

/// Binary numeric promotion.
fn promote(a: &T, b: &T) -> T {
    if *a == T::Double || *b == T::Double {
        T::Double
    } else if *a == T::Long || *b == T::Long {
        T::Long
    } else {
        T::Int
    }
}

/// Whether a value of type `from` can be passed where `to` is expected (Java method
/// invocation conversion), and whether it is an exact match.
fn assignable(from: &T, to: &T) -> Option<bool> {
    let widening =
        |f: &T, t: &T| matches!((f, t), (T::Char, T::Int | T::Long | T::Double) | (T::Int, T::Long | T::Double) | (T::Long, T::Double));
    match (from, to) {
        (a, b) if a == b => Some(true),
        (_, T::Any) => Some(false),
        (T::Null, t) if t.is_ref() => Some(false),
        // Boxing conversions (method invocation phase 2).
        (T::Int, T::Obj(b, _)) | (T::Obj(b, _), T::Int) if b == "Integer" => Some(false),
        (T::Obj(a, _), T::Obj(b, _)) if a == b || b == "Object" || HIER.with(|h| h.borrow().implements(a, b)) => Some(false),
        (T::Str | T::Arr(_), T::Obj(b, _)) if b == "Object" => Some(false),
        // Arrays of references are covariant.
        (T::Arr(a), T::Arr(b)) if a.is_ref() && b.is_ref() => assignable(a, b).map(|_| false),
        (f, t) if widening(f, t) => Some(false),
        _ => None,
    }
}

/// What the emitter knows about a class: fields and methods with their types.
#[derive(Clone, Default)]
pub(crate) struct ClassInfo {
    /// Name -> `(type, is_static)`.
    pub(crate) fields: BTreeMap<String, (T, bool)>,
    /// Name -> overloads; constructors are `<init>`.
    pub(crate) methods: BTreeMap<String, Vec<MethodSig>>,
    /// Interfaces whose fields resolve unqualified (`implements Constants`).
    pub(crate) inherits: Vec<String>,
    pub(crate) type_params: Vec<String>,
    /// Implemented by hand in the support module.
    pub(crate) is_extern: bool,
    /// Has a static initialiser (static fields that are not `const`s, or `static {}`).
    pub(crate) has_statics: bool,
    /// A library enum: `E.NAME` reads `E::of("NAME")`.
    pub(crate) is_enum: bool,
}

#[derive(Clone)]
pub(crate) struct MethodSig {
    pub(crate) params: Vec<T>,
    pub(crate) ret: T,
    pub(crate) is_static: bool,
    /// Returns `JResult` (every translated method; externs that throw checked exceptions).
    pub(crate) fallible: bool,
    /// The Rust name: the method name, or with an overload suffix; for constructors the
    /// suffix of `new_*`/`init_*`.
    pub(crate) rust_name: String,
    /// Index of the declaring member, for translated classes.
    pub(crate) member: Option<usize>,
    /// The last parameter is `T...`.
    pub(crate) varargs: bool,
    /// Has a body (not abstract or an interface method).
    pub(crate) has_body: bool,
    /// The declared return type mentions a type parameter (before substitution).
    pub(crate) returns_param: bool,
}

pub(crate) type Classes = BTreeMap<String, ClassInfo>;

/// Gives overloads distinct Rust names: the Java name when unique, else suffixed with the
/// arity, and with the declaration index among same-arity overloads when needed.
pub(crate) fn name_overloads(info: &mut ClassInfo) {
    for (name, sigs) in &mut info.methods {
        let arities: Vec<usize> = sigs.iter().map(|s| s.params.len()).collect();
        for (i, sig) in sigs.iter_mut().enumerate() {
            let n = arities[i];
            let same: Vec<usize> = (0..arities.len()).filter(|&j| arities[j] == n).collect();
            let suffix = if same.len() == 1 { n.to_string() } else { format!("{n}_{}", same.iter().position(|&j| j == i).expect("self")) };
            sig.rust_name = if name == "<init>" {
                suffix
            } else if arities.len() == 1 {
                rust_ident(name)
            } else {
                format!("{name}_{suffix}")
            };
        }
    }
}

/// `name(Type, ...)` with simple type names: how hand-written members are listed.
pub(crate) fn signature_key(name: &str, params: &[(Type, String)]) -> String {
    let ps: Vec<String> =
        params.iter().map(|(t, _)| format!("{}{}", t.name.rsplit('.').next().unwrap_or(&t.name), "[]".repeat(t.dims))).collect();
    format!("{name}({})", ps.join(","))
}

/// Whether every class `t` mentions is in `known`.
fn known_type(t: &T, known: &BTreeSet<String>) -> bool {
    match t {
        T::Obj(c, args) => known.contains(c) && args.iter().all(|a| known_type(a, known)),
        T::Arr(e) => known_type(e, known),
        _ => true,
    }
}

/// The type of a declared variable, with raw-type overrides (`Vector loe` holding
/// `ParseError`s) applied.
fn var_type(class: &str, ty: &Type, v: &Var, overrides: &BTreeMap<(String, String), Type>) -> T {
    match overrides.get(&(class.to_string(), v.name.clone())) {
        Some(o) => T::of(o),
        None => T::of(&Type { dims: ty.dims + v.extra_dims, ..ty.clone() }),
    }
}

/// Builds the class table entry for translated `members`. Methods whose signature mentions a
/// class outside `known` are left out (and listed in `skipped`): the translation cannot call
/// them, so a call to one is an emission error.
pub(crate) fn class_info(
    class: &str,
    members: &[Member],
    inherits: Vec<String>,
    is_interface: bool,
    known: &BTreeSet<String>,
    overrides: &BTreeMap<(String, String), Type>,
    skipped: &mut Vec<String>,
) -> ClassInfo {
    let mut info = ClassInfo { inherits, ..ClassInfo::default() };
    let (parent, throwable_root) = HIER.with(|h| {
        let h = h.borrow();
        let place = h.places.get(class).cloned().unwrap_or_default();
        let root_throwable = place.parent.is_none() && place.extern_parent.as_deref().is_some_and(hier::is_extern_throwable);
        (place.parent, root_throwable)
    });
    if let Some(p) = parent {
        info.inherits.insert(0, p);
    }
    // An inner class's enclosing instance (javac's `this$0`).
    if let Some(outer) = HIER.with(|h| h.borrow().places.get(class).filter(|p| p.is_inner).and_then(|p| p.outer.clone())) {
        info.fields.insert("this__0".into(), (T::obj(&outer), false));
    }
    // `Throwable.detailMessage`, set by `super(message)`.
    if throwable_root {
        info.fields.insert("detailMessage".into(), (T::Str, false));
    }
    for (idx, m) in members.iter().enumerate() {
        match &m.kind {
            MemberKind::Field { decl: Stmt::Local { ty, vars }, is_static, .. } => {
                for v in vars {
                    let is_static = *is_static || is_interface;
                    let t = var_type(class, ty, v, overrides);
                    // A field of a type the translation does not model is left out; code
                    // that uses it fails to emit.
                    if !known_type(&t, known) {
                        skipped.push(format!("{class}.{} (field)", v.name));
                        continue;
                    }
                    info.fields.insert(v.name.clone(), (t, is_static));
                    let is_const = CONSTS.with(|c| c.borrow().contains(&(class.to_string(), v.name.clone())));
                    info.has_statics |= is_static && !is_const;
                }
            }
            MemberKind::StaticInit(_) => info.has_statics = true,
            MemberKind::Constructor(method) | MemberKind::Method(method) => {
                let is_ctor = matches!(m.kind, MemberKind::Constructor(_));
                // Type parameters (of a generic method or class) are erased to `Object`.
                let class_params = HIER.with(|h| h.borrow().places.get(class).map(|p| p.type_params.clone()).unwrap_or_default());
                let all_params: Vec<String> = method.type_params.iter().chain(&class_params).cloned().collect();
                let erase = |t: &Type| erase_params(t, &all_params);
                let params: Vec<T> = method.params.iter().map(|(t, _)| T::of(&erase(t))).collect();
                let ret = if is_ctor { T::Void } else { T::of(&erase(&method.ret)) };
                if !params.iter().chain([&ret]).all(|t| known_type(t, known)) {
                    skipped.push(format!("{class}.{}", signature_key(&method.name, &method.params)));
                    continue;
                }
                let name = if is_ctor { "<init>".to_string() } else { method.name.clone() };
                info.methods.entry(name).or_default().push(MethodSig {
                    params,
                    ret,
                    is_static: is_ctor || method.is_static,
                    fallible: true,
                    rust_name: String::new(),
                    member: Some(idx),
                    varargs: method.varargs,
                    has_body: method.has_body,
                    returns_param: false,
                });
            }
            _ => {}
        }
    }
    // A class without constructors has Java's implicit no-argument one.
    if !members.iter().any(|m| matches!(m.kind, MemberKind::Constructor(_))) {
        info.methods.insert(
            "<init>".into(),
            vec![MethodSig {
                params: Vec::new(),
                ret: T::Void,
                is_static: true,
                fallible: true,
                rust_name: String::new(),
                member: None,
                varargs: false,
                has_body: true,
                returns_param: false,
            }],
        );
    }
    name_overloads(&mut info);
    info
}

/// `t` with the type parameters `params` replaced by `Object`.
pub(crate) fn erase_params(t: &Type, params: &[String]) -> Type {
    if params.contains(&t.name) {
        return Type { name: "Object".into(), args: Vec::new(), dims: t.dims };
    }
    Type { args: t.args.iter().map(|a| erase_params(a, params)).collect(), ..t.clone() }
}

/// One breakable construct: Java label (if any), Rust label, and the label `continue`
/// breaks to (loops only).
struct Breakable {
    java_label: Option<String>,
    rust_label: String,
    continue_label: Option<String>,
    /// `tries.len()` where the construct starts: a jump out of a `try` with `finally` would
    /// skip the `finally` in the emitted code.
    tries: usize,
}

/// A `try` being emitted: its Rust block label and `finally` body.
struct TryCx {
    label: String,
    finally: Option<Vec<Stmt>>,
}

pub(crate) struct Emitter<'c> {
    classes: &'c Classes,
    class: String,
    method_ret: T,
    is_static: bool,
    in_ctor: bool,
    scopes: Vec<BTreeMap<String, T>>,
    breakables: Vec<Breakable>,
    tries: Vec<TryCx>,
    /// Label attached to the next loop by an enclosing labeled statement.
    pending_label: Option<String>,
    counter: usize,
    pub(crate) errors: Vec<String>,
    /// Signature key of the member being emitted, for error reports.
    member: String,
}

impl<'c> Emitter<'c> {
    pub(crate) fn new(classes: &'c Classes, class: &str) -> Self {
        Emitter {
            classes,
            class: class.into(),
            method_ret: T::Void,
            is_static: false,
            in_ctor: false,
            scopes: Vec::new(),
            breakables: Vec::new(),
            tries: Vec::new(),
            pending_label: None,
            counter: 0,
            errors: Vec::new(),
            member: String::new(),
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("'{prefix}{}", self.counter)
    }

    fn err(&mut self, msg: String) -> String {
        self.errors.push(format!("{}.{}: {msg}", self.class, self.member));
        format!("compile_error!({msg:?})")
    }

    // ---- name resolution ----

    fn local(&self, name: &str) -> Option<&T> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// A field of `class` or of the classes it inherits from: `(owner, type, is_static)`.
    fn field_of(&self, class: &str, name: &str) -> Option<(String, T, bool)> {
        let info = self.classes.get(class)?;
        if let Some((t, s)) = info.fields.get(name) {
            return Some((class.to_string(), t.clone(), *s));
        }
        info.inherits.iter().find_map(|p| self.field_of(p, name))
    }

    /// The overloads of `name` callable with `arity` arguments in `class` and its supertypes,
    /// each with its declaring class; an override hides the overridden method.
    fn overloads(&self, class: &str, name: &str, arity: usize) -> Option<(String, Vec<MethodSig>)> {
        let mut found: Vec<(String, MethodSig)> = Vec::new();
        let mut todo = vec![class.to_string()];
        let mut seen = BTreeSet::new();
        while let Some(c) = todo.pop() {
            if !seen.insert(c.clone()) {
                continue;
            }
            let Some(info) = self.classes.get(&c) else { continue };
            for s in info.methods.get(name).into_iter().flatten() {
                let fits = s.params.len() == arity || (s.varargs && arity + 1 >= s.params.len());
                if fits && !found.iter().any(|(_, f)| f.params == s.params) {
                    found.push((c.clone(), s.clone()));
                }
            }
            // Constructors are not inherited.
            if name != "<init>" {
                todo.extend(info.inherits.iter().rev().cloned());
            }
        }
        let owner = found.first()?.0.clone();
        Some((owner, found.into_iter().map(|(_, s)| s).collect()))
    }

    /// Resolves a call: the overload all arguments are assignable to, preferring exact
    /// matches, with the receiver's type arguments substituted.
    fn resolve(&mut self, class: &str, name: &str, args: &[Expr], type_args: &[T]) -> Option<(String, MethodSig)> {
        let (owner, cands) = self.overloads(class, name, args.len())?;
        let cands: Vec<MethodSig> = cands
            .into_iter()
            .map(|s| MethodSig { params: s.params.iter().map(|p| p.subst(type_args)).collect(), ret: s.ret.subst(type_args), ..s })
            .collect();
        if cands.len() == 1 {
            return Some((owner, cands.into_iter().next().expect("one")));
        }
        let arg_types: Vec<T> = args.iter().map(|a| self.type_of(a)).collect();
        let best = cands
            .into_iter()
            .filter_map(|s| {
                let fit: Option<Vec<bool>> = if s.varargs
                    && !(arg_types.len() == s.params.len()
                        && assignable(&arg_types[arg_types.len() - 1], &s.params[s.params.len() - 1]).is_some())
                {
                    let T::Arr(elem) = &s.params[s.params.len() - 1] else { return None };
                    let n = s.params.len() - 1;
                    let fixed: Option<Vec<bool>> = arg_types[..n].iter().zip(&s.params).map(|(a, p)| assignable(a, p)).collect();
                    let rest: Option<Vec<bool>> = arg_types[n..].iter().map(|a| assignable(a, elem)).collect();
                    fixed.zip(rest).map(|(mut f, r)| {
                        f.extend(r.iter().map(|_| false));
                        f
                    })
                } else {
                    if arg_types.len() != s.params.len() {
                        return None;
                    }
                    arg_types.iter().zip(&s.params).map(|(a, p)| assignable(a, p)).collect()
                };
                fit.map(|f| (f.iter().filter(|e| **e).count(), s))
            })
            .max_by_key(|(exact, _)| *exact)?;
        Some((owner, best.1))
    }

    /// The type of `e` without keeping its code (errors from it are discarded).
    fn type_of(&mut self, e: &Expr) -> T {
        let (errors, counter) = (self.errors.len(), self.counter);
        let t = self.expr(e).1;
        self.errors.truncate(errors);
        self.counter = counter;
        t
    }

    /// `e` names a class (`Integer`, `java.lang.Integer`, `TLAConstants.KeyWords`) rather
    /// than a value.
    fn class_ref(&self, e: &Expr) -> Option<String> {
        fn dotted(e: &Expr) -> Option<String> {
            match e {
                Expr::Name(n) => Some(n.clone()),
                Expr::Field(t, f) => Some(format!("{}.{f}", dotted(t)?)),
                _ => None,
            }
        }
        let path = dotted(e)?;
        let first = path.split('.').next().unwrap_or(&path);
        if self.local(first).is_some() || self.field_of(&self.class, first).is_some() {
            return None;
        }
        let class = class_name(&path);
        (self.classes.contains_key(&class) || class == "String").then_some(class)
    }

    /// The enclosing classes, innermost first, each with an expression for its instance when
    /// one is reachable (through `this__0` of inner classes).
    fn enclosing(&self) -> Vec<(Option<String>, String)> {
        let mut out = Vec::new();
        let mut recv = if self.is_static { None } else { Some("self".to_string()) };
        let mut cur = self.class.clone();
        loop {
            let Some(p) = HIER.with(|h| h.borrow().places.get(&cur).cloned()) else { break };
            let Some(outer) = p.outer else { break };
            recv = match (recv, p.is_inner) {
                (Some(r), true) => Some(format!("{r}.jread(|o| o.this__0.clone())")),
                _ => None,
            };
            out.push((recv.clone(), outer.clone()));
            cur = outer;
        }
        out
    }

    /// An expression for the enclosing instance of class `outer`, from the current class.
    fn outer_instance(&self, outer: &str) -> Option<String> {
        if self.class == outer && !self.is_static {
            return Some("self.clone()".into());
        }
        self.enclosing().into_iter().find(|(_, c)| c == outer).and_then(|(r, _)| r)
    }

    fn is_extern(&self, class: &str) -> bool {
        self.classes.get(class).is_some_and(|c| c.is_extern)
    }

    /// Reads a field of `owner`: a constant, a static, or an instance field of `recv`.
    fn read_field(&mut self, owner: &str, name: &str, t: &T, is_static: bool, recv: Option<String>) -> String {
        if is_static {
            if self.is_const(owner, name) {
                return format!("{owner}::{name}");
            }
            if self.classes.get(owner).is_some_and(|c| c.is_enum) {
                return format!("{owner}::of({name:?})");
            }
            if self.is_extern(owner) {
                return format!("{owner}::{name}()");
            }
            return format!("{owner}::statics().jread(|o| o.{}{})", rust_ident(name), if t.is_ref() { ".clone()" } else { "" });
        }
        let recv = recv.unwrap_or_else(|| "self".into());
        let clone = if t.is_ref() { ".clone()" } else { "" };
        if HIER.with(|h| h.borrow().in_family(owner)) {
            let deref = if t.is_ref() { "" } else { "*" };
            return format!("{recv}.jread(|o| {deref}o.{owner}_{name}(){clone})");
        }
        format!("{recv}.jread(|o| o.{}{clone})", rust_ident(name))
    }

    /// Static final primitive fields with literal initialisers are Rust `const`s.
    fn is_const(&self, owner: &str, name: &str) -> bool {
        self.classes.get(owner).is_some_and(|c| c.fields.get(name).is_some_and(|(t, s)| *s && !t.is_ref()))
            && CONSTS.with(|c| c.borrow().contains(&(owner.to_string(), name.to_string())))
    }

    // ---- statements ----

    fn indent(depth: usize) -> String {
        "    ".repeat(depth)
    }

    /// A method or constructor body as `fn fn_name`. `init_class` runs the class's static
    /// initialiser first (Java initialises a class on its first static call).
    pub(crate) fn method(&mut self, m: &Method, fn_name: &str, is_ctor: bool, init_class: bool) -> String {
        self.method_ret = if is_ctor { T::Void } else { T::of(&m.ret) };
        self.is_static = m.is_static;
        self.in_ctor = is_ctor;
        self.scopes = vec![BTreeMap::new()];
        let class_params = HIER.with(|h| h.borrow().places.get(&self.class).map(|p| p.type_params.clone()).unwrap_or_default());
        let erased: Vec<String> = m.type_params.iter().chain(&class_params).cloned().collect();
        if !erased.is_empty() {
            self.method_ret = if is_ctor { T::Void } else { T::of(&erase_params(&m.ret, &erased)) };
        }
        let mut params = Vec::new();
        for (ty, name) in &m.params {
            let t = T::of(&erase_params(ty, &erased));
            params.push(format!("mut {}: {}", rust_ident(name), t.rust()));
            self.scopes[0].insert(name.clone(), t);
        }
        let recv = if m.is_static {
            ""
        } else if params.is_empty() {
            "&self"
        } else {
            "&self, "
        };
        let mut out = format!("    pub(crate) fn {fn_name}({recv}{}) -> JResult<{}> {{\n", params.join(", "), self.method_ret.rust());
        if init_class {
            out.push_str(&class_init(&self.class));
        }
        // A constructor without `this(...)`/`super(...)` starts with an implicit `super()`.
        let explicit =
            matches!(m.body.first(), Some(Stmt::Expr(Expr::Call { target: None, name, .. })) if name == "this" || name == "super");
        if is_ctor && !explicit {
            let (code, _) = self.ctor_call("super", &[]);
            out.push_str(&format!("        {code};\n"));
        }
        for s in &m.body {
            out.push_str(&self.stmt(s, 2));
        }
        // javac proved every path returns; Rust needs the tail to diverge.
        if self.method_ret == T::Void {
            out.push_str("        #[allow(unreachable_code)]\n        Ok(())\n");
        } else {
            out.push_str("        #[allow(unreachable_code)]\n        unreachable!(\"javac: missing return\")\n");
        }
        out.push_str("    }\n");
        self.in_ctor = false;
        out
    }

    fn block(&mut self, stmts: &[Stmt], depth: usize) -> String {
        self.scopes.push(BTreeMap::new());
        let out: String = stmts.iter().map(|s| self.stmt(s, depth)).collect();
        self.scopes.pop();
        out
    }

    /// The statements that leave the current method: run `finally` bodies first.
    fn finallies(&mut self, depth: usize) -> String {
        let bodies: Vec<Vec<Stmt>> = self.tries.iter().rev().filter_map(|t| t.finally.clone()).collect();
        let saved = std::mem::take(&mut self.tries);
        let out = bodies.iter().map(|b| self.block(b, depth)).collect();
        self.tries = saved;
        out
    }

    /// Propagates an error value: to the innermost `try`, else out of the method.
    fn propagate(&self, err: &str) -> String {
        match self.tries.last() {
            Some(t) => format!("break {} Err({err})", t.label),
            None => format!("return Err({err})"),
        }
    }

    /// `expr?` for a fallible call, routed to the innermost `try`.
    fn tri(&self, call: String) -> String {
        match self.tries.last() {
            Some(t) => format!("(match {call} {{ Ok(v) => v, Err(e) => break {} Err(e) }})", t.label),
            None => format!("{call}?"),
        }
    }

    fn stmt(&mut self, s: &Stmt, depth: usize) -> String {
        let ind = Self::indent(depth);
        match s {
            Stmt::Empty => String::new(),
            Stmt::Unsupported(what) => format!("{ind}{};\n", self.err(format!("{what} (hand-write the member)"))),
            Stmt::Block(b) => format!("{ind}{{\n{}{ind}}}\n", self.block(b, depth + 1)),
            Stmt::Local { ty, vars } => vars.iter().map(|v| self.local_decl(ty, v, depth)).collect(),
            Stmt::Expr(e) => format!("{ind}{};\n", self.expr_stmt(e)),
            Stmt::If { cond, then, els } => {
                let c = self.cond(cond);
                let mut out = format!("{ind}if {c} {{\n{}", self.nested(then, depth + 1));
                match els {
                    Some(e) => out.push_str(&format!("{ind}}} else {{\n{}{ind}}}\n", self.nested(e, depth + 1))),
                    None => out.push_str(&format!("{ind}}}\n")),
                }
                out
            }
            Stmt::While { cond, body } => {
                let label = self.loop_label();
                let c = self.cond(cond);
                let head = if c == "true" { format!("{label}: loop") } else { format!("{label}: while {c}") };
                self.breakables.push(Breakable {
                    java_label: self.pending_label.take(),
                    rust_label: label.clone(),
                    continue_label: Some(label),
                    tries: self.tries.len(),
                });
                let body = self.nested(body, depth + 1);
                self.breakables.pop();
                format!("{ind}{head} {{\n{body}{ind}}}\n")
            }
            Stmt::DoWhile { body, cond } => {
                let label = self.loop_label();
                let inner = self.fresh("dobody");
                self.breakables.push(Breakable {
                    java_label: self.pending_label.take(),
                    rust_label: label.clone(),
                    continue_label: Some(inner.clone()),
                    tries: self.tries.len(),
                });
                let b = self.nested(body, depth + 2);
                self.breakables.pop();
                let c = self.cond(cond);
                format!("{ind}{label}: loop {{\n{ind}    {inner}: {{\n{b}{ind}    }}\n{ind}    if !({c}) {{ break; }}\n{ind}}}\n")
            }
            Stmt::For { init, cond, update, body } => {
                self.scopes.push(BTreeMap::new());
                let label = self.loop_label();
                let inner = self.fresh("forbody");
                let java_label = self.pending_label.take();
                let mut out = format!("{ind}{{\n");
                for i in init {
                    out.push_str(&self.stmt(i, depth + 1));
                }
                let c = cond.as_ref().map_or("true".into(), |c| self.cond(c));
                self.breakables.push(Breakable {
                    java_label,
                    rust_label: label.clone(),
                    continue_label: Some(inner.clone()),
                    tries: self.tries.len(),
                });
                let b = self.nested(body, depth + 3);
                self.breakables.pop();
                let ups: String = update.iter().map(|u| format!("{ind}        {};\n", self.expr_stmt(u))).collect();
                out.push_str(&format!(
                    "{ind}    {label}: loop {{\n{ind}        if !({c}) {{ break; }}\n{ind}        {inner}: {{\n{b}{ind}        }}\n{ups}{ind}    }}\n{ind}}}\n"
                ));
                self.scopes.pop();
                out
            }
            Stmt::ForEach { ty, name, iter, body } => {
                let (arr, at) = self.expr(iter);
                if let T::Obj(c, targs) = &at {
                    // `for (T x : collection)`: through the collection's `iterator()`.
                    let elem = match self.resolve(c, "iterator", &[], targs) {
                        Some((_, sig)) => match sig.ret {
                            T::Obj(_, a) if a.len() == 1 => a[0].clone(),
                            other => return format!("{ind}{};\n", self.err(format!("iterator() returning {other:?}"))),
                        },
                        None => return format!("{ind}{};\n", self.err(format!("for-each over {at:?}"))),
                    };
                    let label = self.loop_label();
                    let inner = self.fresh("forbody");
                    let t = T::of(ty);
                    self.breakables.push(Breakable {
                        java_label: self.pending_label.take(),
                        rust_label: label.clone(),
                        continue_label: Some(inner.clone()),
                        tries: self.tries.len(),
                    });
                    self.scopes.push(BTreeMap::from([(name.clone(), t.clone())]));
                    let b = self.block(&[(**body).clone()], depth + 3);
                    self.scopes.pop();
                    self.breakables.pop();
                    let value = convert("__it.next()", &elem, &t);
                    return format!(
                        "{ind}{{\n{ind}    let __it = ({arr}).iterator();\n{ind}    {label}: loop {{\n{ind}        if !__it.hasNext() {{ break; }}\n{ind}        let mut {}: {} = {value};\n{ind}        {inner}: {{\n{b}{ind}        }}\n{ind}    }}\n{ind}}}\n",
                        rust_ident(name),
                        t.rust()
                    );
                }
                if !matches!(at, T::Arr(_)) {
                    return format!("{ind}{};\n", self.err(format!("for-each over {at:?}")));
                }
                let label = self.loop_label();
                let inner = self.fresh("forbody");
                let t = T::of(ty);
                self.breakables.push(Breakable {
                    java_label: self.pending_label.take(),
                    rust_label: label.clone(),
                    continue_label: Some(inner.clone()),
                    tries: self.tries.len(),
                });
                self.scopes.push(BTreeMap::from([(name.clone(), t.clone())]));
                let b = self.block(&[(**body).clone()], depth + 3);
                self.scopes.pop();
                self.breakables.pop();
                format!(
                    "{ind}{{\n{ind}    let __arr = {arr};\n{ind}    let mut __i: i32 = 0;\n{ind}    {label}: loop {{\n{ind}        if __i >= __arr.length() {{ break; }}\n{ind}        let mut {}: {} = __arr.at(__i);\n{ind}        __i += 1;\n{ind}        {inner}: {{\n{b}{ind}        }}\n{ind}    }}\n{ind}}}\n",
                    rust_ident(name),
                    t.rust()
                )
            }
            Stmt::Labeled { label, body } => {
                if matches!(**body, Stmt::While { .. } | Stmt::DoWhile { .. } | Stmt::For { .. }) {
                    self.pending_label = Some(label.clone());
                    self.stmt(body, depth)
                } else {
                    let rl = self.fresh("blk");
                    self.breakables.push(Breakable {
                        java_label: Some(label.clone()),
                        rust_label: rl.clone(),
                        continue_label: None,
                        tries: self.tries.len(),
                    });
                    let b = self.nested(body, depth + 1);
                    self.breakables.pop();
                    format!("{ind}{rl}: {{\n{b}{ind}}}\n")
                }
            }
            Stmt::Break(label) => {
                let target = match label {
                    Some(l) => self.breakables.iter().rev().find(|b| b.java_label.as_deref() == Some(l.as_str())),
                    None => self.breakables.last(),
                };
                if let Some(b) = target
                    && self.tries[b.tries..].iter().any(|t| t.finally.is_some())
                {
                    return format!("{ind}{};\n", self.err("break out of try-finally".into()));
                }
                match target {
                    Some(b) => format!("{ind}break {};\n", b.rust_label),
                    None => format!("{ind}{};\n", self.err(format!("break without target {label:?}"))),
                }
            }
            Stmt::Continue(label) => {
                let target = self
                    .breakables
                    .iter()
                    .rev()
                    .find(|b| b.continue_label.is_some() && (label.is_none() || b.java_label.as_deref() == label.as_deref()));
                if let Some(b) = target
                    && self.tries[b.tries..].iter().any(|t| t.finally.is_some())
                {
                    return format!("{ind}{};\n", self.err("continue out of try-finally".into()));
                }
                match target.and_then(|b| b.continue_label.clone()) {
                    Some(l) if l.starts_with("'loop") => format!("{ind}continue {l};\n"),
                    Some(l) => format!("{ind}break {l};\n"),
                    None => format!("{ind}{};\n", self.err("continue without loop".into())),
                }
            }
            Stmt::Return(e) => {
                let value = match e {
                    Some(e) => {
                        let ret = self.method_ret.clone();
                        self.expr_as(e, &ret)
                    }
                    None => "()".into(),
                };
                let fin = self.finallies(depth);
                if fin.is_empty() {
                    format!("{ind}return Ok({value});\n")
                } else {
                    format!("{ind}{{\n{ind}    let __ret = {value};\n{fin}{ind}    return Ok(__ret);\n{ind}}}\n")
                }
            }
            Stmt::Throw(e) => format!("{ind}{};\n", self.throw(e)),
            Stmt::Switch { scrutinee, arms } => self.switch(scrutinee, arms, depth),
            Stmt::Try { body, catches, finally } => self.try_stmt(body, catches, finally, depth),
        }
    }

    fn loop_label(&mut self) -> String {
        self.fresh("loop")
    }

    /// A statement used as a body, without an extra block when it already is one.
    fn nested(&mut self, s: &Stmt, depth: usize) -> String {
        match s {
            Stmt::Block(b) => self.block(b, depth),
            other => {
                self.scopes.push(BTreeMap::new());
                let out = self.stmt(other, depth);
                self.scopes.pop();
                out
            }
        }
    }

    fn local_decl(&mut self, ty: &Type, v: &Var, depth: usize) -> String {
        let mut t = T::of(&Type { dims: ty.dims + v.extra_dims, ..ty.clone() });
        let init = match &v.init {
            Some(Expr::ArrayLit(items)) => self.array_lit(&t, items),
            Some(e) => {
                // A raw generic local (`Enumeration e = v.elements()`) takes its initialiser's
                // type arguments.
                if let T::Obj(c, args) = &t
                    && args.is_empty()
                    && self.classes.get(c).is_some_and(|i| !i.type_params.is_empty())
                    && let T::Obj(ic, iargs) = self.type_of(e)
                    && ic == *c
                {
                    t = T::Obj(ic, iargs);
                }
                self.expr_as(e, &t)
            }
            None => t.default_value(),
        };
        self.scopes.last_mut().expect("scope").insert(v.name.clone(), t.clone());
        format!("{}let mut {}: {} = {init};\n", Self::indent(depth), rust_ident(&v.name), t.rust())
    }

    fn array_lit(&mut self, t: &T, items: &[Expr]) -> String {
        let T::Arr(elem) = t else { return self.err(format!("array literal for {t:?}")) };
        let vals: Vec<String> = items
            .iter()
            .map(|i| match i {
                Expr::ArrayLit(inner) => self.array_lit(elem, inner),
                e => self.expr_as(e, elem),
            })
            .collect();
        format!("J::new(vec![{}])", vals.join(", "))
    }

    fn switch(&mut self, scrutinee: &Expr, arms: &[SwitchArm], depth: usize) -> String {
        let ind = Self::indent(depth);
        let label = self.fresh("sw");
        let (s, st) = self.expr(scrutinee);
        let s = if st == T::Char { format!("({s} as i32)") } else { s };
        // A local declared in one arm is in scope in the later arms (one block in Java):
        // declare it before the arms and turn its declaration into an assignment.
        let mut hoisted = String::new();
        self.scopes.push(BTreeMap::new());
        let arms: Vec<SwitchArm> = arms
            .iter()
            .map(|arm| SwitchArm {
                body: arm
                    .body
                    .iter()
                    .flat_map(|st| match st {
                        Stmt::Local { ty, vars } => vars
                            .iter()
                            .filter_map(|v| {
                                let t = T::of(&Type { dims: ty.dims + v.extra_dims, ..ty.clone() });
                                hoisted.push_str(&format!(
                                    "{ind}    let mut {}: {} = {};\n",
                                    rust_ident(&v.name),
                                    t.rust(),
                                    t.default_value()
                                ));
                                self.scopes.last_mut().expect("scope").insert(v.name.clone(), t);
                                v.init
                                    .clone()
                                    .map(|init| Stmt::Expr(Expr::Assign("=", Box::new(Expr::Name(v.name.clone())), Box::new(init))))
                            })
                            .collect::<Vec<_>>(),
                        other => vec![other.clone()],
                    })
                    .collect(),
                ..arm.clone()
            })
            .collect();
        let arms = arms.as_slice();
        let mut out = format!("{ind}{label}: {{\n{hoisted}{ind}    let __s: i32 = {s};\n{ind}    let __k: usize = ");
        let mut default_pos = None;
        for (i, arm) in arms.iter().enumerate() {
            if arm.is_default {
                default_pos = Some(i);
            }
            if arm.labels.is_empty() {
                continue;
            }
            let conds: Vec<String> = arm.labels.iter().map(|l| format!("__s == {}", self.expr_as(l, &T::Int))).collect();
            out.push_str(&format!("if {} {{ {i} }} else ", conds.join(" || ")));
        }
        out.push_str(&match default_pos {
            Some(d) => format!("{{ {d} }};\n"),
            None => format!("{{ break {label}; }};\n"),
        });
        self.breakables.push(Breakable { java_label: None, rust_label: label.clone(), continue_label: None, tries: self.tries.len() });
        for (i, arm) in arms.iter().enumerate() {
            let body = self.block(&arm.body, depth + 2);
            out.push_str(&format!("{ind}    if __k <= {i} {{\n{body}{ind}    }}\n"));
        }
        self.breakables.pop();
        self.scopes.pop();
        out.push_str(&format!("{ind}}}\n"));
        out
    }

    /// `try`: the body is a labeled block yielding `JResult<()>`; matching catches run in a
    /// second labeled block (a throw from a catch breaks out of it); `finally` runs after
    /// both, then an uncaught or rethrown exception propagates. `return` inside either block
    /// runs `finally` itself.
    fn try_stmt(&mut self, body: &[Stmt], catches: &[(Type, String, Vec<Stmt>)], finally: &Option<Vec<Stmt>>, depth: usize) -> String {
        let ind = Self::indent(depth);
        let label = self.fresh("try");
        self.tries.push(TryCx { label: label.clone(), finally: finally.clone() });
        let b = self.block(body, depth + 1);
        self.tries.pop();
        let mut out =
            format!("{ind}let __r: JResult<()> = {label}: {{\n{b}{ind}    #[allow(unreachable_code)]\n{ind}    Ok(())\n{ind}}};\n");
        let catch_label = self.fresh("catch");
        self.tries.push(TryCx { label: catch_label.clone(), finally: finally.clone() });
        let mut arms = String::new();
        for (ty, name, cbody) in catches {
            let class = class_name(&ty.name);
            // A translated exception arrives in its family's variant (narrowed by class); a
            // library one in `JThrow::Extern`, matched by its class chain.
            let (pattern, value) = if let Some(variant) = hier::throw_variant(&class) {
                let narrow = if variant == class { String::new() } else { format!(" if __e.jtest(|o| o.is_{class}())") };
                (format!("Err(JThrow::{variant}(__e)){narrow}"), Some("__e"))
            } else if hier::is_extern_throwable(&class) {
                (format!("Err(__e) if __e.is_a({class:?})"), Some("__e"))
            } else {
                arms.push_str(&format!("{ind}        {},\n", self.err(format!("catch of {class}"))));
                continue;
            };
            self.scopes.push(BTreeMap::new());
            let bind = match value {
                Some(v) => {
                    self.scopes.last_mut().expect("scope").insert(name.clone(), T::obj(&class));
                    format!("{ind}            let mut {}: {} = {v};\n", rust_ident(name), T::obj(&class).rust())
                }
                None => String::new(),
            };
            let cb = self.block(cbody, depth + 3);
            self.scopes.pop();
            arms.push_str(&format!("{ind}        {pattern} => {{\n{bind}{cb}{ind}            #[allow(unreachable_code)]\n{ind}            Ok(())\n{ind}        }}\n"));
        }
        self.tries.pop();
        out.push_str(&format!(
            "{ind}#[allow(unused_labels)]\n{ind}let __r: JResult<()> = {catch_label}: {{\n{ind}    #[allow(unreachable_patterns, unused_variables)]\n{ind}    match __r {{\n{ind}        Ok(()) => Ok(()),\n{arms}{ind}        Err(__other) => Err(__other),\n{ind}    }}\n{ind}}};\n"
        ));
        if let Some(f) = finally {
            out.push_str(&self.block(f, depth));
        }
        let rethrow = self.propagate("__e");
        out.push_str(&format!("{ind}if let Err(__e) = __r {{\n{ind}    {rethrow};\n{ind}}}\n"));
        out
    }

    fn throw(&mut self, e: &Expr) -> String {
        let (v, t) = self.expr(e);
        let err = match t.class() {
            Some(c) if hier::is_extern_throwable(c) => v,
            Some(c) => match hier::throw_variant(c) {
                Some(variant) => format!("JThrow::{variant}({v})"),
                None => return self.err(format!("throw of {t:?}")),
            },
            None => return self.err(format!("throw of {t:?}")),
        };
        let fin = self.finallies(0);
        if self.tries.is_empty() && !fin.is_empty() {
            return format!("{{ {fin} return Err({err}) }}");
        }
        self.propagate(&err)
    }

    // ---- expressions ----

    fn cond(&mut self, e: &Expr) -> String {
        self.expr_as(e, &T::Bool)
    }

    /// An expression statement: assignments and increments need no value.
    fn expr_stmt(&mut self, e: &Expr) -> String {
        match e {
            Expr::Assign(op, lhs, rhs) => self.assign(op, lhs, rhs, false).0,
            Expr::Postfix(op, x) | Expr::Unary(op @ ("++" | "--"), x) => {
                let one = Expr::Int("1".into());
                let op = if *op == "++" { "+=" } else { "-=" };
                self.assign(op, x, &one, false).0
            }
            other => {
                let (v, _) = self.expr(other);
                format!("let _ = {v}")
            }
        }
    }

    /// An expression converted to `target` (widening, `int` -> `char` constants, `null`, and
    /// type arguments of `new C<>()` from the target).
    fn expr_as(&mut self, e: &Expr, target: &T) -> String {
        if let (Expr::New { ty, args }, T::Obj(tc, targs)) = (e, target)
            && ty.args.is_empty()
            && class_name(&ty.name) == *tc
            && !targs.is_empty()
        {
            return self.new_object(ty, args, targs.clone()).0;
        }
        let (v, t) = self.expr(e);
        convert(&v, &t, target)
    }

    pub(crate) fn expr(&mut self, e: &Expr) -> (String, T) {
        match e {
            Expr::Int(s) => int_literal(s),
            Expr::Char(s) => (format!("{}u16", char_literal(s)), T::Char),
            Expr::Str(s) => (format!("JStr::of({:?})", unescape(s)), T::Str),
            Expr::Bool(b) => (b.to_string(), T::Bool),
            Expr::Null => ("JNull::null()".into(), T::Null),
            Expr::This => ("self.clone()".into(), T::obj(&self.class)),
            Expr::Name(n) => self.name(n),
            Expr::Field(target, f) => self.field(target, f),
            Expr::Index(arr, i) => {
                let (a, at) = self.expr(arr);
                let i = self.expr_as(i, &T::Int);
                match at {
                    T::Arr(elem) => (format!("{a}.at({i})"), *elem),
                    other => (self.err(format!("index into {other:?}")), T::Int),
                }
            }
            Expr::Call { target, name, args } => self.call(target.as_deref(), name, args),
            Expr::New { ty, args } => {
                let targs: Vec<T> = ty.args.iter().map(T::of).collect();
                self.new_object(ty, args, targs)
            }
            Expr::NewArray { ty, lens, init } => {
                let t = T::of(ty);
                if let Some(items) = init {
                    return (self.array_lit(&t, items), t);
                }
                if lens.is_empty() {
                    return (self.err("array without length".into()), t);
                }
                // Java evaluates every length before allocating; each given dimension gets
                // its own arrays, the rest are null.
                let mut code = String::from("{ ");
                for (i, len) in lens.iter().enumerate() {
                    let n = self.expr_as(len, &T::Int);
                    code.push_str(&format!("let __d{i}: i32 = {n}; "));
                }
                let mut alloc = String::new();
                let mut levels = Vec::new();
                let mut cur = &t;
                for _ in 0..lens.len() {
                    let T::Arr(elem) = cur else { unreachable!() };
                    levels.push(elem.as_ref());
                    cur = elem;
                }
                for (i, elem) in levels.iter().enumerate().rev() {
                    alloc = if i == lens.len() - 1 {
                        format!("JArr::new_array(__d{i}, {})", elem.default_value())
                    } else {
                        format!("JArr::new_array_with(__d{i}, || {alloc})")
                    };
                }
                code.push_str(&alloc);
                code.push_str(" }");
                (code, t)
            }
            Expr::ArrayLit(_) => (self.err("bare array literal".into()), T::Null),
            Expr::Cast(ty, x) => {
                let t = T::of(ty);
                let (v, xt) = self.expr(x);
                if t.is_numeric() && xt.is_numeric() {
                    (format!("(({v}) as {})", t.rust()), t)
                } else if xt == T::Null {
                    ("JNull::null()".into(), t)
                } else if xt.class() == Some("Object") && t.class() != Some("Object") {
                    (format!("({v}).cast::<{}>()", t.rust()), t)
                } else {
                    // Reference casts are no-ops: one Rust type per tree-node family.
                    (v, t)
                }
            }
            Expr::Unary(op, x) => match *op {
                "!" => (format!("!({})", self.cond(x)), T::Bool),
                "-" => {
                    let (v, xt) = self.expr(x);
                    let t = if xt == T::Char { T::Int } else { xt.clone() };
                    let v = convert(&v, &xt, &t);
                    if t == T::Double { (format!("(-({v}))"), t) } else { (format!("({v}).wrapping_neg()"), t) }
                }
                "+" => self.expr(x),
                "~" => {
                    let (v, t) = self.expr(x);
                    (format!("!({v})"), t)
                }
                _ => self.assign(if *op == "++" { "+=" } else { "-=" }, x, &Expr::Int("1".into()), true),
            },
            Expr::Postfix(op, x) => {
                let (old, t) = self.expr(x);
                let (upd, _) = self.assign(if *op == "++" { "+=" } else { "-=" }, x, &Expr::Int("1".into()), false);
                (format!("{{ let __old = {old}; {upd}; __old }}"), t)
            }
            Expr::Binary(op, a, b) => self.binary(op, a, b),
            Expr::Assign(op, lhs, rhs) => self.assign(op, lhs, rhs, true),
            Expr::InstanceOf(x, ty) => self.instance_of(x, ty),
            Expr::Unsupported(what) => (self.err(format!("{what} (hand-write the member)")), T::Void),
            Expr::Cond(c, a, b) => {
                let c = self.cond(c);
                let at = self.type_of(a);
                let bt = self.type_of(b);
                let t = if at.is_numeric() && bt.is_numeric() {
                    promote(&at, &bt)
                } else if at == T::Null {
                    bt
                } else {
                    at
                };
                let av = self.expr_as(a, &t);
                let bv = self.expr_as(b, &t);
                (format!("(if {c} {{ {av} }} else {{ {bv} }})"), t)
            }
        }
    }

    fn instance_of(&mut self, x: &Expr, ty: &Type) -> (String, T) {
        let c = class_name(&ty.name);
        let (v, xt) = self.expr(x);
        let family = HIER.with(|h| h.borrow().object_type(&c));
        let test = match &family {
            Some(_) => format!("|o| o.is_{c}()"),
            None => "|_| true".into(),
        };
        let code = if xt.class() == Some("Object") {
            let rust = T::obj(&c).rust();
            match &family {
                Some(_) => format!("({v}).instance_of::<{rust}>(|o| o.jtest({test}))"),
                None => format!("({v}).instance_of::<{rust}>(|_| true)"),
            }
        } else if family.is_some() {
            format!("({v}).jtest({test})")
        } else if xt.class() == Some(c.as_str()) {
            format!("!({v}).is_null()")
        } else {
            return (self.err(format!("instanceof {c} on {xt:?}")), T::Bool);
        };
        (code, T::Bool)
    }

    fn name(&mut self, n: &str) -> (String, T) {
        if let Some(t) = self.local(n).cloned() {
            let v = if t.is_ref() { format!("{}.clone()", rust_ident(n)) } else { rust_ident(n) };
            return (v, t);
        }
        if let Some((owner, t, is_static)) = self.field_of(&self.class.clone(), n) {
            if !is_static && self.is_static {
                return (self.err(format!("instance field {n} in static context")), t);
            }
            return (self.read_field(&owner, n, &t, is_static, None), t);
        }
        // A field of an enclosing class.
        for (recv, outer) in self.enclosing() {
            if let Some((owner, t, is_static)) = self.field_of(&outer, n) {
                if is_static {
                    return (self.read_field(&owner, n, &t, true, None), t);
                }
                if let Some(r) = recv {
                    return (self.read_field(&owner, n, &t, false, Some(format!("({r})"))), t);
                }
            }
        }
        (self.err(format!("unknown name {n}")), T::Int)
    }

    fn field(&mut self, target: &Expr, f: &str) -> (String, T) {
        if f == "class"
            && let Some(c) = self.class_ref(target)
        {
            return (format!("JClass({c:?})"), T::obj("Class"));
        }
        if f == "this"
            && let Expr::Name(outer) = target
        {
            return match self.outer_instance(outer) {
                Some(r) => (r, T::obj(outer)),
                None => (self.err(format!("{outer}.this unreachable")), T::Void),
            };
        }
        // `Class.FIELD` (static) when `target` names a class rather than a value.
        if let Some(class) = self.class_ref(target) {
            if let Some((owner, t, true)) = self.field_of(&class, f) {
                return (self.read_field(&owner, f, &t, true, None), t);
            }
            return (self.err(format!("unknown static {class}.{f}")), T::Int);
        }
        let (recv, rt) = self.expr(target);
        match &rt {
            T::Arr(_) if f == "length" => (format!("{recv}.length()"), T::Int),
            T::Obj(class, args) => match self.field_of(class, f) {
                Some((owner, t, is_static)) => {
                    let t = t.subst(args);
                    (self.read_field(&owner, f, &t, is_static, Some(format!("({recv})"))), t)
                }
                None => (self.err(format!("unknown field {class}.{f}")), T::Int),
            },
            other => (self.err(format!("field {f} of {other:?}")), T::Int),
        }
    }

    /// Arguments for `sig`, packing trailing ones into an array for a varargs call.
    fn args_for_sig(&mut self, args: &[Expr], sig: &MethodSig) -> String {
        let n = sig.params.len();
        if sig.varargs {
            let direct = args.len() == n && {
                let t = self.type_of(&args[n - 1]);
                assignable(&t, &sig.params[n - 1]).is_some()
            };
            if !direct {
                let T::Arr(elem) = &sig.params[n - 1] else { unreachable!("varargs parameter is an array") };
                let fixed = self.args_for(&args[..n - 1], &sig.params[..n - 1]);
                let rest: Vec<String> = args[n - 1..].iter().map(|a| self.expr_as(a, elem)).collect();
                let packed = format!("J::new(vec![{}])", rest.join(", "));
                return if fixed.is_empty() { packed } else { format!("{fixed}, {packed}") };
            }
        }
        self.args_for(args, &sig.params)
    }

    fn args_for(&mut self, args: &[Expr], params: &[T]) -> String {
        args.iter()
            .zip(params)
            .map(|(a, p)| {
                if *p == T::Any {
                    let (v, t) = self.expr(a);
                    format!("&{}", to_string_arg(&v, &t))
                } else {
                    self.expr_as(a, p)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A resolved call; `recv` is `None` for static calls.
    fn emit_call(&mut self, owner: &str, sig: &MethodSig, recv: Option<String>, args: &[Expr]) -> (String, T) {
        let a = self.args_for_sig(args, sig);
        // An erased library generic returns elements as the type the caller expects.
        let turbofish = if ERASED_LIBRARY_GENERICS.contains(&owner) && sig.returns_param {
            match &sig.ret {
                T::Obj(c, targs) if matches!(c.as_str(), "Iterator" | "Enumeration") => {
                    format!("::<{}>", targs.first().cloned().unwrap_or_else(|| T::obj("Object")).rust())
                }
                ret => format!("::<{}>", ret.rust()),
            }
        } else {
            String::new()
        };
        let call = match recv {
            Some(r) if !sig.is_static => format!("{r}.{}{turbofish}({a})", sig.rust_name),
            _ => format!("{owner}::{}{turbofish}({a})", sig.rust_name),
        };
        (if sig.fallible { self.tri(call) } else { call }, sig.ret.clone())
    }

    fn call(&mut self, target: Option<&Expr>, name: &str, args: &[Expr]) -> (String, T) {
        let Some(target) = target else {
            if name == "this" || name == "super" {
                return self.ctor_call(name, args);
            }
            let class = self.class.clone();
            if name == "getClass" && args.is_empty() && !self.is_static {
                return (get_class("self", &T::obj(&class)), T::obj("Class"));
            }
            if let Some((owner, sig)) = self.resolve(&class, name, args, &[]) {
                if !sig.is_static && self.is_static {
                    return (self.err(format!("instance method {name} in static context")), sig.ret);
                }
                return self.emit_call(&owner, &sig, Some("self".into()), args);
            }
            // A method of an enclosing class.
            for (recv, outer) in self.enclosing() {
                if let Some((owner, sig)) = self.resolve(&outer, name, args, &[]) {
                    if sig.is_static {
                        return self.emit_call(&owner, &sig, None, args);
                    }
                    if let Some(r) = recv {
                        return self.emit_call(&owner, &sig, Some(format!("({r})")), args);
                    }
                }
            }
            if name == "getMessage" && args.is_empty() && self.field_of(&class, "detailMessage").is_some() {
                let (v, _) = self.name("detailMessage");
                return (v, T::Str);
            }
            return (self.err(format!("unknown method {name}/{}", args.len())), T::Void);
        };
        if let Expr::Name(s) = target
            && s == "super"
        {
            return self.super_call(name, args);
        }
        // `Class.method(...)`.
        if let Some(class) = self.class_ref(target) {
            if class == "String" {
                return self.string_static(name, args);
            }
            return match self.resolve(&class, name, args, &[]) {
                Some((owner, sig)) => self.emit_call(&owner, &sig, None, args),
                None => (self.err(format!("unknown static method {class}.{name}/{}", args.len())), T::Void),
            };
        }
        let (recv, rt) = self.expr(target);
        if name == "getClass" && args.is_empty() {
            return (get_class(&recv, &rt), T::obj("Class"));
        }
        match &rt {
            T::Str => self.string_method(&recv, name, args),
            // `Integer.intValue()` and friends on the primitives that stand for the boxes.
            T::Int | T::Long | T::Bool | T::Char if matches!(name, "intValue" | "longValue" | "booleanValue" | "charValue") => (recv, rt),
            T::Obj(class, targs) => match self.resolve(class, name, args, targs) {
                Some((owner, sig)) => self.emit_call(&owner, &sig, Some(format!("({recv})")), args),
                // `Object`'s methods on a class that does not declare them.
                None => match (name, args) {
                    ("toString", []) => (format!("JStr::of(&({recv}).java_string())"), T::Str),
                    ("hashCode", []) => (format!("({recv}).java_hash()"), T::Int),
                    // `Throwable.getMessage()` of a translated exception: its `detailMessage`.
                    ("getMessage", []) if self.field_of(class, "detailMessage").is_some() => {
                        let (owner, t, is_static) = self.field_of(class, "detailMessage").expect("throwable");
                        (self.read_field(&owner, "detailMessage", &t, is_static, Some(format!("({recv})"))), T::Str)
                    }
                    ("equals", [o]) => {
                        let ot = rt.clone();
                        let o = self.expr_as(o, &ot);
                        (format!("({recv}).java_equals(&{o})"), T::Bool)
                    }
                    _ => (self.err(format!("unknown method {class}.{name}/{}", args.len())), T::Void),
                },
            },
            other => (self.err(format!("method {name} on {other:?}")), T::Void),
        }
    }

    /// `super.m(...)`: the implementation the superclass would run, called directly.
    fn super_call(&mut self, name: &str, args: &[Expr]) -> (String, T) {
        let class = self.class.clone();
        let parent = HIER.with(|h| h.borrow().places.get(&class).and_then(|p| p.parent.clone()));
        let Some(p) = parent else {
            if name == "getMessage" && args.is_empty() && self.field_of(&class, "detailMessage").is_some() {
                let (v, _) = self.name("detailMessage");
                return (v, T::Str);
            }
            return (self.err(format!("super.{name}/{} of a library class", args.len())), T::Void);
        };
        let Some((_, sig)) = self.resolve(&p, name, args, &[]) else {
            return (self.err(format!("unknown method super.{name}/{}", args.len())), T::Void);
        };
        let Some(imp) = SLOTS.with(|s| s.borrow().implementation(&p, &sig.rust_name)) else {
            return (self.err(format!("super.{name} is abstract")), T::Void);
        };
        let a = self.args_for_sig(args, &sig);
        (self.tri(format!("self.{}__{imp}({a})", sig.rust_name)), sig.ret)
    }

    /// `this(...)` / `super(...)` as the first statement of a constructor. `super(...)` is
    /// followed by this class's field initialisers, as in Java.
    fn ctor_call(&mut self, name: &str, args: &[Expr]) -> (String, T) {
        if !self.in_ctor {
            return (self.err(format!("{name}(...) outside a constructor")), T::Void);
        }
        let class = self.class.clone();
        if name == "this" {
            return match self.resolve(&class, "<init>", args, &[]) {
                Some((_, sig)) => {
                    let a = self.args_for_sig(args, &sig);
                    (self.tri(format!("self.init__{class}_{}({a})", sig.rust_name)), T::Void)
                }
                None => (self.err(format!("unknown constructor this/{}", args.len())), T::Void),
            };
        }
        let fields = self.tri(format!("self.init_fields__{class}()"));
        let parent = HIER.with(|h| h.borrow().places.get(&class).and_then(|p| p.parent.clone()));
        if let Some(p) = parent {
            return match self.resolve(&p, "<init>", args, &[]) {
                Some((_, sig)) => {
                    let a = self.args_for_sig(args, &sig);
                    let sup = self.tri(format!("self.init__{p}_{}({a})", sig.rust_name));
                    (format!("{{ {sup}; {fields}; }}"), T::Void)
                }
                None => (self.err(format!("unknown constructor super/{}", args.len())), T::Void),
            };
        }
        let throwable = self.field_of(&class, "detailMessage").is_some();
        match args {
            [] => (format!("{{ {fields}; }}"), T::Void),
            [msg] if throwable => {
                let m = self.expr_as(msg, &T::Str);
                let set = field_write("self", &self.field_of(&class, "detailMessage").expect("throwable").0, "detailMessage", "__v");
                (format!("{{ let __v = {m}; {set}; {fields}; }}"), T::Void)
            }
            _ => (self.err(format!("super/{}", args.len())), T::Void),
        }
    }

    fn string_method(&mut self, recv: &str, name: &str, args: &[Expr]) -> (String, T) {
        let arg_types: Vec<T> = args.iter().map(|a| self.type_of(a)).collect();
        let str_arg = |this: &mut Self, i: usize| this.expr_as(&args[i], &T::Str);
        let int_arg = |this: &mut Self, i: usize| this.expr_as(&args[i], &T::Int);
        match (name, arg_types.as_slice()) {
            // `String.equals(Object)`: false for anything but an equal string.
            ("equals", [T::Obj(c, _)]) if c == "Object" => {
                let o = self.expr(&args[0]).0;
                (format!("{recv}.equals_object(&{o})"), T::Bool)
            }
            ("equals", [_]) => (format!("{recv}.equals(&{})", str_arg(self, 0)), T::Bool),
            ("length", []) => (format!("{recv}.length()"), T::Int),
            ("isEmpty", []) => (format!("{recv}.is_empty()"), T::Bool),
            ("charAt", [_]) => (format!("{recv}.char_at({})", int_arg(self, 0)), T::Char),
            ("substring", [_]) => (format!("{recv}.substring_from({})", int_arg(self, 0)), T::Str),
            ("substring", [_, _]) => {
                let i = int_arg(self, 0);
                let j = int_arg(self, 1);
                (format!("{recv}.substring({i}, {j})"), T::Str)
            }
            ("indexOf", [T::Char]) => (format!("{recv}.index_of_char({})", self.expr_as(&args[0], &T::Char)), T::Int),
            ("indexOf", [T::Str]) => (format!("{recv}.index_of(&{})", str_arg(self, 0)), T::Int),
            ("indexOf", [T::Str, _]) => {
                let a = str_arg(self, 0);
                let from = int_arg(self, 1);
                (format!("{recv}.index_of_from(&{a}, {from})"), T::Int)
            }
            ("compareTo", [T::Str]) => (format!("{recv}.compare_to(&{})", str_arg(self, 0)), T::Int),
            ("lastIndexOf", [T::Str]) => (format!("{recv}.last_index_of(&{})", str_arg(self, 0)), T::Int),
            ("contains", [T::Str]) => (format!("{recv}.contains(&{})", str_arg(self, 0)), T::Bool),
            ("startsWith", [_]) => (format!("{recv}.starts_with(&{})", str_arg(self, 0)), T::Bool),
            ("startsWith", [_, _]) => {
                let a = str_arg(self, 0);
                let at = int_arg(self, 1);
                (format!("{recv}.starts_with_at(&{a}, {at})"), T::Bool)
            }
            ("endsWith", [_]) => (format!("{recv}.ends_with(&{})", str_arg(self, 0)), T::Bool),
            ("concat", [_]) => (format!("concat(&{recv}, &{})", str_arg(self, 0)), T::Str),
            ("trim", []) => (format!("{recv}.trim()"), T::Str),
            ("toLowerCase", []) => (format!("{recv}.to_lower_case()"), T::Str),
            // `replace(CharSequence, CharSequence)` replaces literally (no regex).
            ("replace", [T::Str, T::Str]) => {
                let a = str_arg(self, 0);
                let b = str_arg(self, 1);
                (format!("{recv}.replace(&{a}, &{b})"), T::Str)
            }
            // Only the regexes the sources use, each by a hand-written splitter.
            ("split", [_]) => match &args[0] {
                Expr::Str(re) if unescape(re) == "\\r?\\n" => (format!("{recv}.split_lines()"), T::Arr(Box::new(T::Str))),
                // An alternation of literal strings (`" |,|\\)"`).
                Expr::Str(re) if literal_alternatives(&unescape(re)).is_some() => {
                    let alts = literal_alternatives(&unescape(re)).expect("literal");
                    let list: Vec<String> = alts.iter().map(|a| format!("{a:?}")).collect();
                    (format!("{recv}.split_literals(&[{}])", list.join(", ")), T::Arr(Box::new(T::Str)))
                }
                other => (self.err(format!("String.split with regex {other:?}")), T::Void),
            },
            ("toString" | "intern", []) => (recv.to_string(), T::Str),
            _ => (self.err(format!("String.{name}/{}", args.len())), T::Void),
        }
    }

    fn string_static(&mut self, name: &str, args: &[Expr]) -> (String, T) {
        match (name, args) {
            // `String.format(fmt, args...)`: the conversions the sources use (`%s`, `%d`).
            // `String.format(fmt, objects)` with an `Object[]` of arguments.
            ("format", [fmt, arr]) if matches!(self.type_of(arr), T::Arr(_)) => {
                let f = self.expr_as(fmt, &T::Str);
                let a = self.expr(arr).0;
                (format!("java_format_objects(&{f}, &{a})"), T::Str)
            }
            ("format", [fmt, rest @ ..]) => {
                let f = self.expr_as(fmt, &T::Str);
                let parts: Vec<String> = rest
                    .iter()
                    .map(|a| {
                        let (v, t) = self.expr(a);
                        format!("&{}", to_string_arg(&v, &t))
                    })
                    .collect();
                (format!("java_format(&{f}, &[{}])", parts.join(", ")), T::Str)
            }
            ("valueOf", [a]) => {
                let (v, t) = self.expr(a);
                (format!("JStr::of(&{}.java_string())", to_string_arg(&v, &t)), T::Str)
            }
            _ => (self.err(format!("String.{name}/{}", args.len())), T::Void),
        }
    }

    fn new_object(&mut self, ty: &Type, args: &[Expr], targs: Vec<T>) -> (String, T) {
        let class = class_name(&ty.name);
        if class == "String" {
            let types: Vec<T> = args.iter().map(|a| self.type_of(a)).collect();
            return match types.as_slice() {
                [T::Str] => (self.expr(&args[0]).0, T::Str),
                [T::Arr(e), _, _] if **e == T::Char => {
                    let a = self.expr(&args[0]).0;
                    let off = self.expr_as(&args[1], &T::Int);
                    let len = self.expr_as(&args[2], &T::Int);
                    (format!("JStr::from_chars(&{a}, {off}, {len})"), T::Str)
                }
                _ => (self.err(format!("new String/{}", args.len())), T::Str),
            };
        }
        if hier::is_extern_throwable(&class) {
            let msg = match args {
                [] => "JStr::null()".to_string(),
                [m] => {
                    let (v, t) = self.expr(m);
                    match t {
                        T::Str => v,
                        // `new Error(cause)`: the cause's message.
                        _ => format!("JStr::of(&{}.java_string())", to_string_arg(&v, &t)),
                    }
                }
                _ => return (self.err(format!("new {class}/{}", args.len())), T::obj(&class)),
            };
            return (format!("JThrow::new_extern({class:?}, {msg})"), T::obj(&class));
        }
        let t = T::Obj(class.clone(), targs.clone());
        // Constructors are methods named `<init>` in the class table.
        let inner_outer = HIER.with(|h| h.borrow().places.get(&class).filter(|p| p.is_inner).and_then(|p| p.outer.clone()));
        match self.resolve(&class, "<init>", args, &targs) {
            Some((_, sig)) => {
                let mut a = self.args_for_sig(args, &sig);
                if let Some(o) = inner_outer {
                    let Some(r) = self.outer_instance(&o) else {
                        return (self.err(format!("new {class}: no enclosing {o} instance")), t);
                    };
                    a = if a.is_empty() { r } else { format!("{r}, {a}") };
                }
                let call = format!("{class}::new_{}({a})", sig.rust_name);
                (if sig.fallible { self.tri(call) } else { call }, t)
            }
            None => (self.err(format!("unknown constructor {class}/{}", args.len())), t),
        }
    }

    fn binary(&mut self, op: &str, a: &Expr, b: &Expr) -> (String, T) {
        let (av, at) = self.expr(a);
        let (bv, bt) = self.expr(b);
        // Unboxing in arithmetic and comparisons (not `box == null` or `box == box`).
        let is_integer = |t: &T| t.class() == Some("Integer");
        let compare_refs = matches!(op, "==" | "!=") && (at == T::Null || bt == T::Null || (is_integer(&at) && is_integer(&bt)));
        let ((av, at), (bv, bt)) = if !compare_refs && op != "+" || (op == "+" && at != T::Str && bt != T::Str) {
            let unbox = |v: String, t: T| if is_integer(&t) { (format!("({v}).intValue()"), T::Int) } else { (v, t) };
            (unbox(av, at), unbox(bv, bt))
        } else {
            ((av, at), (bv, bt))
        };
        match op {
            "&&" | "||" => (format!("({} {op} {})", convert(&av, &at, &T::Bool), convert(&bv, &bt, &T::Bool)), T::Bool),
            "+" if at == T::Str || bt == T::Str => (format!("concat(&{}, &{})", to_string_arg(&av, &at), to_string_arg(&bv, &bt)), T::Str),
            "==" | "!=" => {
                let neg = if op == "!=" { "!" } else { "" };
                if at.is_numeric() && bt.is_numeric() {
                    let t = promote(&at, &bt);
                    (format!("({} {op} {})", convert(&av, &at, &t), convert(&bv, &bt, &t)), T::Bool)
                } else if at == T::Bool {
                    (format!("({av} {op} {bv})"), T::Bool)
                } else if bt == T::Null {
                    (format!("{neg}({av}).is_null()"), T::Bool)
                } else if at == T::Null {
                    (format!("{neg}({bv}).is_null()"), T::Bool)
                } else if (at.class() == Some("Object")) != (bt.class() == Some("Object")) {
                    // Identity across a box: the `Object` side holds the other side's type.
                    let (o, x, xt) = if at.class() == Some("Object") { (&av, &bv, &bt) } else { (&bv, &av, &at) };
                    (format!("{neg}{{ let __x = {x}; ({o}).instance_of::<{}>(|v| v.same(&__x)) }}", xt.rust()), T::Bool)
                } else {
                    (format!("{neg}({av}).same(&{bv})"), T::Bool)
                }
            }
            "<" | ">" | "<=" | ">=" => {
                let t = promote(&at, &bt);
                (format!("({} {op} {})", convert(&av, &at, &t), convert(&bv, &bt, &t)), T::Bool)
            }
            "&" | "|" | "^" if at == T::Bool => (format!("({av} {op} {bv})"), T::Bool),
            "<<" | ">>" | ">>>" => {
                let t = if at == T::Long { T::Long } else { T::Int };
                let x = convert(&av, &at, &t);
                let n = convert(&bv, &bt, &T::Int);
                let v = match (op, &t) {
                    ("<<", _) => format!("({x}).wrapping_shl(({n}) as u32)"),
                    (">>", _) => format!("({x}).wrapping_shr(({n}) as u32)"),
                    (_, T::Long) => format!("ushr_i64({x}, {n})"),
                    _ => format!("ushr_i32({x}, {n})"),
                };
                (v, t)
            }
            _ => {
                let t = promote(&at, &bt);
                let x = convert(&av, &at, &t);
                let y = convert(&bv, &bt, &t);
                let v = match (op, &t) {
                    (_, T::Double) => format!("({x} {op} {y})"),
                    ("+", _) => format!("({x}).wrapping_add({y})"),
                    ("-", _) => format!("({x}).wrapping_sub({y})"),
                    ("*", _) => format!("({x}).wrapping_mul({y})"),
                    ("/", _) => format!("({x}).wrapping_div({y})"),
                    ("%", _) => format!("({x}).wrapping_rem({y})"),
                    _ => format!("({x} {op} {y})"),
                };
                (v, t)
            }
        }
    }

    /// `lhs op= rhs`. Returns the Rust code and, when `want_value`, an expression whose value
    /// is the assigned value (Java's assignment expression).
    fn assign(&mut self, op: &str, lhs: &Expr, rhs: &Expr, want_value: bool) -> (String, T) {
        let lt = self.type_of(lhs);
        let value = if op == "=" {
            match rhs {
                Expr::ArrayLit(items) => self.array_lit(&lt, items),
                _ => self.expr_as(rhs, &lt),
            }
        } else {
            let bin = op.trim_end_matches('=');
            let (v, vt) = self.binary(bin, lhs, rhs);
            // Compound assignment narrows back to the target type.
            if vt != lt && lt.is_numeric() { format!("(({v}) as {})", lt.rust()) } else { v }
        };
        let write = |this: &mut Self, v: &str| -> String {
            match lhs {
                Expr::Name(n) if this.local(n).is_some() => format!("{} = {v}", rust_ident(n)),
                Expr::Name(n) => match this.field_of(&this.class.clone(), n) {
                    Some((owner, _, true)) => format!("{owner}::statics().jget_mut().{} = {v}", rust_ident(n)),
                    Some((owner, _, false)) => field_write("self", &owner, n, v),
                    None => {
                        for (recv, outer) in this.enclosing() {
                            match this.field_of(&outer, n) {
                                Some((owner, _, true)) => return format!("{owner}::statics().jget_mut().{} = {v}", rust_ident(n)),
                                Some((owner, _, false)) if recv.is_some() => {
                                    return field_write(&format!("({})", recv.expect("instance")), &owner, n, v);
                                }
                                _ => {}
                            }
                        }
                        this.err(format!("assignment to unknown {n}"))
                    }
                },
                Expr::Field(target, f) => {
                    if let Some(class) = this.class_ref(target) {
                        return format!("{class}::statics().jget_mut().{} = {v}", rust_ident(f));
                    }
                    let (recv, rt) = this.expr(target);
                    let owner = rt.class().and_then(|c| this.field_of(c, f)).map(|(o, ..)| o).unwrap_or_default();
                    field_write(&format!("({recv})"), &owner, f, v)
                }
                Expr::Index(arr, i) => {
                    let (a, _) = this.expr(arr);
                    let i = this.expr_as(i, &T::Int);
                    format!("({a}).set_at({i}, {v})")
                }
                other => this.err(format!("assignment to {other:?}")),
            }
        };
        if want_value {
            let w = write(self, "__v.clone()");
            (format!("{{ let __v: {} = {value}; {w}; __v }}", lt.rust()), lt)
        } else {
            let w = write(self, "__v");
            (format!("{{ let __v: {} = {value}; {w}; }}", lt.rust()), lt)
        }
    }
}

thread_local! {
    /// `(class, field)` pairs emitted as Rust `const`s.
    pub(crate) static CONSTS: std::cell::RefCell<BTreeSet<(String, String)>> = Default::default();
}

/// `recv.getClass()`: the runtime class, by simple name.
fn get_class(recv: &str, t: &T) -> String {
    let c = t.class().unwrap_or_default();
    // An `Object`'s class is whatever it holds.
    if c == "Object" {
        return format!("JClass(({recv}).java_class())");
    }
    if HIER.with(|h| h.borrow().object_type(c).is_some()) {
        format!("JClass(({recv}).jread(|o| o.jclass_name()))")
    } else {
        format!("JClass({c:?})")
    }
}

/// `recv.f = v` for an instance field `f` declared in `owner`.
fn field_write(recv: &str, owner: &str, f: &str, v: &str) -> String {
    if HIER.with(|h| h.borrow().in_family(owner)) {
        format!("*{recv}.jget_mut().{owner}_{f}_mut() = {v}")
    } else {
        format!("{recv}.jget_mut().{} = {v}", rust_ident(f))
    }
}

/// Implicit conversions on assignment and argument passing.
fn convert(v: &str, from: &T, to: &T) -> String {
    let is_integer = |t: &T| t.class() == Some("Integer");
    match (from, to) {
        (a, b) if a == b => v.to_string(),
        // Boxing and unboxing (`null` unboxed is `NullPointerException`).
        (T::Int | T::Char, t) if is_integer(t) => format!("JInteger::of(({v}) as i32)"),
        (f, T::Int) if is_integer(f) => format!("({v}).intValue()"),
        (f, T::Long | T::Double) if is_integer(f) => format!("(({v}).intValue() as {})", to.rust()),
        (T::Null, _) => "JNull::null()".into(),
        (T::Char | T::Int, T::Long) | (T::Char, T::Int) | (T::Int | T::Long | T::Char, T::Double) => format!("(({v}) as {})", to.rust()),
        // Integer constants assigned to `char`/`int` narrowing (e.g. `char c = 65`).
        (T::Int, T::Char) => format!("(({v}) as u16)"),
        // Upcast to `Object`: box the value; the cast back is a checked downcast.
        (_, T::Obj(o, _)) if o == "Object" => format!("Object::of({v})"),
        _ => v.to_string(),
    }
}

/// A string-concatenation operand.
fn to_string_arg(v: &str, t: &T) -> String {
    match t {
        T::Char => format!("JChar({v})"),
        T::Null => "JStr::null()".into(),
        _ => v.to_string(),
    }
}

/// Java integer literal -> Rust literal with Java's value and type.
fn int_literal(s: &str) -> (String, T) {
    let is_long = s.ends_with('L') || s.ends_with('l');
    let digits = s.trim_end_matches(['L', 'l']);
    let value: u64 = if let Some(h) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        u64::from_str_radix(h, 16).unwrap_or(0)
    } else if digits.len() > 1 && digits.starts_with('0') {
        u64::from_str_radix(&digits[1..], 8).unwrap_or(0)
    } else {
        digits.parse().unwrap_or(0)
    };
    if is_long {
        let v = value as i64;
        (if v < 0 { format!("({v}i64)") } else { format!("{v}i64") }, T::Long)
    } else {
        let v = value as u32 as i32;
        (if v < 0 { format!("({v}i32)") } else { format!("{v}i32") }, T::Int)
    }
}

/// The UTF-16 unit of a Java char literal body (`a`, `\n`, `A`, `\\`).
fn char_literal(body: &str) -> u16 {
    let s = unescape(body);
    s.encode_utf16().next().unwrap_or(0)
}

/// Java string/char escapes.
pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            // OctalEscape: up to three digits, three only when the first is 0-3
            // (JavaCC writes literal images such as `"\133"`).
            Some(d @ '0'..='7') => {
                let mut v = d.to_digit(8).expect("octal");
                let max = if d <= '3' { 2 } else { 1 };
                for _ in 0..max {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(n) => {
                            v = v * 8 + n;
                            chars.next();
                        }
                        None => break,
                    }
                }
                out.push(char::from_u32(v).expect("octal escape is at most \\377"));
            }
            Some('u') => {
                while chars.peek() == Some(&'u') {
                    chars.next();
                }
                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap_or(0)).unwrap_or('\u{fffd}'));
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Java identifiers that are Rust keywords.
pub(crate) fn rust_ident(n: &str) -> String {
    match n {
        // Keywords that cannot be raw identifiers.
        "self" | "Self" | "crate" | "super" => format!("{n}_"),
        "type" | "match" | "fn" | "mod" | "use" | "ref" | "move" | "loop" | "impl" | "trait" | "where" | "in" | "as" | "mut" | "let"
        | "unsafe" | "dyn" | "async" | "await" | "box" | "yield" | "macro" | "pub" | "struct" | "enum" | "extern" | "priv" | "override"
        | "typeof" | "virtual" | "final" | "abstract" | "become" | "do" | "unsized" | "try" | "gen" => format!("r#{n}"),
        other => other.to_string(),
    }
}

/// Runs the static initialisers of `class` and its superclasses (superclass first), as
/// Java initialises a class before its first instance or static call.
fn class_init(class: &str) -> String {
    let chain = HIER.with(|h| h.borrow().ancestors(class));
    let with_statics: Vec<String> = chain.into_iter().rev().filter(|c| CLASSES_WITH_STATICS.with(|s| s.borrow().contains(c))).collect();
    with_statics.iter().map(|c| format!("        {c}::statics();\n")).collect()
}

thread_local! {
    /// Classes that have a static initialiser.
    pub(crate) static CLASSES_WITH_STATICS: std::cell::RefCell<BTreeSet<String>> = Default::default();
    /// Virtual-call slots of each family.
    pub(crate) static SLOTS: std::cell::RefCell<Slots> = Default::default();
}

/// One overridable method signature of a family: its Rust name is the dispatcher's.
#[derive(Clone)]
pub(crate) struct Slot {
    pub(crate) rust_name: String,
    pub(crate) java_name: String,
    /// The topmost classes or interfaces declaring it.
    pub(crate) origins: Vec<String>,
    pub(crate) params: Vec<T>,
    pub(crate) ret: T,
    /// Classes that implement it (have a body).
    pub(crate) impls: BTreeSet<String>,
}

#[derive(Default)]
pub(crate) struct Slots {
    /// Root -> slots in declaration order.
    pub(crate) by_root: BTreeMap<String, Vec<Slot>>,
}

impl Slots {
    /// The class whose body runs for `slot` on an object of class `class`.
    pub(crate) fn implementation(&self, class: &str, slot: &str) -> Option<String> {
        let (root, ancestors) = HIER.with(|h| {
            let h = h.borrow();
            (h.root_of(class), h.ancestors(class))
        });
        let s = self
            .by_root
            .get(&root)?
            .iter()
            .find(|s| s.rust_name == slot && s.origins.iter().any(|o| HIER.with(|h| h.borrow().implements(class, o))))?;
        ancestors.into_iter().find(|a| s.impls.contains(a))
    }
}

/// Names the instance methods of every family by signature, so an override and the method it
/// overrides share one dispatcher, and records which classes implement each.
pub(crate) fn assign_slots(classes: &mut Classes) {
    let families: Vec<(String, Vec<String>)> = HIER.with(|h| {
        let h = h.borrow();
        h.families
            .iter()
            .map(|(r, m)| {
                let mut all = m.clone();
                all.extend(h.iface_root.iter().filter(|(_, ir)| *ir == r).map(|(i, _)| i.clone()));
                (r.clone(), all)
            })
            .collect()
    });
    let mut by_root = BTreeMap::new();
    let mut all_assigned = Vec::new();
    for (root, members) in families {
        // A method's origin: the topmost superclass or interface declaring the same
        // signature. Methods that only share a signature (`DecimalNode.bigVal()` and
        // `NumeralNode.bigVal()`) are different slots.
        let declares = |c: &str, name: &str, params: &[T]| {
            classes.get(c).and_then(|i| i.methods.get(name)).is_some_and(|sigs| sigs.iter().any(|s| !s.is_static && s.params == params))
        };
        let origin = |c: &str, name: &str, params: &[T]| -> String {
            let (ancestors, places) = HIER.with(|h| {
                let h = h.borrow();
                (h.ancestors(c), h.places.clone())
            });
            let mut best = c.to_string();
            for a in &ancestors {
                if declares(a, name, params) {
                    best = a.clone();
                }
            }
            // An interface of the class or its ancestors that declares it wins.
            let mut todo: Vec<String> =
                ancestors.iter().flat_map(|a| places.get(a).map(|p| p.interfaces.clone()).unwrap_or_default()).collect();
            let mut seen = BTreeSet::new();
            while let Some(i) = todo.pop() {
                if !seen.insert(i.clone()) {
                    continue;
                }
                if declares(&i, name, params) {
                    best = i.clone();
                }
                todo.extend(places.get(&i).map(|p| p.interfaces.clone()).unwrap_or_default());
            }
            best
        };
        let mut keys: Vec<(String, String, Vec<T>, T)> = Vec::new();
        for m in &members {
            for (name, sigs) in &classes[m].methods {
                for sig in sigs.iter().filter(|s| !s.is_static && name != "<init>") {
                    let o = origin(m, name, &sig.params);
                    if !keys.iter().any(|(ko, n, p, _)| *ko == o && n == name && *p == sig.params) {
                        keys.push((o, name.clone(), sig.params.clone(), sig.ret.clone()));
                    }
                }
            }
        }
        // Origins some class reaches both of are one slot: that class dispatches the same way
        // through either static type.
        let mut group: Vec<usize> = (0..keys.len()).collect();
        fn find(g: &mut [usize], i: usize) -> usize {
            if g[i] != i {
                let r = find(g, g[i]);
                g[i] = r;
            }
            g[i]
        }
        for i in 0..keys.len() {
            for j in i + 1..keys.len() {
                let (a, b) = (&keys[i], &keys[j]);
                if a.1 == b.1 && a.2 == b.2 && a.0 != b.0 {
                    let shared = members.iter().any(|k| {
                        HIER.with(|h| {
                            let h = h.borrow();
                            h.implements(k, &a.0) && h.implements(k, &b.0)
                        })
                    });
                    if shared {
                        let (ri, rj) = (find(&mut group, i), find(&mut group, j));
                        group[rj] = ri;
                    }
                }
            }
        }
        let reps: Vec<usize> = (0..keys.len()).filter(|&i| find(&mut group.clone(), i) == i).collect();
        let mut slots: Vec<Slot> = Vec::new();
        for &r in &reps {
            let (_, name, params, ret) = &keys[r];
            let origins: Vec<String> = (0..keys.len()).filter(|&i| find(&mut group.clone(), i) == r).map(|i| keys[i].0.clone()).collect();
            let same: Vec<usize> = reps.iter().copied().filter(|&q| keys[q].1 == *name).collect();
            let arity_same: Vec<usize> = same.iter().copied().filter(|&q| keys[q].2.len() == params.len()).collect();
            let rust_name = if same.len() == 1 {
                rust_ident(name)
            } else if arity_same.len() == 1 {
                format!("{name}_{}", params.len())
            } else {
                let k = arity_same.iter().position(|&q| q == r).expect("self");
                format!("{name}_{}_{k}", params.len())
            };
            slots.push(Slot {
                rust_name,
                java_name: name.clone(),
                origins,
                params: params.clone(),
                ret: ret.clone(),
                impls: BTreeSet::new(),
            });
        }
        let mut assigned = Vec::new();
        for m in &members {
            for (name, sigs) in &classes[m].methods {
                for (i, sig) in sigs.iter().enumerate().filter(|(_, s)| !s.is_static && name != "<init>") {
                    let o = origin(m, name, &sig.params);
                    let slot =
                        slots.iter_mut().find(|s| s.origins.contains(&o) && s.java_name == *name && s.params == sig.params).expect("slot");
                    assigned.push((m.clone(), name.clone(), i, slot.rust_name.clone()));
                    if sig.has_body && sig.member.is_some() {
                        slot.impls.insert(m.clone());
                    }
                }
            }
        }
        all_assigned.extend(assigned);
        by_root.insert(root, slots);
    }
    for (m, name, i, rust_name) in all_assigned {
        classes.get_mut(&m).expect("member").methods.get_mut(&name).expect("method")[i].rust_name = rust_name;
    }
    SLOTS.with(|s| s.borrow_mut().by_root = by_root);
}

/// Emits the struct, statics and methods of one translated class. `hand_written` lists the
/// members (`name(Type,...)` keys) implemented in the support module instead.
pub(crate) fn emit_class(
    classes: &Classes,
    name: &str,
    members: &[Member],
    hand_written: &BTreeSet<String>,
    overrides: &BTreeMap<(String, String), Type>,
    out: &mut String,
    errors: &mut Vec<String>,
) {
    let mut em = Emitter::new(classes, name);
    let info = &classes[name];
    let (family, root, ancestors, place) = HIER.with(|h| {
        let h = h.borrow();
        (h.in_family(name), h.root_of(name), h.ancestors(name), h.places.get(name).cloned().unwrap_or_default())
    });
    let obj = if family { format!("{root}Obj") } else { name.to_string() };
    // The object's fields: in a family every field from the root down, named by declaring
    // class; otherwise the class's own.
    let mut fields: Vec<(String, T)> = Vec::new();
    if family {
        for d in ancestors.iter().rev() {
            for (f, (t, is_static)) in &classes[d].fields {
                if !is_static {
                    fields.push((format!("{d}_{f}"), t.clone()));
                }
            }
        }
    } else {
        for (f, (t, is_static)) in &info.fields {
            if !is_static {
                fields.push((rust_ident(f), t.clone()));
            }
        }
    }
    let _ = writeln!(out, "pub(crate) struct {name} {{");
    for (f, t) in &fields {
        let _ = writeln!(out, "    pub(crate) {f}: {},", t.rust());
    }
    let _ = writeln!(out, "}}\n");
    let _ = writeln!(
        out,
        "impl {name} {{\n    /// Java's default field values.\n    pub(crate) fn java_default() -> Self {{\n        {name} {{"
    );
    for (f, t) in &fields {
        let _ = writeln!(out, "            {f}: {},", t.default_value());
    }
    let _ = writeln!(out, "        }}\n    }}\n}}\n");

    // Constants and statics.
    let mut statics = String::new();
    let mut static_init = String::new();
    for m in members {
        match &m.kind {
            MemberKind::Field { decl: Stmt::Local { ty, vars }, .. }
                if vars.iter().all(|v| info.fields.get(&v.name).is_some_and(|f| f.1)) =>
            {
                for v in vars {
                    let t = var_type(name, ty, v, overrides);
                    let is_const = CONSTS.with(|c| c.borrow().contains(&(name.to_string(), v.name.clone())));
                    if is_const {
                        let init = v.init.as_ref().map(|e| em.expr_as(e, &t)).unwrap_or_default();
                        let _ = writeln!(out, "impl {name} {{\n    pub(crate) const {}: {} = {init};\n}}", v.name, t.rust());
                    } else {
                        let _ = writeln!(statics, "    pub(crate) {}: {},", rust_ident(&v.name), t.rust());
                        if let Some(init) = &v.init {
                            em.is_static = true;
                            em.scopes = vec![BTreeMap::new()];
                            let value = match init {
                                Expr::ArrayLit(items) => em.array_lit(&t, items),
                                e => em.expr_as(e, &t),
                            };
                            let _ = writeln!(
                                static_init,
                                "        {{ let __v = {value}; {name}::statics().jget_mut().{} = __v; }}",
                                rust_ident(&v.name)
                            );
                        }
                    }
                }
            }
            MemberKind::StaticInit(body) => {
                em.is_static = true;
                em.method_ret = T::Void;
                em.scopes = vec![BTreeMap::new()];
                for s in body {
                    static_init.push_str(&em.stmt(s, 2));
                }
            }
            _ => {}
        }
    }
    if info.has_statics {
        let upper = name.to_uppercase();
        let _ = writeln!(
            out,
            "pub(crate) struct {name}Statics {{\n{statics}}}\n\nthread_local! {{\n    static {upper}_STATICS: std::cell::RefCell<Option<J<{name}Statics>>> = const {{ std::cell::RefCell::new(None) }};\n}}\n\nimpl {name} {{\n    /// Static fields, initialised on first use in declaration order (Java class init).\n    pub(crate) fn statics() -> J<{name}Statics> {{\n        if let Some(s) = {upper}_STATICS.with(|s| s.borrow().clone()) {{\n            return s;\n        }}\n        let s = J::new({name}Statics::java_default());\n        {upper}_STATICS.with(|cell| *cell.borrow_mut() = Some(s.clone()));\n        Self::static_init().expect(\"static initialiser\");\n        s\n    }}\n\n    pub(crate) fn reset_statics() {{\n        {upper}_STATICS.with(|cell| *cell.borrow_mut() = None);\n    }}\n\n    fn static_init() -> JResult<()> {{\n{static_init}        Ok(())\n    }}\n}}\n"
        );
        let _ = writeln!(out, "impl {name}Statics {{\n    fn java_default() -> Self {{\n        {name}Statics {{");
        for (f, (t, is_static)) in &info.fields {
            if *is_static && !em.is_const(name, f) {
                let _ = writeln!(out, "            {}: {},", rust_ident(f), t.default_value());
            }
        }
        let _ = writeln!(out, "        }}\n    }}\n}}\n");
    }
    if place.is_interface {
        errors.append(&mut em.errors);
        return;
    }

    // Methods.
    let mut statics_impl = String::new();
    let mut instance_impl = String::new();
    let alloc = if family { format!("{root}Obj::{name}({name}::java_default())") } else { format!("{name}::java_default()") };
    // An inner class receives its enclosing instance first and stores it before any
    // constructor code runs, as javac does.
    let (outer_param, outer_set) = match place.outer.as_ref().filter(|_| place.is_inner) {
        Some(o) => (format!("this__0: {}", T::obj(o).rust()), format!("        {};\n", field_write("this", name, "this__0", "this__0"))),
        None => (String::new(), String::new()),
    };
    let with_outer = |params: &str| match (outer_param.is_empty(), params.is_empty()) {
        (true, _) => params.to_string(),
        (false, true) => outer_param.clone(),
        (false, false) => format!("{outer_param}, {params}"),
    };
    let sig_of = |idx: usize| info.methods.values().flatten().find(|s| s.member == Some(idx));
    for (idx, m) in members.iter().enumerate() {
        let (MemberKind::Method(method) | MemberKind::Constructor(method)) = &m.kind else { continue };
        let is_ctor = matches!(m.kind, MemberKind::Constructor(_));
        let key = signature_key(if is_ctor { "<init>" } else { &method.name }, &method.params);
        let Some(sig) = sig_of(idx) else { continue };
        if hand_written.contains(&key) || !method.has_body {
            continue;
        }
        em.member = key.clone();
        if is_ctor {
            let suffix = &sig.rust_name;
            instance_impl.push_str(&em.method(method, &format!("init__{name}_{suffix}"), true, false));
            if !place.is_abstract {
                let params: Vec<String> = method.params.iter().map(|(t, p)| format!("{}: {}", rust_ident(p), T::of(t).rust())).collect();
                let names: Vec<String> = method.params.iter().map(|(_, p)| rust_ident(p)).collect();
                let _ = writeln!(
                    statics_impl,
                    "    pub(crate) fn new_{suffix}({}) -> JResult<J<{obj}>> {{\n{}        let this = J::new({alloc});\n{outer_set}        this.init__{name}_{suffix}({})?;\n        Ok(this)\n    }}",
                    with_outer(&params.join(", ")),
                    class_init(name),
                    names.join(", ")
                );
            }
        } else if method.is_static {
            statics_impl.push_str(&em.method(method, &sig.rust_name, false, true));
        } else {
            let fn_name = if family { format!("{}__{name}", sig.rust_name) } else { sig.rust_name.clone() };
            instance_impl.push_str(&em.method(method, &fn_name, false, false));
        }
    }
    if !members.iter().any(|m| matches!(m.kind, MemberKind::Constructor(_))) {
        // Java's implicit constructor: `super()`, then the field initialisers.
        let implicit = Method {
            name: name.into(),
            ret: Type::named("void"),
            params: Vec::new(),
            body: Vec::new(),
            is_static: false,
            has_body: true,
            varargs: false,
            type_params: Vec::new(),
        };
        instance_impl.push_str(&em.method(&implicit, &format!("init__{name}_0"), true, false));
        if !place.is_abstract {
            let _ = writeln!(
                statics_impl,
                "    pub(crate) fn new_0({}) -> JResult<J<{obj}>> {{\n{}        let this = J::new({alloc});\n{outer_set}        this.init__{name}_0()?;\n        Ok(this)\n    }}",
                with_outer(""),
                class_init(name)
            );
        }
    }
    // This class's instance field initialisers, run by its constructors right after `super()`.
    let mut field_init = String::new();
    em.is_static = false;
    em.scopes = vec![BTreeMap::new()];
    for m in members {
        if let MemberKind::Field { decl: Stmt::Local { ty, vars }, .. } = &m.kind
            && vars.iter().all(|v| info.fields.get(&v.name).is_some_and(|f| !f.1))
        {
            for v in vars {
                if let Some(init) = &v.init {
                    let t = var_type(name, ty, v, overrides);
                    let value = match init {
                        Expr::ArrayLit(items) => em.array_lit(&t, items),
                        e => em.expr_as(e, &t),
                    };
                    let _ = writeln!(field_init, "        {{ let __v = {value}; {}; }}", field_write("self", name, &v.name, "__v"));
                }
            }
        }
    }
    let _ = writeln!(out, "impl {name} {{\n{statics_impl}}}\n");
    let _ = writeln!(
        out,
        "impl J<{obj}> {{\n    pub(crate) fn init_fields__{name}(&self) -> JResult<()> {{\n{field_init}        Ok(())\n    }}\n\n{instance_impl}}}\n"
    );
    errors.append(&mut em.errors);
}

/// The family type of `root`: the enum over its concrete classes, field accessors,
/// `instanceof` tests and virtual-call dispatchers.
pub(crate) fn emit_family(classes: &Classes, root: &str, out: &mut String) {
    let (members, ifaces, concrete) = HIER.with(|h| {
        let h = h.borrow();
        let members = h.families[root].clone();
        let ifaces: Vec<String> = h.iface_root.iter().filter(|(_, r)| *r == root).map(|(i, _)| i.clone()).collect();
        let concrete: Vec<String> = members.iter().filter(|m| !h.places[*m].is_abstract).cloned().collect();
        (members, ifaces, concrete)
    });
    let below = |c: &str| HIER.with(|h| h.borrow().concrete_below(c));
    let _ = writeln!(out, "pub(crate) enum {root}Obj {{");
    for k in &concrete {
        let _ = writeln!(out, "    {k}({k}),");
    }
    let _ = writeln!(out, "}}\n\n#[derive(Clone, Copy, PartialEq, Eq, Debug)]\npub(crate) enum {root}Class {{");
    for k in &concrete {
        let _ = writeln!(out, "    {k},");
    }
    let _ = writeln!(out, "}}\n\nimpl {root}Obj {{\n    pub(crate) fn jclass(&self) -> {root}Class {{\n        match self {{");
    for k in &concrete {
        let _ = writeln!(out, "            {root}Obj::{k}(_) => {root}Class::{k},");
    }
    let _ = writeln!(out, "        }}\n    }}\n    pub(crate) fn jclass_name(&self) -> &'static str {{\n        match self {{");
    for k in &concrete {
        let _ = writeln!(out, "            {root}Obj::{k}(_) => {k:?},");
    }
    let _ = writeln!(out, "        }}\n    }}");
    for d in &members {
        let ks = below(d);
        for (f, (t, is_static)) in &classes[d].fields {
            if *is_static {
                continue;
            }
            for (suffix, amp) in [("", "&"), ("_mut", "&mut ")] {
                let _ = writeln!(
                    out,
                    "    pub(crate) fn {d}_{f}{suffix}({amp}self) -> {amp}{} {{\n        #[allow(unreachable_patterns)]\n        match self {{",
                    t.rust()
                );
                for k in &ks {
                    let _ = writeln!(out, "            {root}Obj::{k}(o) => {amp}o.{d}_{f},");
                }
                let _ = writeln!(out, "            _ => panic!(\"java.lang.ClassCastException: not a {d}\"),\n        }}\n    }}");
            }
        }
    }
    // `Class.isInstance` by class name.
    let _ = writeln!(out, "    pub(crate) fn is_instance_of(&self, class: &str) -> bool {{\n        match class {{");
    for c in members.iter().chain(&ifaces) {
        let _ = writeln!(out, "            {c:?} => self.is_{c}(),");
    }
    let _ = writeln!(out, "            \"Object\" => true,\n            _ => false,\n        }}\n    }}");
    for c in members.iter().chain(&ifaces) {
        let ks = below(c);
        let pat = if ks.is_empty() {
            "_ if false".to_string()
        } else {
            ks.iter().map(|k| format!("{root}Obj::{k}(_)")).collect::<Vec<_>>().join(" | ")
        };
        let _ = writeln!(out, "    pub(crate) fn is_{c}(&self) -> bool {{\n        matches!(self, {pat})\n    }}");
    }
    let _ = writeln!(out, "}}\n\nimpl J<{root}Obj> {{");
    let slots = SLOTS.with(|s| s.borrow().by_root.get(root).cloned().unwrap_or_default());
    for slot in &slots {
        let params: Vec<String> = slot.params.iter().enumerate().map(|(i, t)| format!("p{i}: {}", t.rust())).collect();
        let names: Vec<String> = (0..slot.params.len()).map(|i| format!("p{i}")).collect();
        let _ = writeln!(
            out,
            "    pub(crate) fn {}(&self, {}) -> JResult<{}> {{\n        match self.jread(|o| o.jclass()) {{",
            slot.rust_name,
            params.join(", "),
            slot.ret.rust()
        );
        for k in &concrete {
            match SLOTS.with(|s| s.borrow().implementation(k, &slot.rust_name)) {
                Some(imp) => {
                    let _ = writeln!(out, "            {root}Class::{k} => self.{}__{imp}({}),", slot.rust_name, names.join(", "));
                }
                None => {
                    // `Object`'s own methods for classes that do not override them.
                    let fallback = match (slot.java_name.as_str(), slot.params.len()) {
                        ("hashCode", 0) => "Ok(self.identity_hash())".to_string(),
                        ("equals", 1) => format!("Ok(p0.instance_of::<J<{root}Obj>>(|o| o.same(self)))"),
                        ("toString", 0) => {
                            "Ok(JStr::of(&object_to_string(self.jread(|o| o.jclass_name()), self.identity_hash())))".to_string()
                        }
                        _ => format!("panic!(\"java.lang.AbstractMethodError: {k}.{}\")", slot.java_name),
                    };
                    let _ = writeln!(out, "            {root}Class::{k} => {fallback},");
                }
            }
        }
        let _ = writeln!(out, "        }}\n    }}");
        // Stable names for hand-written code: `{method}_{arity}_via_{Origin}` (the slot's own
        // name if one origin overloads the method at that arity).
        for o in &slot.origins {
            let same_arity = slots
                .iter()
                .filter(|t| t.java_name == slot.java_name && t.params.len() == slot.params.len() && t.origins.contains(o))
                .count();
            let alias = if same_arity == 1 { format!("{}_{}", slot.java_name, slot.params.len()) } else { slot.rust_name.clone() };
            let _ = writeln!(
                out,
                "    pub(crate) fn {alias}_via_{o}(&self, {}) -> JResult<{}> {{\n        self.{}({})\n    }}",
                params.join(", "),
                slot.ret.rust(),
                slot.rust_name,
                names.join(", ")
            );
        }
    }
    let _ = writeln!(out, "}}\n");
}

/// `impl JValue` and `impl JToString` for the Rust type of every translated class: Java's
/// `hashCode`/`equals`/`toString`, through the class's overrides where it has them.
pub(crate) fn emit_values(classes: &Classes, names: &[String], out: &mut String) {
    let em = Emitter::new(classes, "");
    let mut done = BTreeSet::new();
    for c in names {
        let (root, is_iface) = HIER.with(|h| {
            let h = h.borrow();
            (h.root_of(c), h.places.get(c).is_some_and(|p| p.is_interface))
        });
        if is_iface || classes.get(c).is_some_and(|i| i.is_extern) || !done.insert(root.clone()) {
            continue;
        }
        let rust = T::obj(&root).rust();
        let family = HIER.with(|h| h.borrow().in_family(&root));
        let call = |name: &str, arity: usize| em.overloads(&root, name, arity).and_then(|(_, s)| s.into_iter().find(|s| !s.is_static));
        let class_name = if family { "self.jread(|o| o.jclass_name())".to_string() } else { format!("{root:?}") };
        let hash = match call("hashCode", 0) {
            Some(s) => format!("self.{}().expect(\"hashCode\")", s.rust_name),
            None => "self.identity_hash()".into(),
        };
        let eq = match call("equals", 1) {
            Some(s) if s.params[0].class() == Some("Object") => {
                format!("self.{}(Object::of(other.clone())).expect(\"equals\")", s.rust_name)
            }
            _ => "self.same(other)".into(),
        };
        let default_string = format!("object_to_string({class_name}, self.identity_hash())");
        let string = match call("toString", 0) {
            Some(s) => format!("self.{}().expect(\"toString\").java_string()", s.rust_name),
            // Members that override `toString` below a root that does not (`AST.If`).
            None if family => {
                let concrete = HIER.with(|h| h.borrow().concrete_below(&root));
                let arms: Vec<String> = concrete
                    .iter()
                    .filter_map(|k| {
                        let s = em.overloads(k, "toString", 0).and_then(|(_, s)| s.into_iter().find(|s| !s.is_static))?;
                        Some(format!("            {k:?} => self.{}().expect(\"toString\").java_string(),", s.rust_name))
                    })
                    .collect();
                if arms.is_empty() {
                    default_string
                } else {
                    format!("match self.jread(|o| o.jclass_name()) {{\n{}\n            _ => {default_string},\n        }}", arms.join("\n"))
                }
            }
            None => default_string,
        };
        let instance_of = if family {
            "self.jtest(|o| o.is_instance_of(class))".to_string()
        } else {
            format!("!self.is_null() && (class == {root:?} || class == \"Object\")")
        };
        let class_of = if family { "self.jread(|o| o.jclass_name())".to_string() } else { format!("{root:?}") };
        let _ = writeln!(
            out,
            "impl JValue for {rust} {{\n    fn java_class(&self) -> &'static str {{\n        {class_of}\n    }}\n    fn java_instance_of(&self, class: &str) -> bool {{\n        {instance_of}\n    }}\n    fn is_java_null(&self) -> bool {{\n        self.is_null()\n    }}\n    fn object_string(&self) -> String {{\n        self.java_string()\n    }}\n    fn java_hash(&self) -> i32 {{\n        if self.is_null() {{ 0 }} else {{ {hash} }}\n    }}\n    fn java_equals(&self, other: &Self) -> bool {{\n        {eq}\n    }}\n}}\n\nimpl JToString for {rust} {{\n    fn java_string(&self) -> String {{\n        if self.is_null() {{ \"null\".into() }} else {{ {string} }}\n    }}\n}}\n"
        );
    }
}

/// `JThrow`: what a translated `throw` carries. One variant per translated exception family
/// (or class), and `Extern` for library exceptions the code throws or catches.
pub(crate) fn emit_jthrow(classes: &Classes, out: &mut String) {
    let roots: Vec<String> = HIER.with(|h| h.borrow().throwable_roots.iter().cloned().collect());
    let _ = writeln!(out, "#[derive(Clone)]\npub(crate) enum JThrow {{");
    for r in &roots {
        let _ = writeln!(out, "    {r}({}),", T::obj(r).rust());
    }
    let _ = writeln!(out, "    Extern(ExternThrowable),\n}}\n");
    let _ = writeln!(
        out,
        "impl JThrow {{\n    /// Whether this is an instance of the library class `class`.\n    pub(crate) fn is_a(&self, class: &str) -> bool {{\n        match self {{"
    );
    for r in &roots {
        let ext = HIER.with(|h| h.borrow().places[r].extern_parent.clone()).unwrap_or_default();
        let chain = hier::extern_throwable_chain(&ext);
        let _ = writeln!(out, "            JThrow::{r}(_) => {:?}.contains(&class),", chain);
    }
    let _ = writeln!(out, "            JThrow::Extern(e) => extern_is_a(e.class, class),\n        }}\n    }}\n");
    let _ = writeln!(out, "    pub(crate) fn getMessage(&self) -> JResult<JStr> {{\n        match self {{");
    let em = Emitter::new(classes, "");
    for r in &roots {
        let call = match em.overloads(r, "getMessage", 0) {
            Some((_, sigs)) if !sigs.is_empty() => format!("e.{}()", sigs[0].rust_name),
            _ => {
                let owner = em.field_of(r, "detailMessage").map(|(o, ..)| o).unwrap_or_else(|| r.clone());
                if HIER.with(|h| h.borrow().in_family(&owner)) {
                    format!("Ok(e.jread(|o| o.{owner}_detailMessage().clone()))")
                } else {
                    "Ok(e.jread(|o| o.detailMessage.clone()))".into()
                }
            }
        };
        let _ = writeln!(out, "            JThrow::{r}(e) => {call},");
    }
    let _ = writeln!(out, "            JThrow::Extern(e) => Ok(e.message.clone()),\n        }}\n    }}\n}}\n");
    let _ = writeln!(
        out,
        "impl std::fmt::Debug for JThrow {{\n    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        match self {{"
    );
    for r in &roots {
        let _ = writeln!(out, "            JThrow::{r}(_) => f.write_str({r:?}),");
    }
    let _ = writeln!(out, "            JThrow::Extern(e) => f.write_str(e.class),\n        }}\n    }}\n}}\n");
    let _ = writeln!(
        out,
        "/// The library exception hierarchy, as far as `catch` needs it.\nfn extern_is_a(class: &str, target: &str) -> bool {{\n    match class {{"
    );
    for (c, _) in hier::EXTERN_THROWABLES {
        let _ = writeln!(out, "        {c:?} => {:?}.contains(&target),", hier::extern_throwable_chain(c));
    }
    let _ = writeln!(out, "        _ => false,\n    }}\n}}\n");
}

/// Static final primitive fields initialised by a literal become Rust `const`s.
pub(crate) fn collect_consts(class: &str, members: &[Member], is_interface: bool) {
    for m in members {
        if let MemberKind::Field { decl: Stmt::Local { ty, vars }, is_static, is_final } = &m.kind
            && ((*is_static && *is_final) || is_interface)
        {
            for v in vars {
                let t = T::of(&Type { dims: ty.dims + v.extra_dims, ..ty.clone() });
                let literal = matches!(v.init, Some(Expr::Int(_) | Expr::Char(_) | Expr::Bool(_)))
                    || matches!(&v.init, Some(Expr::Unary("-", x)) if matches!(**x, Expr::Int(_)));
                if !t.is_ref() && literal {
                    CONSTS.with(|c| c.borrow_mut().insert((class.to_string(), v.name.clone())));
                }
            }
        }
    }
}
