//! Structural rules of the workspace, checked on the source text so that a change that breaks
//! one fails `cargo test` with the fix in its message.
//!
//! - `tlars-tool`'s modules form layers; an import from a lower layer into a higher one is new
//!   only if it is not in [`TOOL_UPWARD_EDGES`], which may shrink but never grow.
//! - No other crate gains a module cycle beyond [`OTHER_CYCLE_EDGES`].
//! - `thread_local!` (Java statics kept per thread) stays where [`THREAD_LOCALS`] lists it.
//! - The `HashMap`/`HashSet` exception stays where [`HASH_CONTAINER_EXCEPTIONS`] lists it.
//! - External dependencies are the ones in [`EXTERNAL_DEPENDENCIES`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Lowest first. A module may import from its own layer and the layers below.
const TOOL_LAYERS: &[&[&str]] = &[
    &["fp64", "fp64_polys", "error", "globals"],
    &["value", "state", "ctx"],
    &["actions", "effects", "symmetry"],
    &["eval", "modules", "spec", "tool", "live"],
];

/// `(from, to)` imports in `tlars-tool` that point up a layer, as they stood when the check
/// was added. Cut one by moving what it needs down a layer or passing it in, then remove it
/// here.
const TOOL_UPWARD_EDGES: &[(&str, &str)] = &[
    ("actions", "eval"),
    ("ctx", "eval"),
    ("ctx", "modules"),
    ("state", "eval"),
    ("symmetry", "eval"),
    // `value` takes `&mut Ev` as TLC's values take the `Tool`; this one stays (DESIGN.md).
    ("value", "eval"),
    ("value", "symmetry"),
];

/// `(crate, from, to)` imports that close a module cycle outside `tlars-tool`.
const OTHER_CYCLE_EDGES: &[(&str, &str, &str)] = &[("tlars-sany", "cfg", "parser"), ("tlars-sany", "parser", "cfg")];

/// `(file, count)`: where `thread_local!` may appear in production code.
const THREAD_LOCALS: &[(&str, usize)] = &[
    ("crates/tlars-sany/src/parser/files.rs", 2),
    ("crates/tlars-sany/src/parser/load.rs", 1),
    ("crates/tlars-sany/src/parser/support.rs", 4),
];

/// Files that may take `#[expect(clippy::disallowed_types, ..)]`.
const HASH_CONTAINER_EXCEPTIONS: &[&str] = &["crates/tlars-sany/src/intern.rs"];

/// `(crate, dependency)`: every dependency that is not a workspace path.
const EXTERNAL_DEPENDENCIES: &[(&str, &str)] = &[
    // The binary's global allocator.
    ("tlars", "mimalloc"),
    // Transcript normalisation in the conformance harness, which is not shipped.
    ("tlars-conformance", "regex"),
];

/// Files `xtask` writes; they follow the generator, not these rules.
const GENERATED: &[&str] = &[
    "crates/tlars-msg/src/catalog.rs",
    "crates/tlars-tool/src/fp64_polys.rs",
    "crates/tlars-tool/src/modules/java_overrides.rs",
    "crates/tlars-sany/src/error_codes.rs",
    "crates/tlars-sany/src/standard_modules.rs",
    "crates/tlars-sany/src/parser/generated.rs",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).expect("under the workspace").to_string_lossy().replace('\\', "/")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("read_dir").map(|e| e.expect("entry").path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// The production source of one crate: `(path, text)` without test-only files, generated
/// files, line comments, and inline `#[cfg(test)] mod .. { .. }` blocks (which end a file).
fn production_sources(root: &Path, krate: &str) -> Vec<(PathBuf, String)> {
    let src = root.join("crates").join(krate).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    let mut test_only = BTreeSet::new();
    for f in &files {
        let text = std::fs::read_to_string(f).expect("read");
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        for w in lines.windows(2) {
            if w[0] == "#[cfg(test)]"
                && let Some(name) = w[1].strip_prefix("mod ").and_then(|r| r.strip_suffix(';'))
            {
                let stem = f.file_stem().expect("stem");
                let dir = if stem == "mod" || stem == "lib" || stem == "main" {
                    f.parent().expect("dir").to_path_buf()
                } else {
                    f.with_extension("")
                };
                test_only.insert(dir.join(format!("{name}.rs")));
                test_only.insert(dir.join(name).join("mod.rs"));
            }
        }
    }
    files
        .into_iter()
        .filter(|f| !test_only.contains(f) && !GENERATED.contains(&rel(root, f).as_str()))
        .map(|f| {
            let text = std::fs::read_to_string(&f).expect("read");
            let lines: Vec<&str> = text.lines().collect();
            let end = lines
                .windows(2)
                .position(|w| w[0].trim() == "#[cfg(test)]" && w[1].trim_start().starts_with("mod ") && w[1].trim_end().ends_with('{'))
                .unwrap_or(lines.len());
            let code: Vec<&str> = lines[..end].iter().copied().filter(|l| !l.trim_start().starts_with("//")).collect();
            (f, code.join("\n"))
        })
        .collect()
}

/// The names a `crate::` path imports at its top level: `crate::a::b` gives `a`, and
/// `crate::{a::x, b}` gives `a` and `b`.
fn crate_roots(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = code;
    while let Some(i) = rest.find("crate::") {
        let before = rest[..i].chars().next_back();
        rest = &rest[i + "crate::".len()..];
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        if let Some(group) = rest.strip_prefix('{') {
            let (mut depth, mut name, mut at_start) = (0usize, String::new(), true);
            for c in group.chars() {
                match c {
                    '{' => depth += 1,
                    '}' if depth == 0 => break,
                    '}' => depth -= 1,
                    ',' if depth == 0 => at_start = true,
                    c if depth == 0 && at_start && (c.is_alphanumeric() || c == '_') => name.push(c),
                    _ if depth == 0 && at_start && !name.is_empty() => {
                        out.push(std::mem::take(&mut name));
                        at_start = false;
                    }
                    _ => {}
                }
            }
            if !name.is_empty() {
                out.push(name);
            }
        } else {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            out.push(name);
        }
    }
    out
}

/// `(from, to)` module imports inside one crate, by top-level module.
fn module_edges(root: &Path, krate: &str) -> BTreeSet<(String, String)> {
    let src = root.join("crates").join(krate).join("src");
    let sources = production_sources(root, krate);
    let modules: BTreeSet<String> = std::fs::read_dir(&src)
        .expect("read_dir")
        .map(|e| e.expect("entry").path())
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().into_owned();
            (p.is_dir() || p.extension().is_some_and(|e| e == "rs")).then_some(name)
        })
        .filter(|n| n != "lib" && n != "main")
        .collect();
    let mut edges = BTreeSet::new();
    for (path, code) in sources {
        let top = rel(&src, &path).split('/').next().expect("component").trim_end_matches(".rs").to_string();
        if !modules.contains(&top) {
            continue;
        }
        for to in crate_roots(&code) {
            if to != top && modules.contains(&to) {
                edges.insert((top.clone(), to));
            }
        }
    }
    edges
}

fn layer_of(module: &str) -> usize {
    TOOL_LAYERS.iter().position(|l| l.contains(&module)).unwrap_or_else(|| {
        panic!("tlars-tool module `{module}` has no layer: add it to TOOL_LAYERS in crates/tlars-tool/tests/architecture.rs")
    })
}

#[test]
fn tool_modules_import_only_from_their_layer_or_below() {
    let root = workspace();
    let upward: BTreeSet<(String, String)> =
        module_edges(&root, "tlars-tool").into_iter().filter(|(from, to)| layer_of(from) < layer_of(to)).collect();
    let allowed: BTreeSet<(String, String)> = TOOL_UPWARD_EDGES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let new: Vec<_> = upward.difference(&allowed).collect();
    let gone: Vec<_> = allowed.difference(&upward).collect();
    assert!(
        new.is_empty(),
        "tlars-tool imports that point up a layer: {new:?}. Move what the lower module needs into its layer \
         (or `error`), or pass it in as an argument. Do not add the edge to TOOL_UPWARD_EDGES."
    );
    assert!(gone.is_empty(), "these upward imports no longer exist; remove them from TOOL_UPWARD_EDGES: {gone:?}");
}

#[test]
fn other_crates_gain_no_module_cycle() {
    let root = workspace();
    let mut cycles = BTreeSet::new();
    for krate in ["tlars-java", "tlars-msg", "tlars-sany", "tlars-check", "tlars-conformance", "tlars", "xtask"] {
        let edges = module_edges(&root, krate);
        for (a, b) in &edges {
            if edges.contains(&(b.clone(), a.clone())) {
                cycles.insert((krate.to_string(), a.clone(), b.clone()));
            }
        }
    }
    let allowed: BTreeSet<(String, String, String)> =
        OTHER_CYCLE_EDGES.iter().map(|(k, a, b)| (k.to_string(), a.to_string(), b.to_string())).collect();
    let new: Vec<_> = cycles.difference(&allowed).collect();
    let gone: Vec<_> = allowed.difference(&cycles).collect();
    assert!(
        new.is_empty(),
        "module imports that close a cycle, as (crate, from, to): {new:?}. Move the shared item into a module \
         both can import, or pass it in. Do not add the edge to OTHER_CYCLE_EDGES."
    );
    assert!(gone.is_empty(), "these cycles are gone; remove them from OTHER_CYCLE_EDGES: {gone:?}");
}

fn production_files_of_all_crates(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for krate in ["tlars-java", "tlars-msg", "tlars-sany", "tlars-tool", "tlars-check", "tlars-conformance", "tlars"] {
        for (p, code) in production_sources(root, krate) {
            out.push((rel(root, &p), code));
        }
    }
    out
}

#[test]
fn thread_locals_stay_where_they_are() {
    let root = workspace();
    let found: BTreeMap<String, usize> = production_files_of_all_crates(&root)
        .into_iter()
        .map(|(p, code)| (p, code.matches("thread_local!").count()))
        .filter(|(_, n)| *n > 0)
        .collect();
    let allowed: BTreeMap<String, usize> = THREAD_LOCALS.iter().map(|(p, n)| (p.to_string(), *n)).collect();
    let over: Vec<_> = found.iter().filter(|(p, n)| allowed.get(*p).is_none_or(|a| *n > a)).collect();
    assert!(
        over.is_empty(),
        "new `thread_local!`, as (file, count): {over:?}. Per-run state is a field of `Session`; \
         a Java static of the translated parser joins an existing thread_local! block in tlars-sany's parser."
    );
    assert!(found == allowed, "thread_local! counts went down to {found:?}; lower THREAD_LOCALS to match");
}

#[test]
fn hash_container_exceptions_stay_where_they_are() {
    let root = workspace();
    let found: BTreeSet<String> = production_files_of_all_crates(&root)
        .into_iter()
        .filter(|(_, code)| code.contains("clippy::disallowed_types"))
        .map(|(p, _)| p)
        .collect();
    let allowed: BTreeSet<String> = HASH_CONTAINER_EXCEPTIONS.iter().map(|p| p.to_string()).collect();
    assert!(
        found.is_subset(&allowed),
        "std HashMap/HashSet exceptions in {found:?} (allowed: {allowed:?}). Use BTreeMap/BTreeSet, or \
         tlars_java::JavaHashMap where TLC's iteration order is observable."
    );
    assert!(found == allowed, "an exception is gone; remove it from HASH_CONTAINER_EXCEPTIONS: {found:?}");
}

/// The dependency names in `[dependencies]`, `[dev-dependencies]` and `[build-dependencies]`
/// that are not `path = ...` entries.
fn external_dependencies(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_deps = matches!(line, "[dependencies]" | "[dev-dependencies]" | "[build-dependencies]");
            continue;
        }
        if in_deps
            && !line.starts_with('#')
            && let Some((name, spec)) = line.split_once('=')
            && !spec.contains("path =")
        {
            out.push(name.trim().to_string());
        }
    }
    out
}

#[test]
fn external_dependencies_are_the_listed_ones() {
    let root = workspace();
    let mut found = BTreeSet::new();
    let mut crates: Vec<_> = std::fs::read_dir(root.join("crates")).expect("crates").map(|e| e.expect("entry").path()).collect();
    crates.sort();
    for dir in crates {
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).expect("Cargo.toml");
        let krate = dir.file_name().expect("name").to_string_lossy().into_owned();
        for dep in external_dependencies(&manifest) {
            found.insert((krate.clone(), dep));
        }
    }
    let allowed: BTreeSet<(String, String)> = EXTERNAL_DEPENDENCIES.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
    let new: Vec<_> = found.difference(&allowed).collect();
    assert!(
        new.is_empty(),
        "new external dependencies, as (crate, dependency): {new:?}. Production crates use std only; write it \
         with std, or ask the owner and add it to EXTERNAL_DEPENDENCIES with the reason in Cargo.toml."
    );
    assert!(found == allowed, "a dependency is gone; remove it from EXTERNAL_DEPENDENCIES: {found:?}");
}
