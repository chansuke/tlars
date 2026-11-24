//! `xtask conformance record|run`: tla-ts's TLC corpus (`$TLA_TS_CORPUS`) against TLC at the
//! pinned revision (`$TLA2TOOLS_JAR`) and against tlars.
//!
//! `record` runs TLC on every case and context and writes the normalised outcomes to
//! `crates/tlars-conformance/expected/<suite>-<context>.txt`. `run` runs tlars in-process on
//! the same cases, prints pass counts per category and writes the failures to
//! `target/conformance/<suite>-<context>.report.txt`. The cases that pass are listed in
//! `expected/<suite>-<context>.passing`; `run --check` fails when one of them no longer
//! passes, and `run --update` rewrites the list.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tlars_conformance::{Case, Context, Corpus, Expectations, Outcome, RECORDING_LOCALE, TLC_JVM_ARGS, transcript};

pub(crate) struct Args {
    pub(crate) suite: String,
    pub(crate) context: Context,
    pub(crate) filter: Option<String>,
    pub(crate) jobs: usize,
    pub(crate) mode: RunMode,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunMode {
    Report,
    Check,
    Update,
}

const TIMEOUT: Duration = Duration::from_secs(120);

fn corpus() -> Result<Corpus, String> {
    let root = std::env::var_os("TLA_TS_CORPUS").ok_or("TLA_TS_CORPUS is not set; run inside `nix develop`")?;
    Ok(Corpus::open(Path::new(&root)))
}

fn expected_path(root: &Path, args: &Args) -> PathBuf {
    root.join(format!("crates/tlars-conformance/expected/{}-{}.txt", args.suite, args.context.name()))
}

fn passing_path(root: &Path, args: &Args) -> PathBuf {
    expected_path(root, args).with_extension("passing")
}

/// The cases of the suite that run in the context, filtered.
fn selected(args: &Args) -> Result<Vec<Case>, String> {
    let cases = corpus()?.cases(&format!("tlc/{}", args.suite)).map_err(|e| e.to_string())?;
    Ok(cases
        .into_iter()
        .filter(|c| c.contexts().contains(&args.context))
        .filter(|c| args.filter.as_ref().is_none_or(|f| c.id.contains(f.as_str())))
        .collect())
}

/// Runs `work` over `items` on `jobs` threads; results in input order.
fn parallel<T: Send + Sync + 'static, R: Send + 'static>(
    items: Vec<T>,
    jobs: usize,
    work: impl Fn(&T) -> R + Send + Sync + 'static,
) -> Vec<R> {
    let items = Arc::new(items);
    let next = Arc::new(Mutex::new(0usize));
    let work = Arc::new(work);
    let (tx, rx) = mpsc::channel();
    let handles: Vec<_> = (0..jobs.max(1))
        .map(|_| {
            let (items, next, work, tx) = (items.clone(), next.clone(), work.clone(), tx.clone());
            std::thread::spawn(move || {
                loop {
                    let i = {
                        let mut n = next.lock().expect("lock");
                        let i = *n;
                        *n += 1;
                        i
                    };
                    let Some(item) = items.get(i) else { break };
                    let _ = tx.send((i, work(item)));
                }
            })
        })
        .collect();
    drop(tx);
    let mut results: BTreeMap<usize, R> = rx.into_iter().collect();
    for h in handles {
        let _ = h.join();
    }
    (0..results.len()).map(|i| results.remove(&i).expect("every result")).collect()
}

fn scratch_dir(root: &Path, engine: &str, case: &Case, context: Context) -> Result<PathBuf, String> {
    let dir = root.join("target/conformance").join(engine).join(&case.id).join(context.name());
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// TLC on one case: the JVM `hosts.ts` runs, killed after [`TIMEOUT`].
fn run_tlc(jar: &str, root: &Path, case: &Case, context: Context) -> Result<Outcome, String> {
    let dir = scratch_dir(root, "tlc", case, context)?;
    let args = case.materialize(context, &dir).map_err(|e| e.to_string())?;
    let jvm: Vec<String> = TLC_JVM_ARGS.iter().map(|a| if *a == "-Xss2m" { case.stack() } else { a.to_string() }).collect();
    let mut child = std::process::Command::new("java")
        .args(&jvm)
        .args(["-cp", jar, "tlc2.TLC"])
        .args(&args)
        .current_dir(&dir)
        .env("LC_ALL", RECORDING_LOCALE)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("java: {e}"))?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            return Err(format!("{}: TLC timed out", case.id));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = String::from_utf8_lossy(&reader.join().expect("reader")).into_owned();
    let exit = status.code().ok_or_else(|| format!("{}: TLC killed by a signal", case.id))?;
    let transcript =
        if case.is_exit_only() { String::new() } else { transcript::normalize(&out, &dir.display().to_string(), context.keeps_state_space()) };
    Ok(Outcome { exit, transcript })
}

pub(crate) fn record(root: &Path, args: &Args) -> Result<(), String> {
    let jar = std::env::var("TLA2TOOLS_JAR").map_err(|_| "TLA2TOOLS_JAR is not set; run inside `nix develop`")?;
    let cases = selected(args)?;
    let context = args.context;
    let root_owned = root.to_path_buf();
    let results = parallel(cases.clone(), args.jobs, move |case| run_tlc(&jar, &root_owned, case, context));
    let path = expected_path(root, args);
    let mut expected = std::fs::read_to_string(&path).map(|t| Expectations::parse(&t)).unwrap_or_default();
    let mut failures = Vec::new();
    for (case, result) in cases.iter().zip(results) {
        match result {
            Ok(outcome) => {
                expected.0.insert((case.id.clone(), context), outcome);
            }
            Err(e) => failures.push(e),
        }
    }
    std::fs::create_dir_all(path.parent().expect("parent")).map_err(|e| e.to_string())?;
    std::fs::write(&path, expected.render()).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("recorded {} cases into {}", cases.len() - failures.len(), path.display());
    if failures.is_empty() { Ok(()) } else { Err(failures.join("\n")) }
}

enum Verdict {
    Pass,
    /// A difference tlars did not announce: wrong behaviour rather than a missing feature.
    Mismatch(String),
    /// tlars stopped at a construct it does not implement, and said so.
    Unimplemented(String),
    Panic(String),
    Timeout,
}

/// What a `todo!` panic says. A panic message cannot come from the spec under test, so
/// matching on it is unambiguous; `EvalError::unimplemented` is reported out of band instead
/// (`Invocation::unimplemented`), because a spec's own `Print` could imitate its text.
const UNIMPLEMENTED_PANIC: &str = "not yet implemented";

/// A run that reported a construct tlars does not implement never counts as a pass, however
/// its exit status compares: the agreement would be accidental, and an `exit-only` case has
/// no transcript that would expose it.
fn verdict(want: &Outcome, got: &Outcome, unimplemented: bool) -> Verdict {
    if unimplemented {
        let difference = if got == want { "exit and transcript agree, but the run stopped at an unimplemented construct".to_string() } else { first_difference(want, got) };
        return Verdict::Unimplemented(difference);
    }
    if got == want {
        return Verdict::Pass;
    }
    Verdict::Mismatch(first_difference(want, got))
}

/// tlars on one case, in-process, on its own thread (fresh thread-local state, a stack as
/// large as `-Xss`, and a timeout).
fn run_tlars(root: &Path, case: &Case, context: Context) -> Result<(Outcome, bool), Verdict> {
    let dir = scratch_dir(root, "tlars", case, context).map_err(Verdict::Panic)?;
    let args = case.materialize(context, &dir).map_err(|e| Verdict::Panic(e.to_string()))?;
    let keeps = context.keeps_state_space();
    let exit_only = case.is_exit_only();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let out = Arc::new(Mutex::new(Vec::new()));
            let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let inv = tlars_check::Invocation {
                args,
                system_properties: Vec::new(),
                cwd: dir.clone(),
                stdout: Box::new(SharedBuf(out.clone())),
                stderr: Box::new(std::io::sink()),
                // Hermetic: `System.in` at end of file, so a paused run resumes at once.
                stdin: Box::new(std::io::empty()),
                env: tlars_check::Env::hermetic(),
                unimplemented: Some(flag.clone()),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tlars_check::run(inv)));
            let text = String::from_utf8_lossy(&out.lock().expect("lock")).into_owned();
            let unimplemented = flag.load(std::sync::atomic::Ordering::Relaxed);
            let _ = tx.send(match result {
                Ok(status) => Ok((
                    Outcome {
                        exit: status as i32,
                        transcript: if exit_only { String::new() } else { transcript::normalize(&text, &dir.display().to_string(), keeps) },
                    },
                    unimplemented,
                )),
                Err(p) => Err(Verdict::Panic(panic_message(&p))),
            });
        })
        .map_err(|e| Verdict::Panic(e.to_string()))?;
    rx.recv_timeout(TIMEOUT).unwrap_or(Err(Verdict::Timeout))
}

struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for SharedBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

fn first_difference(expected: &Outcome, actual: &Outcome) -> String {
    if expected.exit != actual.exit {
        return format!("exit: TLC {} tlars {}", expected.exit, actual.exit);
    }
    let (e, a): (Vec<&str>, Vec<&str>) = (expected.transcript.lines().collect(), actual.transcript.lines().collect());
    let i = e.iter().zip(&a).position(|(x, y)| x != y).unwrap_or(e.len().min(a.len()));
    format!("line {}: TLC {:?} tlars {:?}", i + 1, e.get(i), a.get(i))
}

pub(crate) fn run(root: &Path, args: &Args) -> Result<(), String> {
    let path = expected_path(root, args);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}; run `xtask conformance record` first", path.display()))?;
    let expected = Arc::new(Expectations::parse(&text));
    let (deferred, cases): (Vec<Case>, Vec<Case>) = selected(args)?
        .into_iter()
        .filter(|c| expected.0.contains_key(&(c.id.clone(), args.context)))
        .partition(|c| c.deferred_to().is_some());
    let mut later: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &deferred {
        *later.entry(c.deferred_to().expect("deferred")).or_default() += 1;
    }
    // Panics are verdicts here; the default hook would print each one.
    std::panic::set_hook(Box::new(|_| {}));
    let context = args.context;
    let root_owned = root.to_path_buf();
    let exp = expected.clone();
    let verdicts = parallel(cases.clone(), args.jobs, move |case| {
        let want = &exp.0[&(case.id.clone(), context)];
        match run_tlars(&root_owned, case, context) {
            Ok((got, unimplemented)) => verdict(want, &got, unimplemented),
            Err(Verdict::Panic(m)) if m.contains(UNIMPLEMENTED_PANIC) => Verdict::Unimplemented(format!("panic: {m}")),
            Err(v) => v,
        }
    });
    let _ = std::panic::take_hook();

    let category = |id: &str| id.split('/').nth(2).unwrap_or("?").to_string();
    let mut by_category: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut passing = Vec::new();
    let mut report = String::new();
    let mut panics: BTreeMap<String, usize> = BTreeMap::new();
    let mut real = 0;
    let mut unimplemented = 0;
    for (case, v) in cases.iter().zip(&verdicts) {
        let entry = by_category.entry(category(&case.id)).or_default();
        entry.1 += 1;
        match v {
            Verdict::Pass => {
                entry.0 += 1;
                passing.push(case.id.clone());
            }
            Verdict::Mismatch(d) => {
                real += 1;
                report.push_str(&format!("REAL {} mismatch: {d}\n", case.id));
            }
            Verdict::Unimplemented(d) => {
                unimplemented += 1;
                report.push_str(&format!("{} unimplemented: {d}\n", case.id));
            }
            Verdict::Panic(m) => {
                real += 1;
                let first = m.lines().next().unwrap_or("").to_string();
                *panics.entry(first.clone()).or_default() += 1;
                report.push_str(&format!("REAL {} panic: {first}\n", case.id));
            }
            Verdict::Timeout => {
                real += 1;
                report.push_str(&format!("REAL {} timeout\n", case.id));
            }
        }
    }
    let total: usize = by_category.values().map(|v| v.1).sum();
    for (c, (p, n)) in &by_category {
        println!("  {c:<16} {p:>4}/{n}");
    }
    println!(
        "{} {}: {}/{total} pass, {real} real mismatches, {unimplemented} blocked on unimplemented features",
        args.suite,
        context.name(),
        passing.len()
    );
    if !later.is_empty() {
        println!("  not run (later milestones): {}", later.iter().map(|(m, n)| format!("{m} {n}")).collect::<Vec<_>>().join(", "));
    }
    for (m, n) in panics.iter().rev().take(5) {
        println!("  {n:>4} panics: {m}");
    }
    let report_path = root.join(format!("target/conformance/{}-{}.report.txt", args.suite, context.name()));
    std::fs::create_dir_all(report_path.parent().expect("parent")).map_err(|e| e.to_string())?;
    std::fs::write(&report_path, &report).map_err(|e| e.to_string())?;
    println!("failures: {}", report_path.display());

    let listed: Vec<String> = std::fs::read_to_string(passing_path(root, args))
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default();
    let regressed: Vec<&String> = listed.iter().filter(|id| cases.iter().any(|c| &&c.id == id) && !passing.contains(id)).collect();
    match args.mode {
        RunMode::Update if args.filter.is_none() => {
            std::fs::write(passing_path(root, args), passing.iter().map(|p| format!("{p}\n")).collect::<String>())
                .map_err(|e| e.to_string())?;
            println!("wrote {}", passing_path(root, args).display());
        }
        RunMode::Update => return Err("--update needs the whole suite (no --filter)".into()),
        _ => {}
    }
    if !regressed.is_empty() {
        let msg = format!("{} listed cases no longer pass:\n{}", regressed.len(), regressed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
        if args.mode == RunMode::Check {
            return Err(msg);
        }
        println!("{msg}");
    }
    Ok(())
}
