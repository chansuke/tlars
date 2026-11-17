//! Build-time generators. Inputs are the pinned tlaplus sources (read-only); outputs are
//! checked-in Rust files. `--check` regenerates in memory and fails on any difference, so
//! the generated tables and the pinned sources cannot drift apart.
//!
//!   xtask gen-fidelity [--check]   class-static UniqueStrings, BuiltInOperators, Operators +
//!                                  synonyms, SyntaxTreeConstants names, StandardModules/*.tla,
//!                                  FP64.Polys, EC/MP catalogue cross-check
//!   xtask bench run|compare      end-to-end benchmarks over tlaplus/Examples (see bench.rs)
//!
//! Implemented so far: `gen-fidelity` (FP64.Polys, EC/MP message catalog, SANY ErrorCode)
//! and `bench`. Planned:
//!   xtask jj2rs [--check]          TLAplusParser.java -> tlars-sany/src/parser/generated.rs
//!                                  (+ jj_la1 expected-token tables); unknown semantic-action
//!                                  snippets fail generation.
//!                                  TLAplusParserTokenManager.java -> tlars-sany/src/lexer/dfa.rs
//!                                  (jjnewStateCnt/jjMoveNfa tables, jjstrLiteralImages, lexical
//!                                  state transitions), so lexical-error text and DFA quirks are
//!                                  by construction too, not only parse errors
//!   xtask intern-dump-diff <dump>  compare tlars's intern sequence with an instrumented TLC
//!                                  InternTable dump for a spec

mod bench;
mod catalog;
mod jj2rs;
mod json;

use std::path::{Path, PathBuf};

enum Task {
    GenFidelity { check: bool },
    BenchRun(bench::RunArgs),
    BenchCompare { baseline: PathBuf, candidate: PathBuf },
    Jj2rsSurvey(PathBuf),
    Jj2rsEmit(PathBuf),
}

const USAGE: &str = "usage: xtask gen-fidelity [--check]\n       xtask bench run [--engine tlc|tlars] [--workers N] [--repeat N] [--filter TEXT] [--timeout SECS]\n       xtask bench compare <baseline.jsonl> <candidate.jsonl>";

fn parse_args(args: &[String]) -> Result<Task, String> {
    let strs: Vec<&str> = args.iter().map(String::as_str).collect();
    match strs.as_slice() {
        ["gen-fidelity"] => Ok(Task::GenFidelity { check: false }),
        ["gen-fidelity", "--check"] => Ok(Task::GenFidelity { check: true }),
        ["bench", "compare", a, b] => Ok(Task::BenchCompare { baseline: a.into(), candidate: b.into() }),
        ["jj2rs", "survey", file] => Ok(Task::Jj2rsSurvey(file.into())),
        ["jj2rs", "emit", dir] => Ok(Task::Jj2rsEmit(dir.into())),
        ["bench", "run", rest @ ..] => parse_bench_run(rest).map(Task::BenchRun),
        _ => Err(USAGE.into()),
    }
}

fn parse_bench_run(mut rest: &[&str]) -> Result<bench::RunArgs, String> {
    let mut args = bench::RunArgs {
        engine: bench::Engine::Tlc,
        workers: 1,
        repeat: 1,
        filter: None,
        timeout: std::time::Duration::from_secs(600),
    };
    let number = |v: &str| v.parse::<u64>().map_err(|e| format!("{v}: {e}"));
    while let [flag, value, tail @ ..] = rest {
        match *flag {
            "--engine" => {
                args.engine = match *value {
                    "tlc" => bench::Engine::Tlc,
                    "tlars" => bench::Engine::Tlars,
                    _ => return Err(format!("unknown engine {value}")),
                }
            }
            "--workers" => args.workers = number(value)? as usize,
            "--repeat" => args.repeat = number(value)?.max(1) as usize,
            "--filter" => args.filter = Some(value.to_string()),
            "--timeout" => args.timeout = std::time::Duration::from_secs(number(value)?),
            _ => return Err(USAGE.into()),
        }
        rest = tail;
    }
    if rest.is_empty() { Ok(args) } else { Err(USAGE.into()) }
}

/// The pinned tlaplus checkout. The flake's devShell sets it from `flake.lock`.
fn tlaplus_src() -> Result<PathBuf, String> {
    std::env::var_os("TLAPLUS_SRC")
        .map(PathBuf::from)
        .ok_or_else(|| "TLAPLUS_SRC is not set; run inside `nix develop`".into())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One generated file: where it goes and what it should contain.
struct Generated {
    path: PathBuf,
    contents: String,
}

fn gen_fidelity(tlaplus: &Path, check: bool) -> Result<(), String> {
    let tlc = tlaplus.join("tlatools/org.lamport.tlatools/src");
    let cat = catalog::read(&tlc)?;
    if std::env::var_os("XTASK_LIST_COMPUTED").is_some() {
        for name in catalog::computed_names(&cat) {
            println!("computed: {name}");
        }
    }
    let msg_catalog = Generated {
        path: workspace_root().join("crates/tlars-msg/src/catalog.rs"),
        contents: catalog::render(&cat)?,
    };
    let sany_codes = Generated {
        path: workspace_root().join("crates/tlars-sany/src/error_codes.rs"),
        contents: catalog::sany_error_codes(&tlc, &cat)?,
    };
    let outputs = vec![fp64_polys(&tlc)?, msg_catalog, sany_codes];
    let mut stale = Vec::new();
    for out in &outputs {
        let current = std::fs::read_to_string(&out.path).unwrap_or_default();
        if current == out.contents {
            continue;
        }
        if check {
            stale.push(out.path.display().to_string());
        } else {
            std::fs::write(&out.path, &out.contents).map_err(|e| format!("{}: {e}", out.path.display()))?;
            println!("wrote {}", out.path.display());
        }
    }
    if stale.is_empty() { Ok(()) } else { Err(format!("stale generated files:\n{}", stale.join("\n"))) }
}

/// `FP64.Polys` (the `-fp` index table) from `tlc2/util/FP64.java`.
fn fp64_polys(tlc: &Path) -> Result<Generated, String> {
    let src_path = tlc.join("tlc2/util/FP64.java");
    let src = std::fs::read_to_string(&src_path).map_err(|e| format!("{}: {e}", src_path.display()))?;
    let start = src.find("Polys = {").ok_or("FP64.java: Polys table not found")?;
    let end = start + src[start..].find("};").ok_or("FP64.java: Polys table not terminated")?;
    let polys: Vec<&str> = src[start + "Polys = {".len()..end]
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| t.strip_suffix('L').unwrap_or(t))
        .collect();
    let mut out = String::from(
        "// @generated by `xtask gen-fidelity` from tlc2/util/FP64.java (`FP64.Polys`). Do not edit.\n\n",
    );
    out.push_str(&format!("pub(crate) static POLYS: [u64; {}] = [\n", polys.len()));
    for p in &polys {
        let hex = p.strip_prefix("0x").ok_or_else(|| format!("FP64.java: unexpected poly {p}"))?;
        u64::from_str_radix(hex, 16).map_err(|e| format!("FP64.java: {p}: {e}"))?;
        out.push_str(&format!("    0x{},\n", hex.to_ascii_uppercase()));
    }
    out.push_str("];\n");
    Ok(Generated { path: workspace_root().join("crates/tlars-tool/src/fp64_polys.rs"), contents: out })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = parse_args(&args).and_then(|task| match task {
        Task::GenFidelity { check } => gen_fidelity(&tlaplus_src()?, check),
        Task::BenchRun(args) => bench::run(&workspace_root(), &args),
        Task::BenchCompare { baseline, candidate } => bench::compare(&baseline, &candidate),
        Task::Jj2rsSurvey(file) => jj2rs::survey(&file),
        Task::Jj2rsEmit(dir) => jj2rs::emit(&dir, &tlaplus_src()?, &workspace_root().join("crates/tlars-sany/src/parser/generated.rs")),
    });
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
