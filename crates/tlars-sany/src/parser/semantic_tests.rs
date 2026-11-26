//! SANY's front end against TLC's: the golden files hold, per spec, what
//! `tools/golden/SemanticGolden.java` printed for `SANY.parse` (exit code, logged messages,
//! `ToolIO` lines, strings interned, the `ExternalModuleTable` dump and each definition's
//! level). Each spec gets a fresh interner (and with it fresh statics), as the golden runs each in a
//! fresh JVM.

use super::driver::{self, SanySettings};
use super::files::{self, FilenameToStream};
use super::generated::SpecObj;
use super::java::*;
use super::support::{self, LogLevel, SanyOutput};
use crate::Interner;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// `SemanticGolden.normalize`: "Parsing file /abs/dir/M.tla (url)" -> "Parsing file M.tla".
fn normalize(message: &str) -> String {
    let Some(path) = message.strip_prefix("Parsing file ") else { return message.to_string() };
    let path = path.find(" (").map_or(path, |i| &path[..i]);
    format!("Parsing file {}", path.rsplit('/').next().unwrap_or(path))
}

fn level_name(l: LogLevel) -> &'static str {
    match l {
        LogLevel::Trace => "TRACE",
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warning => "WARNING",
        LogLevel::Error => "ERROR",
    }
}

const SETTINGS: SanySettings = SanySettings {
    strict_error_codes: false,
    semantic_analysis: true,
    level_checking: true,
    linting: true,
    validate_pcal_translation: true,
    suppressed_codes: Vec::new(),
    messages_as_error_codes: Vec::new(),
};

fn run_one(out: &mut String, interner: &Interner, root: &Path, file: &str, depth: i32) -> JResult<()> {
    let path = root.join(file);
    let dir = path.parent().expect("parent").to_path_buf();
    let name = path.file_name().expect("file name").to_string_lossy().into_owned();
    let sany_out = SanyOutput::new_0();
    let spec =
        SpecObj::new_2(JStr::of(&name), FilenameToStream::new(Some(dir), Vec::new(), std::env::current_dir().expect("working directory")))?;
    let exit = match driver::parse(&spec, &sany_out, &SETTINGS) {
        Ok(code) => code.code().to_string(),
        Err(_) => "FrontEndException".to_string(),
    };
    let error_level = spec.getErrorLevel()?;
    let _ = writeln!(out, "== {file}\nexit {exit}\nerrorLevel {error_level}");
    for (level, msg) in &sany_out.jget().messages {
        if *level != LogLevel::Trace {
            let _ = writeln!(out, "log {}", esc(&format!("{} {}", level_name(*level), normalize(msg))));
        }
    }
    let tool_io = files::take_tool_io();
    for err in [false, true] {
        for (_, line) in tool_io.iter().filter(|(e, _)| *e == err) {
            for l in line.split('\n').filter(|l| !l.is_empty()) {
                let _ = writeln!(out, "toolio {} {}", if err { "err" } else { "out" }, esc(l));
            }
        }
    }
    // Before the dump, which interns strings of its own.
    let mut interned = 0;
    while let Some(a) = interner.uid_to_atom(interned + 1) {
        interned += 1;
        let _ = writeln!(out, "intern {interned} {}", esc(&interner.text(a)));
    }
    if exit == "0" && error_level == 0 {
        let table = spec.getExternalModuleTable()?;
        // A null dereference panics with "NullPointerException"; the golden records the
        // exception's class.
        let errors = spec.getSemanticErrors()?;
        let dump = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| table.toString_2(depth, errors)));
        let dump = match dump {
            Ok(d) => DumpResult::Text(d?.java_string()),
            Err(p)
                if p.downcast_ref::<String>().is_some_and(|m| m == "NullPointerException")
                    || p.downcast_ref::<&str>().is_some_and(|m| *m == "NullPointerException") =>
            {
                DumpResult::Exception("java.lang.NullPointerException")
            }
            Err(p) => std::panic::resume_unwind(p),
        };
        DUMP.with(|d| *d.borrow_mut() = Some(dump));
        let modules = table.getModuleNodes()?;
        let mut levels = String::new();
        for i in 0..modules.length() {
            let m = modules.at(i);
            let defs = m.getOpDefs()?;
            for j in 0..defs.length() {
                let d = defs.at(j);
                let _ = writeln!(
                    levels,
                    "level {} {} {}",
                    m.getName_0_via_SymbolNode()?.java_string(),
                    d.getName_0_via_SymbolNode()?.java_string(),
                    d.getLevel()?
                );
            }
        }
        LEVELS.with(|l| *l.borrow_mut() = Some(levels));
    }
    Ok(())
}

enum DumpResult {
    Text(String),
    Exception(&'static str),
}

thread_local! {
    static DUMP: std::cell::RefCell<Option<DumpResult>> = const { std::cell::RefCell::new(None) };
    static LEVELS: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Runs `files` (relative to `root`) in order and prints them as `SemanticGolden` does.
fn run_all(root: &Path, files: &[String], depth: i32) -> String {
    let mut out = String::new();
    for file in files {
        let interner = Arc::new(Interner::new());
        support::enter(&interner);
        support::set_host(support::test_host());
        if let Err(e) = run_one(&mut out, &interner, root, file, depth) {
            panic!("{file}: uncaught {e:?}");
        }
        match DUMP.with(|d| d.borrow_mut().take()) {
            Some(DumpResult::Text(dump)) => {
                for l in dump.split('\n') {
                    let _ = writeln!(out, "dump {}", esc(l));
                }
            }
            Some(DumpResult::Exception(class)) => {
                let _ = writeln!(out, "dump-exception {class}");
            }
            None => {}
        }
        if let Some(levels) = LEVELS.with(|l| l.borrow_mut().take()) {
            out.push_str(&levels);
        }
    }
    out
}

/// Compares with the golden and reports the first differing line with context.
fn check(golden: &Path, root: &Path) {
    let expected = std::fs::read_to_string(golden).unwrap_or_else(|e| panic!("{}: {e}", golden.display()));
    let depth: i32 = expected
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("depth "))
        .and_then(|d| d.parse().ok())
        .unwrap_or_else(|| panic!("{}: no `depth` line", golden.display()));
    let files: Vec<String> = expected.lines().filter_map(|l| l.strip_prefix("== ")).map(str::to_string).collect();
    let actual = format!("depth {depth}\n{}", run_all(root, &files, depth));
    if actual == expected {
        return;
    }
    let dump_path = crate_dir().join("../../target").join(golden.file_name().expect("file name")).with_extension("actual.txt");
    let _ = std::fs::write(&dump_path, &actual);
    let (exp, act): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    let i = exp.iter().zip(&act).position(|(a, b)| a != b).unwrap_or(exp.len().min(act.len()));
    let spec = exp[..i.min(exp.len())].iter().rev().find(|l| l.starts_with("== ")).copied().unwrap_or("?");
    panic!(
        "{}: first difference at line {} (in {spec}); full output in {}\n  TLC:   {:?}\n  tlars: {:?}",
        golden.display(),
        i + 1,
        dump_path.display(),
        exp.get(i),
        act.get(i)
    );
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn examples() -> PathBuf {
    PathBuf::from(std::env::var_os("TLAPLUS_EXAMPLES").expect("TLAPLUS_EXAMPLES is not set; run inside `nix develop`"))
}

/// Runs on a large stack: the generator and level checker recurse per expression level.
fn on_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new().stack_size(1 << 28).spawn(f).expect("thread").join().unwrap_or_else(|e| std::panic::resume_unwind(e));
}

/// Specs that exercise the linter, in `tests/semantic`.
#[test]
fn semantic_fixtures_match_tlc() {
    on_big_stack(|| check(&crate_dir().join("tests/golden/semantic-fixtures.txt"), &crate_dir().join("tests/semantic")));
}

#[test]
fn semantic_examples_match_tlc() {
    on_big_stack(|| check(&crate_dir().join("tests/golden/semantic-examples.txt"), &examples()));
}

/// Every module in tlaplus/Examples as a root; needs `tools/golden/semantic_golden.sh --all`.
#[test]
#[ignore]
fn semantic_all_examples() {
    on_big_stack(|| check(&crate_dir().join("../../target/semantic-all.txt"), &examples()));
}

/// The golden's lines for one spec, by tag.
struct GoldenSpec {
    file: String,
    exit: String,
    lines: Vec<String>,
}

fn golden_specs(golden: &Path) -> Vec<GoldenSpec> {
    let text = std::fs::read_to_string(golden).unwrap_or_else(|e| panic!("{}: {e}", golden.display()));
    let mut specs: Vec<GoldenSpec> = Vec::new();
    for line in text.lines() {
        if let Some(f) = line.strip_prefix("== ") {
            specs.push(GoldenSpec { file: f.to_string(), exit: String::new(), lines: Vec::new() });
        } else if let Some(spec) = specs.last_mut() {
            if let Some(e) = line.strip_prefix("exit ") {
                spec.exit = e.to_string();
            }
            spec.lines.push(line.to_string());
        }
    }
    specs
}

#[derive(Default)]
struct Console(Vec<String>);

impl crate::SanyOut for Console {
    fn log_line(&mut self, text: &str) {
        self.line(text);
    }
    fn line(&mut self, text: &str) {
        self.0.push(format!("out {}", esc(&normalize(text))));
    }
    fn err_line(&mut self, text: &str) {
        self.0.push(format!("err {}", esc(text)));
    }
}

/// `crate::load` against the goldens: the console lines are TLC's log and `ToolIO` lines,
/// and for specs SANY accepts, the graph's modules, definitions and levels are TLC's.
fn check_load(golden: &Path, root: &Path) {
    for spec in golden_specs(golden) {
        let path = root.join(&spec.file);
        let dir = path.parent().expect("parent").to_path_buf();
        let name = path.file_name().expect("file name").to_string_lossy().into_owned();
        let interner = Arc::new(Interner::new());
        let mut console = Console::default();
        let req = crate::LoadRequest {
            root_file: &name,
            user_dir: Some(&dir),
            library_paths: &[],
            working_dir: &dir,
            interner: interner.clone(),
            extra_root_extends: &[],
            suppressed_codes: &[],
            messages_as_errors: &[],
            host: support::test_host(),
        };
        let result = crate::load(req, &mut console);
        // The golden keeps SANY's messages and ToolIO's lines apart; both reach the console.
        let mut expected: Vec<String> = spec
            .lines
            .iter()
            .filter_map(|l| {
                let l = l.strip_prefix("log ")?;
                let (_, msg) = l.split_once(' ')?;
                Some(format!("out {msg}"))
            })
            .collect();
        expected.extend(spec.lines.iter().filter_map(|l| l.strip_prefix("toolio ")).map(|l| {
            let (stream, msg) = l.split_once(' ').unwrap_or((l, ""));
            format!("{stream} {msg}")
        }));
        // The golden drops empty `ToolIO` lines.
        let mut actual: Vec<String> = console.0.iter().filter(|l| *l != "out " && *l != "err ").cloned().collect();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected, "{}: console lines (sorted)", spec.file);
        match (&result, spec.exit.as_str()) {
            (Ok(graph), "0") => {
                let expected_levels: Vec<&String> = spec.lines.iter().filter(|l| l.starts_with("level ")).collect();
                let mut levels = Vec::new();
                let externals = spec.lines.iter().filter(|l| l.starts_with("log INFO Semantic processing of module ")).count();
                for (_, m) in graph.modules().take(externals) {
                    for d in m.op_defs.iter() {
                        let level = graph.op_level(*d) as i32;
                        levels.push(format!("level {} {} {level}", interner.text(m.name), interner.text(graph.op_def(*d).name)));
                    }
                }
                let levels: Vec<&String> = levels.iter().collect();
                assert_eq!(levels, expected_levels, "{}: definitions and levels", spec.file);
            }
            (Err(_), "0") => panic!("{}: SANY accepted the spec, load failed: {result:?}", spec.file, result = result.as_ref().err()),
            (Ok(_), exit) => panic!("{}: SANY failed (exit {exit}), load succeeded", spec.file),
            (Err(_), _) => {}
        }
    }
}

#[test]
fn load_matches_tlc() {
    check_load(&crate_dir().join("tests/golden/semantic-fixtures.txt"), &crate_dir().join("tests/semantic"));
    check_load(&crate_dir().join("tests/golden/semantic-examples.txt"), &examples());
}

/// `load` for every module in tlaplus/Examples as a root; needs
/// `tools/golden/semantic_golden.sh --all`.
#[test]
#[ignore]
fn load_all_examples() {
    check_load(&crate_dir().join("../../target/semantic-all.txt"), &examples());
}
