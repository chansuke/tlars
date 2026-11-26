//! The translated parser against TLC's: the golden files hold, per module, what
//! `tools/golden/ParseGolden.java` printed for TLC's parser (result, logged messages, strings
//! interned while parsing, the syntax tree). Modules are parsed in the golden's order with one
//! interner, as TLC parses a spec's modules in one JVM.

use super::generated::{SyntaxTreeNode, TLAplusParser};
use super::java::*;
use super::support::{self, LogLevel, SanyOutput};
use crate::Interner;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

fn dump(out: &mut String, n: &J<SyntaxTreeNode>, depth: usize) {
    let (kind, image, loc, zero, one) = n.jread(|n| (n.kind, n.image, n.location.clone(), n.zero.clone(), n.one.clone()));
    let image = if image.is_null() { "-".into() } else { esc(image.toString().as_str()) };
    let count = |a: &JArr<J<SyntaxTreeNode>>| if a.is_null() { "-".into() } else { a.length().to_string() };
    let _ =
        writeln!(out, "{depth} {kind} {image} {},{},{},{} {} {}", loc.at(0), loc.at(1), loc.at(2), loc.at(3), count(&zero), count(&one));
    for heirs in [zero, one] {
        if heirs.is_null() {
            continue;
        }
        for i in 0..heirs.length() {
            let c = heirs.at(i);
            if c.is_null() {
                let _ = writeln!(out, "{} null", depth + 1);
            } else {
                dump(out, &c, depth + 1);
            }
        }
    }
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

/// Parses `files` (relative to `root`) in order and prints them as `ParseGolden` does.
fn parse_all(root: &Path, files: &[String]) -> String {
    let interner = Arc::new(Interner::new());
    support::enter(&interner);
    let mut out = String::new();
    let mut interned = 0;
    for file in files {
        let bytes = std::fs::read(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let sany_out = SanyOutput::new_0();
        let parser = TLAplusParser::new_from_bytes(sany_out.clone(), &bytes).expect("constructor");
        let ok = parser.parse().expect("parse() catches what the parser throws");
        let _ = writeln!(out, "== {file}\nparse {ok}");
        for (level, msg) in &sany_out.jget().messages {
            if *level != LogLevel::Trace {
                let _ = writeln!(out, "log {}", esc(&format!("{} {msg}", level_name(*level))));
            }
        }
        while let Some(a) = interner.uid_to_atom(interned + 1) {
            interned += 1;
            let _ = writeln!(out, "intern {interned} {}", esc(&interner.text(a)));
        }
        if ok {
            dump(&mut out, &parser.jread(|p| p.ParseTree.clone()), 0);
        }
    }
    out
}

/// Compares with the golden and reports the first differing line with context.
fn check(golden: &Path, root: &Path) {
    let expected = std::fs::read_to_string(golden).unwrap_or_else(|e| panic!("{}: {e}", golden.display()));
    let files: Vec<String> = expected.lines().filter_map(|l| l.strip_prefix("== ")).map(str::to_string).collect();
    let actual = parse_all(root, &files);
    if actual == expected {
        return;
    }
    // The whole output, for `diff` against the golden.
    let dump_path = crate_dir().join("../../target").join(golden.file_name().expect("file name")).with_extension("actual.txt");
    let _ = std::fs::write(&dump_path, &actual);
    let (exp, act): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    let i = exp.iter().zip(&act).position(|(a, b)| a != b).unwrap_or(exp.len().min(act.len()));
    let module = exp[..i.min(exp.len())].iter().rev().find(|l| l.starts_with("== ")).copied().unwrap_or("?");
    panic!(
        "{}: first difference at line {} (in {module}); full output in {}\n  TLC:   {:?}\n  tlars: {:?}",
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

#[test]
fn parse_errors_match_tlc() {
    check(&crate_dir().join("tests/golden/parse-errors.txt"), &crate_dir().join("tests/parse"));
}

#[test]
fn examples_match_tlc() {
    check(&crate_dir().join("tests/golden/parse-examples.txt"), &examples());
}

/// Every module in tlaplus/Examples; needs `tools/golden/parse_golden.sh --all` first.
#[test]
#[ignore]
fn all_examples() {
    check(&crate_dir().join("../../target/parse-all.txt"), &examples());
}
