//! `xtask bench`: end-to-end model-checking benchmarks over tlaplus/Examples models.
//!
//! The suite (`bench/suite.txt`) lists model `.cfg` paths relative to the Examples checkout
//! (`TLAPLUS_EXAMPLES`). Module path, expected result and state counts come from the
//! Examples manifests, so the suite holds no copies of them. Every run uses the conformance
//! arguments (`-tool -fp 0 -seed 4 -lncheck final -noGenerateSpecTE`), checks the counts
//! against the manifest, and appends one JSON line per run to `bench/results/`.

use crate::json::{self, Json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Engine {
    Tlc,
    Tlars,
}

impl Engine {
    fn name(self) -> &'static str {
        match self {
            Engine::Tlc => "tlc",
            Engine::Tlars => "tlars",
        }
    }
}

pub(crate) struct RunArgs {
    pub(crate) engine: Engine,
    pub(crate) workers: usize,
    pub(crate) repeat: usize,
    pub(crate) filter: Option<String>,
    pub(crate) timeout: Duration,
}

/// What a manifest records about a model.
struct Model {
    cfg: PathBuf,
    module: PathBuf,
    result: String,
    expected: Option<Counts>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Counts {
    generated: u64,
    distinct: u64,
    depth: u64,
}

/// One benchmark run, as written to the results file.
struct Record {
    case: String,
    engine: Engine,
    workers: usize,
    wall_ms: u128,
    /// The engine's own `Finished in <n>ms` (tool mode), excluding JVM start-up for TLC.
    reported_ms: Option<u64>,
    max_rss_kb: Option<u64>,
    exit: i32,
    counts: Option<Counts>,
    /// Counts equal the manifest's (`None`: the manifest records none).
    matches_manifest: Option<bool>,
    timed_out: bool,
}

impl Record {
    fn to_json(&self) -> String {
        let opt = |v: Option<String>| v.unwrap_or_else(|| "null".into());
        format!(
            "{{\"case\":{},\"engine\":\"{}\",\"workers\":{},\"wall_ms\":{},\"reported_ms\":{},\"max_rss_kb\":{},\"exit\":{},\"generated\":{},\"distinct\":{},\"depth\":{},\"matches_manifest\":{},\"timed_out\":{}}}",
            json::quote(&self.case),
            self.engine.name(),
            self.workers,
            self.wall_ms,
            opt(self.reported_ms.map(|v| v.to_string())),
            opt(self.max_rss_kb.map(|v| v.to_string())),
            self.exit,
            opt(self.counts.map(|c| c.generated.to_string())),
            opt(self.counts.map(|c| c.distinct.to_string())),
            opt(self.counts.map(|c| c.depth.to_string())),
            opt(self.matches_manifest.map(|v| v.to_string())),
            self.timed_out,
        )
    }
}

fn examples_root() -> Result<PathBuf, String> {
    std::env::var_os("TLAPLUS_EXAMPLES").map(PathBuf::from).ok_or_else(|| "TLAPLUS_EXAMPLES is not set; run inside `nix develop`".into())
}

/// Every model of every `specifications/*/manifest.json`, keyed by its `.cfg` path.
fn load_manifests(root: &Path) -> Result<BTreeMap<PathBuf, Model>, String> {
    let mut models = BTreeMap::new();
    let specs = std::fs::read_dir(root.join("specifications")).map_err(|e| e.to_string())?;
    for dir in specs {
        let manifest = dir.map_err(|e| e.to_string())?.path().join("manifest.json");
        let Ok(text) = std::fs::read_to_string(&manifest) else { continue };
        let doc = json::parse(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
        for module in doc.get("modules").map(Json::as_array).unwrap_or_default() {
            let Some(module_path) = module.get("path").and_then(Json::as_str) else { continue };
            for model in module.get("models").map(Json::as_array).unwrap_or_default() {
                let Some(cfg) = model.get("path").and_then(Json::as_str) else { continue };
                let count = |k: &str| model.get(k).and_then(Json::as_f64).map(|v| v as u64);
                let expected = match (count("totalStates"), count("distinctStates"), count("stateDepth")) {
                    (Some(generated), Some(distinct), Some(depth)) => Some(Counts { generated, distinct, depth }),
                    _ => None,
                };
                let result = model.get("result").and_then(Json::as_str).unwrap_or("unknown").to_string();
                models.insert(PathBuf::from(cfg), Model { cfg: cfg.into(), module: module_path.into(), result, expected });
            }
        }
    }
    Ok(models)
}

/// `bench/suite.txt`: one cfg path per line, `#` comments.
fn load_suite(workspace: &Path) -> Result<Vec<PathBuf>, String> {
    let path = workspace.join("bench/suite.txt");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()).map(PathBuf::from).collect())
}

fn case_name(cfg: &Path) -> String {
    let dir = cfg.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = cfg.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    format!("{dir}/{stem}")
}

/// The body of the first `-tool` message with `code`.
fn message_body(stdout: &str, code: u32) -> Option<&str> {
    let start = stdout.find(&format!("@!@!@STARTMSG {code}:"))?;
    let body = &stdout[start..];
    let body = &body[body.find('\n')? + 1..];
    Some(&body[..body.find("\n@!@!@ENDMSG").unwrap_or(body.len())])
}

/// 2199 `X states generated, Y distinct states found, Z states left on queue.` and 2194 depth.
fn parse_counts(stdout: &str) -> Option<Counts> {
    let stats = message_body(stdout, 2199)?;
    let mut numbers = stats.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse::<u64>());
    let generated = numbers.next()?.ok()?;
    let distinct = numbers.next()?.ok()?;
    let depth = message_body(stdout, 2194)?.split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty())?.parse().ok()?;
    Some(Counts { generated, distinct, depth })
}

/// 2186 in tool mode: `Finished in <n>ms at (...)`.
fn parse_reported_ms(stdout: &str) -> Option<u64> {
    let body = message_body(stdout, 2186)?;
    body.strip_prefix("Finished in ")?.split("ms").next()?.parse().ok()
}

fn engine_command(engine: Engine, workspace: &Path) -> Result<PathBuf, String> {
    match engine {
        Engine::Tlc => Ok(PathBuf::from("tlc")),
        Engine::Tlars => {
            let bin = workspace.join("target/release/tlars");
            if bin.exists() { Ok(bin) } else { Err("target/release/tlars missing: run `cargo build --release -p tlars`".into()) }
        }
    }
}

fn run_once(root: &Path, model: &Model, args: &RunArgs, bin: &Path, scratch: &Path) -> Result<Record, String> {
    let module = root.join(&model.module);
    let cfg = root.join(&model.cfg);
    let cwd = module.parent().ok_or("module without a directory")?;
    std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    let metadir = scratch.join("meta");
    let rss_file = scratch.join("rss");
    let _ = std::fs::remove_file(&rss_file);
    let mut cmd = Command::new("time");
    cmd.arg("-f").arg("%M").arg("-o").arg(&rss_file).arg(bin);
    cmd.args(["-tool", "-workers", &args.workers.to_string(), "-fp", "0", "-seed", "4", "-lncheck", "final", "-noGenerateSpecTE"]);
    cmd.arg("-metadir").arg(&metadir).arg("-config").arg(&cfg).arg(&module);
    cmd.current_dir(cwd).stdout(Stdio::piped()).stderr(Stdio::null());

    let start = Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("spawn {}: {e} (is GNU time on PATH?)", bin.display()))?;
    // Drain stdout on a thread so a chatty engine cannot block on a full pipe.
    let mut pipe = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut pipe, &mut s);
        s
    });
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > args.timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait().map_err(|e| e.to_string())?;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let wall_ms = start.elapsed().as_millis();
    let stdout = reader.join().unwrap_or_default();
    let counts = parse_counts(&stdout);
    let max_rss_kb = std::fs::read_to_string(&rss_file).ok().and_then(|s| s.lines().last()?.trim().parse().ok());
    let matches_manifest = match (model.expected, counts) {
        (Some(e), Some(c)) => Some(e == c),
        (Some(_), None) => Some(false),
        (None, _) => None,
    };
    Ok(Record {
        case: case_name(&model.cfg),
        engine: args.engine,
        workers: args.workers,
        wall_ms,
        reported_ms: parse_reported_ms(&stdout),
        max_rss_kb,
        exit: status.code().unwrap_or(-1),
        counts,
        matches_manifest,
        timed_out,
    })
}

fn median(mut xs: Vec<u128>) -> u128 {
    xs.sort_unstable();
    xs.get(xs.len() / 2).copied().unwrap_or(0)
}

pub(crate) fn run(workspace: &Path, args: &RunArgs) -> Result<(), String> {
    let root = examples_root()?;
    let manifests = load_manifests(&root)?;
    let bin = engine_command(args.engine, workspace)?;
    let epoch = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let results_dir = workspace.join("bench/results");
    std::fs::create_dir_all(&results_dir).map_err(|e| e.to_string())?;
    let out_path = results_dir.join(format!("{}-w{}-{epoch}.jsonl", args.engine.name(), args.workers));
    let mut out = String::new();
    println!("{:<40} {:>9} {:>9} {:>8} {:>10} {:>6}", "case", "wall s", "own s", "rss MB", "distinct", "check");
    let mut failures = 0;
    for cfg in load_suite(workspace)? {
        if args.filter.as_ref().is_some_and(|f| !cfg.to_string_lossy().contains(f.as_str())) {
            continue;
        }
        let model = manifests.get(&cfg).ok_or_else(|| format!("{} is not in any Examples manifest", cfg.display()))?;
        if model.result != "success" {
            return Err(format!("{}: manifest result is {:?}; the suite only holds successful models", cfg.display(), model.result));
        }
        let mut records = Vec::new();
        for i in 0..args.repeat {
            let scratch = workspace.join("target/bench").join(format!("{}-{i}", case_name(&cfg).replace('/', "_")));
            let record = run_once(&root, model, args, &bin, &scratch)?;
            out.push_str(&record.to_json());
            out.push('\n');
            records.push(record);
        }
        let last = records.last().ok_or("--repeat must be at least 1")?;
        let check = match (last.timed_out, last.matches_manifest, last.exit) {
            (true, ..) => "TIMEOUT",
            (_, Some(false), _) => "MISMATCH",
            (_, _, e) if e != 0 => "EXIT",
            (_, Some(true), _) => "ok",
            (_, None, _) => "-",
        };
        failures += usize::from(!matches!(check, "ok" | "-"));
        let secs = |ms: u128| format!("{:.2}", ms as f64 / 1000.0);
        println!(
            "{:<40} {:>9} {:>9} {:>8} {:>10} {:>6}",
            case_name(&cfg),
            secs(median(records.iter().map(|r| r.wall_ms).collect())),
            last.reported_ms.map_or("-".into(), |v| secs(u128::from(v))),
            last.max_rss_kb.map_or("-".into(), |v| (v / 1024).to_string()),
            last.counts.map_or("-".into(), |c| c.distinct.to_string()),
            check
        );
    }
    std::fs::write(&out_path, out).map_err(|e| e.to_string())?;
    println!("wrote {}", out_path.display());
    if failures > 0 { Err(format!("{failures} case(s) did not match the manifest")) } else { Ok(()) }
}

/// Median wall time per case of a results file.
fn load_medians(path: &Path) -> Result<BTreeMap<String, u128>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut by_case: BTreeMap<String, Vec<u128>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v = json::parse(line)?;
        let case = v.get("case").and_then(Json::as_str).ok_or("record without case")?.to_string();
        let wall = v.get("wall_ms").and_then(Json::as_f64).ok_or("record without wall_ms")? as u128;
        by_case.entry(case).or_default().push(wall);
    }
    Ok(by_case.into_iter().map(|(k, v)| (k, median(v))).collect())
}

/// `xtask bench compare <baseline.jsonl> <candidate.jsonl>`: candidate/baseline wall time.
pub(crate) fn compare(baseline: &Path, candidate: &Path) -> Result<(), String> {
    let a = load_medians(baseline)?;
    let b = load_medians(candidate)?;
    println!("{:<40} {:>10} {:>10} {:>8}", "case", "base s", "cand s", "ratio");
    for (case, base) in &a {
        let Some(cand) = b.get(case) else { continue };
        println!(
            "{:<40} {:>10.2} {:>10.2} {:>8.2}",
            case,
            *base as f64 / 1000.0,
            *cand as f64 / 1000.0,
            *cand as f64 / (*base).max(1) as f64
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_mode_summary() {
        let out = "@!@!@STARTMSG 2199:0 @!@!@\n12 states generated, 5 distinct states found, 0 states left on queue.\n@!@!@ENDMSG 2199 @!@!@\n\
                   @!@!@STARTMSG 2194:0 @!@!@\nThe depth of the complete state graph search is 4.\n@!@!@ENDMSG 2194 @!@!@\n\
                   @!@!@STARTMSG 2186:0 @!@!@\nFinished in 1234ms at (2026-11-17 22:00:00)\n@!@!@ENDMSG 2186 @!@!@\n";
        assert_eq!(parse_counts(out), Some(Counts { generated: 12, distinct: 5, depth: 4 }));
        assert_eq!(parse_reported_ms(out), Some(1234));
    }
}
