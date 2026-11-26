//! The exact set of Java module overrides, read out of `tlc2/module/*.java`.
//!
//! `SpecProcessor.processModuleOverrides` (`SpecProcessor.java:2119-2330`) replaces a TLA+
//! definition's body with a Java method through two mechanisms, and only those two:
//!
//! 1. For every module in the module table, `tlaClass.loadClass(<module name>)`; each public
//!    static method of that class that carries none of the override annotations becomes a
//!    `MethodValue` under `TLARegistry.mapName(<method name>)`, and binds the module's
//!    definition of that name (`:2138-2185`). The classes TLC treats this way are the ones
//!    `BuiltInModuleHelper.isBuiltInModule` recognises; for any other class TLC takes the
//!    user-override path with its arity check and its own messages, which tlars does not
//!    support (DESIGN.md).
//! 2. For every class in `TLCBuiltInOverrides.get()`, each method annotated `@Evaluation`,
//!    `@TLAPlusCallable` or `@TLAPlusOperator` binds the definition the annotation names in
//!    the module the annotation names (`:2202-2325`).
//!
//! Every other definition of a module that happens to share a name with a Java class is an
//! ordinary TLA+ definition, evaluated from its body. That is the whole point of this table:
//! `_Possible!_PrintCounts` or `TLCExt!AssertEq` have no override and must not be treated as
//! one.
//!
//! The annotation's `minLevel` is the other reason the table exists: `getLevelBound` takes it
//! as the level of an application, which is what keeps `TLCGet` out of constant folding
//! (`SymbolNodeValueLookupProvider.java:270-276`).

use std::collections::BTreeMap;
use std::path::Path;

/// How `processModuleOverrides` binds the method, which is the value class it wraps it in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `MethodValue`: an unannotated public static method, or one annotated
    /// `@TLAPlusOperator`.
    Method,
    /// `EvaluatingValue`: `@Evaluation`.
    Evaluating,
    /// `CallableValue`: `@TLAPlusCallable`.
    Callable,
}

impl Kind {
    fn rust(self) -> &'static str {
        match self {
            Kind::Method => "Method",
            Kind::Evaluating => "Evaluating",
            Kind::Callable => "Callable",
        }
    }
}

struct Override {
    min_level: u8,
    kind: Kind,
    /// `tlc2.module.<class>.<method>`.
    java: String,
    /// `Method.getParameterCount()`.
    params: u8,
}

/// One public static method of a `tlc2.module` class.
struct Method {
    name: String,
    params: u8,
    /// The override annotation on it, if any.
    annotation: Option<Annotation>,
}

struct Annotation {
    kind: Kind,
    module: String,
    /// `definition()` for `@Evaluation`/`@TLAPlusCallable`, `identifier()` for
    /// `@TLAPlusOperator`.
    definition: String,
    min_level: u8,
}

pub(crate) fn generate(tlc: &Path) -> Result<String, String> {
    let dir = tlc.join("tlc2/module");
    let mut classes: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".java").map(str::to_string))
        .collect();
    classes.sort();
    let sources: BTreeMap<String, String> = classes
        .iter()
        .map(|c| {
            let path = dir.join(format!("{c}.java"));
            let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok((c.clone(), crate::catalog::strip_comments(&src)))
        })
        .collect::<Result<_, String>>()?;
    let source =
        |class: &str| -> Result<&String, String> { sources.get(class).ok_or_else(|| format!("tlc2/module/{class}.java is missing")) };

    let built_in = built_in_classes(source("BuiltInModuleHelper")?)?;
    let indexed = indexed_classes(source("TLCBuiltInOverrides")?)?;
    let registries: BTreeMap<&String, BTreeMap<String, String>> =
        classes.iter().map(|c| Ok((c, registry_names(source(c)?)?))).collect::<Result<_, String>>()?;

    // `(module, TLA+ name) -> the method bound to it`, deduplicated so that a name two
    // mechanisms or two methods claim fails here instead of depending on the order
    // `Class.getDeclaredMethods()` happens to return.
    let mut table: BTreeMap<(String, String), Override> = BTreeMap::new();
    let mut insert = |module: &str, name: &str, o: Override| -> Result<(), String> {
        let key = (module.to_string(), name.to_string());
        if let Some(prev) = table.get(&key) {
            return Err(format!("{module}!{name} is overridden twice, by {} and by {}", prev.java, o.java));
        }
        table.insert(key, o);
        Ok(())
    };

    for class in &classes {
        let registry = &registries[class];
        for m in methods(class, source(class)?)? {
            // `TLARegistry` is global and every class registers its own names in its own
            // static initialiser, which `loadClass` runs before the methods are read. Taking
            // the class's own registry is therefore exact only as long as no other class
            // maps the same method name differently.
            for (other, names) in &registries {
                if *other != class && names.get(&m.name).is_some_and(|v| Some(v) != registry.get(&m.name)) {
                    return Err(format!(
                        "{class}.{} is mapped by {other}'s TLARegistry entries, so the name depends on class-load order",
                        m.name
                    ));
                }
            }
            let java = format!("tlc2.module.{class}.{}", m.name);
            match m.annotation {
                Some(a) => {
                    if !indexed.contains(class) {
                        return Err(format!("{java} carries an override annotation but {class} is not in TLCBuiltInOverrides.get()"));
                    }
                    let o = Override { min_level: a.min_level, kind: a.kind, java, params: m.params };
                    insert(&a.module, &a.definition, o)?;
                }
                // The classic mechanism maps the method name through `TLARegistry` and binds
                // the definition of that name in the module named like the class.
                None if built_in.contains(class) => {
                    let name = registry.get(&m.name).cloned().unwrap_or_else(|| m.name.clone());
                    insert(class, &name, Override { min_level: 0, kind: Kind::Method, java, params: m.params })?;
                }
                None => {}
            }
        }
    }

    let mut out = String::from(
        "// @generated by `xtask gen-fidelity` from tlc2/module (see xtask/src/java_overrides.rs).\n\
         // Do not edit.\n\n\
         /// How `processModuleOverrides` binds the method, which is the value class it wraps it in.\n\
         #[derive(Clone, Copy, PartialEq, Eq, Debug)]\n\
         pub(crate) enum OverrideKind {\n\
         \x20   /// `MethodValue`: an unannotated public static method, or one annotated\n\
         \x20   /// `@TLAPlusOperator`.\n\
         \x20   Method,\n\
         \x20   /// `EvaluatingValue`: `@Evaluation`.\n\
         \x20   Evaluating,\n\
         \x20   /// `CallableValue`: `@TLAPlusCallable`.\n\
         \x20   Callable,\n\
         }\n\n\
         /// One definition `processModuleOverrides` replaces with a Java method.\n\
         pub(crate) struct JavaOverride {\n\
         \x20   /// The module whose definition is replaced.\n\
         \x20   pub(crate) module: &'static str,\n\
         \x20   /// The definition's TLA+ name.\n\
         \x20   pub(crate) tla_name: &'static str,\n\
         \x20   /// `@Evaluation(minLevel = ...)` / `@TLAPlusOperator(minLevel = ...)`, which\n\
         \x20   /// `getLevelBound` takes as the level of an application. The classic mechanism\n\
         \x20   /// has no annotation to read, so it is always 0 there (`MethodValue.get(Method)`).\n\
         \x20   pub(crate) min_level: u8,\n\
         \x20   pub(crate) kind: OverrideKind,\n\
         \x20   /// The Java method bound to it.\n\
         \x20   pub(crate) java: &'static str,\n\
         \x20   /// `Method.getParameterCount()`. For `OverrideKind::Evaluating` this is the\n\
         \x20   /// fixed `@Evaluation` signature's count, not the operator's arity.\n\
         \x20   pub(crate) params: u8,\n\
         }\n\n\
         /// Every Java override TLC has, by `(module, tla_name)`. A definition that is not here\n\
         /// is an ordinary TLA+ definition, whatever its module is named.\n\
         pub(crate) static JAVA_OVERRIDES: &[JavaOverride] = &[\n",
    );
    for ((module, name), o) in &table {
        out.push_str(&format!(
            "    JavaOverride {{ module: {module:?}, tla_name: {name:?}, min_level: {}, kind: OverrideKind::{}, java: {:?}, params: {} }},\n",
            o.min_level,
            o.kind.rust(),
            o.java,
            o.params,
        ));
    }
    out.push_str("];\n");
    Ok(out)
}

/// The classes `BuiltInModuleHelper.isBuiltInModule` recognises, from its
/// `clazz == <Class>.class` tests.
fn built_in_classes(src: &str) -> Result<Vec<String>, String> {
    let mut classes = Vec::new();
    for rest in src.split("clazz == ").skip(1) {
        let class = rest.split(".class").next().unwrap_or_default().trim();
        if class.is_empty() || !class.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(format!("BuiltInModuleHelper.java: cannot read the class in `clazz == {class}`"));
        }
        classes.push(class.to_string());
    }
    if classes.is_empty() {
        return Err("BuiltInModuleHelper.java: no `clazz == <Class>.class` tests".into());
    }
    Ok(classes)
}

/// `TLCBuiltInOverrides.get()`'s `return new Class[] { ... }`.
fn indexed_classes(src: &str) -> Result<Vec<String>, String> {
    let start = src.find("new Class[] {").ok_or("TLCBuiltInOverrides.java: no `new Class[] {`")?;
    let body = &src[start + "new Class[] {".len()..];
    let body = &body[..body.find('}').ok_or("TLCBuiltInOverrides.java: unterminated class array")?];
    body.split(',')
        .map(|t| {
            let t = t.trim();
            t.strip_suffix(".class").map(str::to_string).ok_or_else(|| format!("TLCBuiltInOverrides.java: unexpected array element {t}"))
        })
        .collect()
}

/// `TLARegistry.put("<java name>", "<TLA+ name>")` of one class's static initialiser, which
/// is what `TLARegistry.mapName` reads back for that class's methods.
fn registry_names(src: &str) -> Result<BTreeMap<String, String>, String> {
    let mut names = BTreeMap::new();
    for rest in src.split("TLARegistry.put(").skip(1) {
        let args = rest.split(')').next().unwrap_or_default();
        let literals: Vec<String> = split_top_level(args).iter().map(|a| java_string(a)).collect::<Result<_, _>>()?;
        let [java, tla] = literals.as_slice() else {
            return Err(format!("TLARegistry.put({args}): expected two string literals"));
        };
        if names.insert(java.clone(), tla.clone()).is_some() {
            return Err(format!("TLARegistry.put: {java} is registered twice in one class"));
        }
    }
    Ok(names)
}

/// The public static methods of `class`, each with the override annotation that precedes it.
/// Everything the reader cannot classify is an error, so an upstream shape this does not know
/// fails the generator instead of silently dropping an override.
fn methods(class: &str, src: &str) -> Result<Vec<Method>, String> {
    const MODIFIERS: &[&str] = &["public", "protected", "private", "static", "final", "synchronized", "abstract", "native"];
    let lines: Vec<&str> = src.lines().map(str::trim).collect();
    let mut out = Vec::new();
    let mut pending: Option<Annotation> = None;
    for (i, line) in lines.iter().copied().enumerate() {
        if line.starts_with('@') {
            if let Some(a) = annotation(class, line)? {
                pending = Some(a);
            }
            continue;
        }
        let Some((prefix, args)) = line.split_once('(') else { continue };
        if prefix.contains('=') {
            continue;
        }
        let tokens: Vec<&str> = prefix.split_whitespace().collect();
        let Some((name, before)) = tokens.split_last() else { continue };
        let modifiers: Vec<&&str> = before.iter().filter(|t| MODIFIERS.contains(t)).collect();
        // Not a public static method, so a pending annotation keeps waiting for one.
        if !modifiers.contains(&&"public") || !modifiers.contains(&&"static") {
            continue;
        }
        let types: Vec<&&str> = before.iter().filter(|t| !MODIFIERS.contains(t)).collect();
        if types.len() != 1 {
            return Err(format!("{class}.java: cannot read the return type of `{line}`"));
        }
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(format!("{class}.java: `{name}` is not a method name in `{line}`"));
        }
        out.push(Method { name: (*name).to_string(), params: parameter_count(class, &lines, i, args)?, annotation: pending.take() });
    }
    if let Some(a) = pending {
        return Err(format!("{class}.java: @{} on {}!{} precedes no method", a.kind.rust(), a.module, a.definition));
    }
    Ok(out)
}

/// The declared parameter count, counting the top-level commas of the parameter list, which
/// may continue on the lines after the one the declaration starts on.
fn parameter_count(class: &str, lines: &[&str], i: usize, first: &str) -> Result<u8, String> {
    let mut text = first.to_string();
    let mut j = i;
    let end = loop {
        if let Some(end) = close_paren(&text) {
            break end;
        }
        j += 1;
        let next = lines.get(j).ok_or_else(|| format!("{class}.java: unterminated parameter list of `{}`", lines[i]))?;
        text.push(' ');
        text.push_str(next);
    };
    let list = text[..end].trim();
    let count = if list.is_empty() { 0 } else { split_top_level(list).len() };
    u8::try_from(count).map_err(|e| format!("{class}.java: `{}`: {e}", lines[i]))
}

/// The index of the `)` that closes the parameter list `text` starts inside.
fn close_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(i),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// `@Evaluation(...)`, `@TLAPlusCallable(...)` or `@TLAPlusOperator(...)` on one line. Any
/// other annotation is not an override and reads as `None`.
fn annotation(class: &str, line: &str) -> Result<Option<Annotation>, String> {
    let (name, rest) = line[1..].split_once('(').map_or((&line[1..], ""), |(n, r)| (n, r));
    let kind = match name.trim() {
        "Evaluation" => Kind::Evaluating,
        "TLAPlusCallable" => Kind::Callable,
        "TLAPlusOperator" => Kind::Method,
        _ => return Ok(None),
    };
    let args = rest.strip_suffix(')').ok_or_else(|| format!("{class}.java: `{line}` does not close on one line"))?;
    let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
    for arg in split_top_level(args) {
        let (key, value) = arg.split_once('=').ok_or_else(|| format!("{class}.java: `{arg}` is not `key = value`"))?;
        if fields.insert(key.trim(), value.trim()).is_some() {
            return Err(format!("{class}.java: `{line}` sets {} twice", key.trim()));
        }
    }
    let field = |key: &str| -> Result<String, String> {
        java_string(fields.get(key).ok_or_else(|| format!("{class}.java: `{line}` has no {key}"))?)
    };
    let module = field("module")?;
    let definition = if kind == Kind::Method { field("identifier")? } else { field("definition")? };
    // Every built-in override turns the load and mismatch messages off. One that did not
    // would print EC.TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE_LOADED, which tlars does not.
    if fields.get("warn").copied() != Some("false") {
        return Err(format!("{class}.java: `{line}` does not set warn = false"));
    }
    let min_level = match fields.get("minLevel").copied() {
        None => 0,
        Some("LevelConstants.ConstantLevel") => 0,
        Some("LevelConstants.VariableLevel") => 1,
        Some("LevelConstants.ActionLevel") => 2,
        Some("LevelConstants.TemporalLevel") => 3,
        Some(v) => v.parse().map_err(|e| format!("{class}.java: `{line}`: minLevel {v}: {e}"))?,
    };
    if min_level > 3 {
        return Err(format!("{class}.java: `{line}`: minLevel {min_level} is not a TLA+ level"));
    }
    Ok(Some(Annotation { kind, module, definition, min_level }))
}

/// Splits on the commas that are not inside brackets, angle brackets or a string.
fn split_top_level(args: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start, mut in_string, mut escaped) = (0i32, 0usize, false, false);
    for (i, c) in args.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            _ if in_string => {}
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(args[start..].trim());
    parts.retain(|p| !p.is_empty());
    parts
}

/// A Java string literal's value. Only the escapes TLC's module sources use are accepted.
fn java_string(literal: &str) -> Result<String, String> {
    let body =
        literal.trim().strip_prefix('"').and_then(|s| s.strip_suffix('"')).ok_or_else(|| format!("{literal} is not a string literal"))?;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            other => return Err(format!("{literal}: unsupported escape \\{}", other.unwrap_or(' '))),
        }
    }
    Ok(out)
}
