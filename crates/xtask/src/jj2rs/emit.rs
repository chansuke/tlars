//! Rust emission for the Java subset. The output targets `tlars-sany`'s `parser::java`
//! runtime: every class `C` becomes `struct C` plus `impl J<C>` (instance methods take the
//! receiver as `&J<C>`), statics live in a lazily initialised `CStatics`, and every
//! translated method returns `JResult<_>` so Java exceptions propagate with `?`.

use super::ast::{Expr, Method, Stmt, SwitchArm, Type, Var};
use super::parse::{Member, MemberKind};
use std::collections::BTreeMap;
use std::fmt::Write;

/// A Java type, as far as emission needs to know it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum T {
    Bool,
    Int,
    Long,
    Char,
    Double,
    Str,
    Obj(String),
    Arr(Box<T>),
    Void,
    Null,
}

impl T {
    pub(crate) fn of(ty: &Type) -> T {
        let base = match ty.name.rsplit('.').next().unwrap_or(&ty.name) {
            "int" | "short" | "byte" => T::Int,
            "long" => T::Long,
            "char" => T::Char,
            "boolean" => T::Bool,
            "double" | "float" => T::Double,
            "void" => T::Void,
            "String" => T::Str,
            // The parser treats tree nodes through both names; one Rust type serves both.
            "TreeNode" => T::Obj("SyntaxTreeNode".into()),
            other => T::Obj(other.into()),
        };
        (0..ty.dims).fold(base, |t, _| T::Arr(Box::new(t)))
    }

    fn is_numeric(&self) -> bool {
        matches!(self, T::Int | T::Long | T::Char | T::Double)
    }

    fn is_ref(&self) -> bool {
        matches!(self, T::Str | T::Obj(_) | T::Arr(_) | T::Null)
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
            T::Obj(c) => format!("J<{c}>"),
            T::Arr(e) => format!("JArr<{}>", e.rust()),
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

/// What the emitter knows about a class: fields and methods with their types.
#[derive(Clone, Default)]
pub(crate) struct ClassInfo {
    pub(crate) fields: BTreeMap<String, (T, bool)>,
    /// Name -> overloads `(params, ret, is_static, returns JResult)`.
    pub(crate) methods: BTreeMap<String, Vec<MethodSig>>,
    /// Interfaces/classes whose fields resolve unqualified (`implements Constants`).
    pub(crate) inherits: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct MethodSig {
    pub(crate) params: Vec<T>,
    pub(crate) ret: T,
    pub(crate) is_static: bool,
    pub(crate) fallible: bool,
}

pub(crate) type Classes = BTreeMap<String, ClassInfo>;

/// Builds the class table entry for translated `members`.
/// `interface` fields are implicitly static.
pub(crate) fn class_info(members: &[Member], inherits: Vec<String>, is_interface: bool) -> ClassInfo {
    let mut info = ClassInfo { inherits, ..ClassInfo::default() };
    for m in members {
        match &m.kind {
            MemberKind::Field { decl: Stmt::Local { ty, vars }, is_static } => {
                for v in vars {
                    let t = T::of(&Type { name: ty.name.clone(), dims: ty.dims + v.extra_dims });
                    info.fields.insert(v.name.clone(), (t, *is_static || is_interface));
                }
            }
            MemberKind::Constructor(c) => {
                info.methods.entry("<init>".into()).or_default().push(MethodSig {
                    params: c.params.iter().map(|(t, _)| T::of(t)).collect(),
                    ret: T::Void,
                    is_static: true,
                    fallible: true,
                });
            }
            MemberKind::Method(method) => {
                info.methods.entry(method.name.clone()).or_default().push(MethodSig {
                    params: method.params.iter().map(|(t, _)| T::of(t)).collect(),
                    ret: T::of(&method.ret),
                    is_static: method.is_static,
                    fallible: true,
                });
            }
            _ => {}
        }
    }
    // A class without constructors has Java's implicit no-argument one.
    info.methods.entry("<init>".into()).or_insert_with(|| {
        vec![MethodSig { params: Vec::new(), ret: T::Void, is_static: true, fallible: true }]
    });
    info
}

/// One breakable construct: Java label (if any), Rust label, and the label `continue`
/// breaks to (loops only).
struct Breakable {
    java_label: Option<String>,
    rust_label: String,
    continue_label: Option<String>,
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
    scopes: Vec<BTreeMap<String, T>>,
    breakables: Vec<Breakable>,
    tries: Vec<TryCx>,
    /// Label attached to the next loop by an enclosing labeled statement.
    pending_label: Option<String>,
    counter: usize,
    pub(crate) errors: Vec<String>,
}

impl<'c> Emitter<'c> {
    pub(crate) fn new(classes: &'c Classes, class: &str) -> Self {
        Emitter {
            classes,
            class: class.into(),
            method_ret: T::Void,
            is_static: false,
            scopes: Vec::new(),
            breakables: Vec::new(),
            tries: Vec::new(),
            pending_label: None,
            counter: 0,
            errors: Vec::new(),
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("'{prefix}{}", self.counter)
    }

    fn err(&mut self, msg: String) -> String {
        self.errors.push(format!("{}: {msg}", self.class));
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

    fn method_of(&self, class: &str, name: &str, arity: usize) -> Option<(String, MethodSig)> {
        let info = self.classes.get(class)?;
        if let Some(sig) = info.methods.get(name).and_then(|o| o.iter().find(|s| s.params.len() == arity)) {
            return Some((class.to_string(), sig.clone()));
        }
        info.inherits.iter().find_map(|p| self.method_of(p, name, arity))
    }

    /// `C::statics()`: the lazily initialised static fields of `C`.
    fn statics(owner: &str) -> String {
        format!("{owner}::statics()")
    }

    /// Reads a field of `owner`: a constant, a static, or an instance field of `recv`.
    fn read_field(&mut self, owner: &str, name: &str, t: &T, is_static: bool, recv: Option<String>) -> String {
        let clone = if t.is_ref() { ".clone()" } else { "" };
        if is_static {
            if self.is_const(owner, name) {
                return format!("{owner}::{name}");
            }
            return format!("{}.get().{name}{clone}", Self::statics(owner));
        }
        let recv = recv.unwrap_or_else(|| "self".into());
        format!("{recv}.get().{name}{clone}")
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

    pub(crate) fn method(&mut self, m: &Method, as_constructor: Option<&str>) -> String {
        self.method_ret = if as_constructor.is_some() { T::Void } else { T::of(&m.ret) };
        self.is_static = m.is_static;
        self.scopes = vec![BTreeMap::new()];
        let mut params = Vec::new();
        for (ty, name) in &m.params {
            let t = T::of(ty);
            params.push(format!("mut {}: {}", rust_ident(name), t.rust()));
            self.scopes[0].insert(name.clone(), t);
        }
        let recv = if m.is_static { "" } else if params.is_empty() { "&self" } else { "&self, " };
        let fn_name = as_constructor.map_or_else(|| rust_ident(&m.name), str::to_string);
        let mut out = format!(
            "    pub(crate) fn {fn_name}({recv}{}) -> JResult<{}> {{\n",
            params.join(", "),
            self.method_ret.rust()
        );
        for s in &m.body {
            out.push_str(&self.stmt(s, 2));
        }
        if self.method_ret == T::Void {
            out.push_str("        #[allow(unreachable_code)]\n        Ok(())\n");
        }
        out.push_str("    }\n");
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
                self.breakables.push(Breakable { java_label: self.pending_label.take(), rust_label: label.clone(), continue_label: Some(label) });
                let body = self.nested(body, depth + 1);
                self.breakables.pop();
                format!("{ind}{head} {{\n{body}{ind}}}\n")
            }
            Stmt::DoWhile { body, cond } => {
                let label = self.loop_label();
                let inner = self.fresh("dobody");
                self.breakables.push(Breakable { java_label: self.pending_label.take(), rust_label: label.clone(), continue_label: Some(inner.clone()) });
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
                self.breakables.push(Breakable { java_label, rust_label: label.clone(), continue_label: Some(inner.clone()) });
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
                if !matches!(at, T::Arr(_)) {
                    return format!("{ind}{};\n", self.err(format!("for-each over {at:?}")));
                }
                let label = self.loop_label();
                let inner = self.fresh("forbody");
                let t = T::of(ty);
                self.breakables.push(Breakable { java_label: self.pending_label.take(), rust_label: label.clone(), continue_label: Some(inner.clone()) });
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
                    self.breakables.push(Breakable { java_label: Some(label.clone()), rust_label: rl.clone(), continue_label: None });
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
                match target {
                    Some(b) => format!("{ind}break {};\n", b.rust_label),
                    None => format!("{ind}{};\n", self.err(format!("break without target {label:?}"))),
                }
            }
            Stmt::Continue(label) => {
                let target = self.breakables.iter().rev().find(|b| {
                    b.continue_label.is_some() && (label.is_none() || b.java_label.as_deref() == label.as_deref())
                });
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
        let t = T::of(&Type { name: ty.name.clone(), dims: ty.dims + v.extra_dims });
        let init = match &v.init {
            Some(Expr::ArrayLit(items)) => self.array_lit(&t, items),
            Some(e) => self.expr_as(e, &t),
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
        let mut out = format!("{ind}{label}: {{\n{ind}    let __s: i32 = {s};\n{ind}    let __k: usize = ");
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
        self.breakables.push(Breakable { java_label: None, rust_label: label.clone(), continue_label: None });
        for (i, arm) in arms.iter().enumerate() {
            let body = self.block(&arm.body, depth + 2);
            out.push_str(&format!("{ind}    if __k <= {i} {{\n{body}{ind}    }}\n"));
        }
        self.breakables.pop();
        out.push_str(&format!("{ind}}}\n"));
        out
    }

    fn try_stmt(&mut self, body: &[Stmt], catches: &[(Type, String, Vec<Stmt>)], finally: &Option<Vec<Stmt>>, depth: usize) -> String {
        let ind = Self::indent(depth);
        let label = self.fresh("try");
        self.tries.push(TryCx { label: label.clone(), finally: finally.clone() });
        let b = self.block(body, depth + 1);
        self.tries.pop();
        let mut out = format!("{ind}let __r = {label}: {{\n{b}{ind}    #[allow(unreachable_code)]\n{ind}    Ok(())\n{ind}}};\n");
        // Catches run with this try's finally still pending.
        let catch_label = self.fresh("catch");
        self.tries.push(TryCx { label: catch_label, finally: None });
        let mut arms = String::new();
        for (ty, name, cbody) in catches {
            let class = ty.name.rsplit('.').next().unwrap_or(&ty.name).to_string();
            let pattern = match class.as_str() {
                "ParseException" => "Err(JThrow::ParseException(__e))",
                "LookaheadSuccess" => "Err(JThrow::LookaheadSuccess)",
                "TokenMgrError" => "Err(JThrow::TokenMgrError(__e))",
                "IOException" | "UnsupportedEncodingException" => "Err(JThrow::IoException)",
                _ => {
                    let e = self.err(format!("catch of {class}"));
                    arms.push_str(&format!("{ind}    Err(_) if false => {{ {e}; }}\n"));
                    continue;
                }
            };
            self.scopes.push(BTreeMap::new());
            let bind = if pattern.contains("__e") { format!("{ind}        let mut {}: J<{class}> = __e;\n", rust_ident(name)) } else { String::new() };
            if pattern.contains("__e") {
                self.scopes.last_mut().expect("scope").insert(name.clone(), T::Obj(class.clone()));
            }
            let cb = self.block(cbody, depth + 2);
            self.scopes.pop();
            arms.push_str(&format!("{ind}    {pattern} => {{\n{bind}{cb}{ind}    }}\n"));
        }
        self.tries.pop();
        let fin = finally.as_ref().map(|f| self.block(f, depth + 2)).unwrap_or_default();
        let rethrow = self.propagate("__other");
        out.push_str(&format!("{ind}#[allow(unreachable_patterns, unused_variables)]\n{ind}match __r {{\n{ind}    Ok(()) => {{}}\n{arms}{ind}    Err(__other) => {{\n{fin}{ind}        {rethrow};\n{ind}    }}\n{ind}}}\n"));
        if let Some(f) = finally {
            out.push_str(&self.block(f, depth));
        }
        out
    }

    fn throw(&mut self, e: &Expr) -> String {
        // `Error`s and unchecked exceptions abort the parse, as in TLC.
        if let Expr::New { ty, args } = e
            && matches!(ty.rsplit('.').next(), Some("Error" | "RuntimeException" | "IllegalStateException" | "IllegalArgumentException"))
        {
            let msg = match args.first() {
                Some(a) => {
                    let (v, t) = self.expr(a);
                    format!("{}.java_string()", to_string_arg(&v, &t))
                }
                None => "String::new()".into(),
            };
            return format!("panic!(\"{{}}: {{}}\", {:?}, {msg})", ty);
        }
        let (v, t) = self.expr(e);
        let err = match &t {
            T::Obj(c) if c == "ParseException" => format!("JThrow::ParseException({v})"),
            T::Obj(c) if c == "TokenMgrError" => format!("JThrow::TokenMgrError({v})"),
            T::Obj(c) if c == "LookaheadSuccess" => "JThrow::LookaheadSuccess".into(),
            T::Obj(c) if c == "IOException" => "JThrow::IoException".into(),
            // `Error`, `RuntimeException` and friends abort, as an uncaught throw does.
            _ => return format!("panic!(\"{{}}\", {v}.java_string())"),
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

    /// An expression converted to `target` (widening, `int` -> `char` constants, `null`).
    fn expr_as(&mut self, e: &Expr, target: &T) -> String {
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
            Expr::This => ("self.clone()".into(), T::Obj(self.class.clone())),
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
            Expr::New { ty, args } => self.new_object(ty, args),
            Expr::NewArray { ty, len, init } => {
                let t = T::of(ty);
                match (len, init) {
                    (_, Some(items)) => (self.array_lit(&t, items), t),
                    (Some(len), None) => {
                        let T::Arr(elem) = &t else { unreachable!() };
                        let n = self.expr_as(len, &T::Int);
                        (format!("JArr::new_array({n}, {})", elem.default_value()), t)
                    }
                    (None, None) => (self.err("array without length".into()), t),
                }
            }
            Expr::ArrayLit(_) => (self.err("bare array literal".into()), T::Null),
            Expr::Cast(ty, x) => {
                let t = T::of(ty);
                let (v, xt) = self.expr(x);
                if t.is_numeric() && xt.is_numeric() {
                    (format!("(({v}) as {})", t.rust()), t)
                } else if xt == T::Null {
                    ("JNull::null()".into(), t)
                } else {
                    // Reference casts are no-ops: one Rust type per tree-node family.
                    (v, t)
                }
            }
            Expr::Unary(op, x) => match *op {
                "!" => (format!("!({})", self.cond(x)), T::Bool),
                "-" => {
                    let (v, t) = self.expr(x);
                    let t = if t == T::Char { T::Int } else { t };
                    let v = convert(&v, &self.expr(x).1, &t);
                    (format!("({v}).wrapping_neg()"), t)
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
            Expr::Cond(c, a, b) => {
                let c = self.cond(c);
                let (_, at) = self.expr(a);
                let (_, bt) = self.expr(b);
                let t = if at.is_numeric() && bt.is_numeric() { promote(&at, &bt) } else if at == T::Null { bt } else { at };
                let av = self.expr_as(a, &t);
                let bv = self.expr_as(b, &t);
                (format!("(if {c} {{ {av} }} else {{ {bv} }})"), t)
            }
        }
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
        (self.err(format!("unknown name {n}")), T::Int)
    }

    fn field(&mut self, target: &Expr, f: &str) -> (String, T) {
        // `Class.FIELD` (static) when `target` names a class rather than a value.
        if let Expr::Name(class) = target
            && self.local(class).is_none()
            && self.field_of(&self.class.clone(), class).is_none()
        {
            let class = class.rsplit('.').next().unwrap_or(class).to_string();
            if let Some((owner, t, true)) = self.field_of(&class, f) {
                return (self.read_field(&owner, f, &t, true, None), t);
            }
            return (self.err(format!("unknown static {class}.{f}")), T::Int);
        }
        let (recv, rt) = self.expr(target);
        match &rt {
            T::Arr(_) if f == "length" => (format!("{recv}.length()"), T::Int),
            T::Obj(class) => match self.field_of(class, f) {
                Some((owner, t, is_static)) => (self.read_field(&owner, f, &t, is_static, Some(format!("({recv})"))), t),
                None => (self.err(format!("unknown field {class}.{f}")), T::Int),
            },
            other => (self.err(format!("field {f} of {other:?}")), T::Int),
        }
    }

    fn args_for(&mut self, args: &[Expr], params: &[T]) -> String {
        args.iter().zip(params).map(|(a, p)| self.expr_as(a, p)).collect::<Vec<_>>().join(", ")
    }

    fn call(&mut self, target: Option<&Expr>, name: &str, args: &[Expr]) -> (String, T) {
        let Some(target) = target else {
            if name == "this" || name == "super" {
                return (self.err(format!("{name}(...) outside a translated constructor")), T::Void);
            }
            return match self.method_of(&self.class.clone(), name, args.len()) {
                Some((owner, sig)) => {
                    let a = self.args_for(args, &sig.params);
                    let call = if sig.is_static { format!("{owner}::{}({a})", rust_ident(name)) } else { format!("self.{}({a})", rust_ident(name)) };
                    (if sig.fallible { self.tri(call) } else { call }, sig.ret)
                }
                None => (self.err(format!("unknown method {name}/{}", args.len())), T::Void),
            };
        };
        // `Class.method(...)`.
        if let Expr::Name(class) = target
            && self.local(class).is_none()
            && self.field_of(&self.class.clone(), class).is_none()
        {
            let class = class.rsplit('.').next().unwrap_or(class).to_string();
            return match self.method_of(&class, name, args.len()) {
                Some((owner, sig)) => {
                    let a = self.args_for(args, &sig.params);
                    let call = format!("{owner}::{}({a})", rust_ident(name));
                    (if sig.fallible { self.tri(call) } else { call }, sig.ret)
                }
                None => (self.err(format!("unknown static method {class}.{name}/{}", args.len())), T::Void),
            };
        }
        let (recv, rt) = self.expr(target);
        match &rt {
            T::Str => self.string_method(&recv, name, args),
            T::Obj(class) => match self.method_of(class, name, args.len()) {
                Some((_, sig)) => {
                    let a = self.args_for(args, &sig.params);
                    let call = format!("({recv}).{}({a})", rust_ident(name));
                    (if sig.fallible { self.tri(call) } else { call }, sig.ret)
                }
                None => (self.err(format!("unknown method {class}.{name}/{}", args.len())), T::Void),
            },
            other => (self.err(format!("method {name} on {other:?}")), T::Void),
        }
    }

    fn string_method(&mut self, recv: &str, name: &str, args: &[Expr]) -> (String, T) {
        let arg_types: Vec<T> = args.iter().map(|a| self.expr(a).1).collect();
        match (name, arg_types.as_slice()) {
            ("equals", [_]) => {
                let a = self.expr_as(&args[0], &T::Str);
                (format!("{recv}.equals(&{a})"), T::Bool)
            }
            ("length", []) => (format!("{recv}.length()"), T::Int),
            ("charAt", [_]) => {
                let i = self.expr_as(&args[0], &T::Int);
                (format!("{recv}.char_at({i})"), T::Char)
            }
            ("substring", [_]) => {
                let i = self.expr_as(&args[0], &T::Int);
                (format!("{recv}.substring_from({i})"), T::Str)
            }
            ("substring", [_, _]) => {
                let i = self.expr_as(&args[0], &T::Int);
                let j = self.expr_as(&args[1], &T::Int);
                (format!("{recv}.substring({i}, {j})"), T::Str)
            }
            ("indexOf", [T::Char]) => {
                let c = self.expr_as(&args[0], &T::Char);
                (format!("{recv}.index_of_char({c})"), T::Int)
            }
            ("indexOf", [T::Str]) => {
                let s = self.expr_as(&args[0], &T::Str);
                (format!("{recv}.index_of(&{s})"), T::Int)
            }
            ("startsWith", [_]) => {
                let s = self.expr_as(&args[0], &T::Str);
                (format!("{recv}.starts_with(&{s})"), T::Bool)
            }
            ("endsWith", [_]) => {
                let s = self.expr_as(&args[0], &T::Str);
                (format!("{recv}.ends_with(&{s})"), T::Bool)
            }
            ("toString" | "intern", []) => (recv.to_string(), T::Str),
            _ => (self.err(format!("String.{name}/{}", args.len())), T::Void),
        }
    }

    fn new_object(&mut self, ty: &str, args: &[Expr]) -> (String, T) {
        let class = ty.rsplit('.').next().unwrap_or(ty).to_string();
        let t = T::Obj(if class == "TreeNode" { "SyntaxTreeNode".into() } else { class.clone() });
        // Constructors are methods named `<init>` in the class table.
        match self.method_of(&class, "<init>", args.len()) {
            Some((_, sig)) => {
                let a = self.args_for(args, &sig.params);
                let call = format!("{class}::new_{}({a})", args.len());
                (if sig.fallible { self.tri(call) } else { call }, t)
            }
            None => (self.err(format!("unknown constructor {class}/{}", args.len())), t),
        }
    }

    fn binary(&mut self, op: &str, a: &Expr, b: &Expr) -> (String, T) {
        let (av, at) = self.expr(a);
        let (bv, bt) = self.expr(b);
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
                } else if at == T::Str {
                    (format!("{neg}jstr_same(&{av}, &{bv})"), T::Bool)
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
        let (_, lt) = self.expr(lhs);
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
                    Some((owner, _, true)) => format!("{}.get_mut().{n} = {v}", Self::statics(&owner)),
                    Some(_) => format!("self.get_mut().{n} = {v}"),
                    None => this.err(format!("assignment to unknown {n}")),
                },
                Expr::Field(target, f) => {
                    let (recv, _) = this.expr(target);
                    format!("({recv}).get_mut().{f} = {v}")
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
    pub(crate) static CONSTS: std::cell::RefCell<std::collections::BTreeSet<(String, String)>> = Default::default();
}

/// Implicit conversions on assignment and argument passing.
fn convert(v: &str, from: &T, to: &T) -> String {
    match (from, to) {
        (a, b) if a == b => v.to_string(),
        (T::Null, _) => "JNull::null()".into(),
        (T::Char | T::Int, T::Long) | (T::Char, T::Int) | (T::Int | T::Long | T::Char, T::Double) => format!("(({v}) as {})", to.rust()),
        // Integer constants assigned to `char`/`int` narrowing (e.g. `char c = 65`).
        (T::Int, T::Char) => format!("(({v}) as u16)"),
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
            Some('0') => {
                // Octal escape (JavaCC writes `\0`).
                let mut v = 0u32;
                while let Some(d) = chars.peek().and_then(|d| d.to_digit(8)) {
                    v = v * 8 + d;
                    chars.next();
                }
                out.push(char::from_u32(v).unwrap_or('\0'));
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
        "type" | "match" | "fn" | "mod" | "use" | "ref" | "move" | "loop" | "impl" | "trait" | "where" | "in" | "as" | "self"
        | "Self" | "crate" | "super" | "mut" | "let" | "unsafe" | "dyn" | "async" | "await" | "box" | "yield" | "macro"
        | "pub" | "struct" | "enum" | "extern" | "priv" | "override" | "typeof" | "virtual" | "final" | "abstract" | "become"
        | "do" | "unsized" | "try" | "gen" => format!("r#{n}"),
        other => other.to_string(),
    }
}

/// Emits the struct, statics and methods of one translated class. `skip` lists methods that
/// are hand-written instead.
pub(crate) fn emit_class(classes: &Classes, name: &str, members: &[Member], skip: &[&str], out: &mut String, errors: &mut Vec<String>) {
    let mut em = Emitter::new(classes, name);
    let info = &classes[name];
    // Instance fields.
    let _ = writeln!(out, "pub(crate) struct {name} {{");
    for (f, (t, is_static)) in &info.fields {
        if !is_static {
            let _ = writeln!(out, "    pub(crate) {f}: {},", t.rust());
        }
    }
    let _ = writeln!(out, "}}\n");
    let _ = writeln!(out, "impl {name} {{\n    /// Java's default field values.\n    pub(crate) fn java_default() -> Self {{\n        {name} {{");
    for (f, (t, is_static)) in &info.fields {
        if !is_static {
            let _ = writeln!(out, "            {f}: {},", t.default_value());
        }
    }
    let _ = writeln!(out, "        }}\n    }}\n}}\n");

    // Constants and statics.
    let mut statics = String::new();
    let mut static_init = String::new();
    for m in members {
        match &m.kind {
            MemberKind::Field { decl: Stmt::Local { ty, vars }, .. } if vars.iter().all(|v| info.fields.get(&v.name).is_some_and(|f| f.1)) => {
                for v in vars {
                    let t = T::of(&Type { name: ty.name.clone(), dims: ty.dims + v.extra_dims });
                    let is_const = CONSTS.with(|c| c.borrow().contains(&(name.to_string(), v.name.clone())));
                    if is_const {
                        let init = v.init.as_ref().map(|e| em.expr_as(e, &t)).unwrap_or_default();
                        let _ = writeln!(out, "impl {name} {{\n    pub(crate) const {}: {} = {init};\n}}", v.name, t.rust());
                    } else {
                        let _ = writeln!(statics, "    pub(crate) {}: {},", v.name, t.rust());
                        if let Some(init) = &v.init {
                            em.is_static = true;
                            em.scopes = vec![BTreeMap::new()];
                            let value = match init {
                                Expr::ArrayLit(items) => em.array_lit(&t, items),
                                e => em.expr_as(e, &t),
                            };
                            let _ = writeln!(static_init, "        {{ let __v = {value}; {}::statics().get_mut().{} = __v; }}", name, v.name);
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
    if !statics.is_empty() {
        let _ = writeln!(
            out,
            "#[derive(Default)]\npub(crate) struct {name}Statics {{\n{statics}}}\n\nthread_local! {{\n    static {upper}_STATICS: std::cell::RefCell<Option<J<{name}Statics>>> = const {{ std::cell::RefCell::new(None) }};\n}}\n\nimpl {name} {{\n    /// Static fields, initialised on first use in declaration order (Java class init).\n    pub(crate) fn statics() -> J<{name}Statics> {{\n        if let Some(s) = {upper}_STATICS.with(|s| s.borrow().clone()) {{\n            return s;\n        }}\n        let s = J::new({name}Statics::default());\n        {upper}_STATICS.with(|cell| *cell.borrow_mut() = Some(s.clone()));\n        Self::static_init().expect(\"static initialiser\");\n        s\n    }}\n\n    fn static_init() -> JResult<()> {{\n{static_init}        Ok(())\n    }}\n}}\n",
            upper = name.to_uppercase()
        );
    }

    // Methods.
    let mut statics_impl = String::new();
    let mut instance_impl = String::new();
    let mut ctor_count: BTreeMap<usize, usize> = BTreeMap::new();
    for m in members {
        if let MemberKind::Constructor(c) = &m.kind {
            *ctor_count.entry(c.params.len()).or_default() += 1;
        }
    }
    for m in members {
        match &m.kind {
            MemberKind::Method(method) if !skip.contains(&method.name.as_str()) => {
                let code = em.method(method, None);
                if method.is_static { statics_impl.push_str(&code) } else { instance_impl.push_str(&code) }
            }
            MemberKind::Constructor(c) if ctor_count[&c.params.len()] == 1 && !skip.contains(&"<init>") => {
                let n = c.params.len();
                let code = em.method(c, Some(&format!("init_{n}")));
                instance_impl.push_str(&code);
                let params: Vec<String> = c.params.iter().map(|(t, p)| format!("{}: {}", rust_ident(p), T::of(t).rust())).collect();
                let names: Vec<String> = c.params.iter().map(|(_, p)| rust_ident(p)).collect();
                let _ = writeln!(
                    statics_impl,
                    "    pub(crate) fn new_{n}({}) -> JResult<J<{name}>> {{\n        let this = J::new({name}::java_default());\n        this.init_fields()?;\n        this.init_{n}({})?;\n        Ok(this)\n    }}",
                    params.join(", "),
                    names.join(", ")
                );
            }
            _ => {}
        }
    }
    if ctor_count.is_empty() {
        let _ = writeln!(
            statics_impl,
            "    pub(crate) fn new_0() -> JResult<J<{name}>> {{\n        let this = J::new({name}::java_default());\n        this.init_fields()?;\n        Ok(this)\n    }}"
        );
    }
    // Instance field initialisers, run once per `new` (Java runs them in the constructor
    // that calls `super`).
    let mut field_init = String::new();
    em.is_static = false;
    em.scopes = vec![BTreeMap::new()];
    for m in members {
        if let MemberKind::Field { decl: Stmt::Local { ty, vars }, .. } = &m.kind
            && vars.iter().all(|v| info.fields.get(&v.name).is_some_and(|f| !f.1))
        {
            for v in vars {
                if let Some(init) = &v.init {
                    let t = T::of(&Type { name: ty.name.clone(), dims: ty.dims + v.extra_dims });
                    let value = match init {
                        Expr::ArrayLit(items) => em.array_lit(&t, items),
                        e => em.expr_as(e, &t),
                    };
                    let _ = writeln!(field_init, "        {{ let __v = {value}; self.get_mut().{} = __v; }}", v.name);
                }
            }
        }
    }
    let _ = writeln!(out, "impl {name} {{\n{statics_impl}}}\n");
    let _ = writeln!(
        out,
        "impl J<{name}> {{\n    pub(crate) fn init_fields(&self) -> JResult<()> {{\n{field_init}        Ok(())\n    }}\n\n{instance_impl}}}\n"
    );
    errors.append(&mut em.errors);
}

/// Static final primitive fields initialised by a literal become Rust `const`s.
pub(crate) fn collect_consts(class: &str, members: &[Member], is_interface: bool) {
    for m in members {
        if let MemberKind::Field { decl: Stmt::Local { ty, vars }, is_static } = &m.kind
            && (*is_static || is_interface)
        {
            for v in vars {
                let t = T::of(&Type { name: ty.name.clone(), dims: ty.dims + v.extra_dims });
                let literal = matches!(v.init, Some(Expr::Int(_) | Expr::Char(_) | Expr::Bool(_)))
                    || matches!(&v.init, Some(Expr::Unary("-", x)) if matches!(**x, Expr::Int(_)));
                if !t.is_ref() && literal {
                    CONSTS.with(|c| c.borrow_mut().insert((class.to_string(), v.name.clone())));
                }
            }
        }
    }
}
