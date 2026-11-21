//! Whole-spec runs compared with TLC, for behaviour the corpus does not reach: each fixture
//! under `tests/specs/<case>/` is a spec and a cfg, and `tests/golden/specs.txt` holds the
//! exit status and normalised `-tool` transcript TLC at the pinned revision produced for it
//! (recorded by `tools/golden/spec_golden.sh`). tlars runs the same arguments in-process and
//! must agree on both.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tlars_conformance::{DETERMINISTIC_ARGS, transcript};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Undoes the recorder's escaping in one left-to-right pass: a transcript can hold a
/// literal `\n` of its own (a message TLC embedded in a string), which two passes of
/// `replace` would turn into a newline.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn specs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/specs")
}

/// The fixture's files copied into a scratch directory, so the run writes nothing into the
/// repository, and the extra arguments its `args` file asks for.
fn materialize(case: &str, dir: &Path) -> Vec<String> {
    let source = specs_dir().join(case);
    let mut extra = Vec::new();
    for entry in std::fs::read_dir(&source).expect("fixture directory") {
        let path = entry.expect("entry").path();
        let name = path.file_name().expect("file name").to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).expect("fixture file");
        if name == "args" {
            extra.extend(text.lines().filter(|l| !l.is_empty()).map(str::to_string));
        } else {
            std::fs::write(dir.join(&name), text).expect("write fixture");
        }
    }
    let mut args = vec!["-tool".to_string()];
    args.extend(DETERMINISTIC_ARGS.iter().map(|a| a.to_string()));
    args.push("-metadir".into());
    args.push(dir.join(".meta").display().to_string());
    args.extend(extra);
    args.push("-config".into());
    args.push(format!("{case}.cfg"));
    args.push(format!("{case}.tla"));
    args
}

/// tlars on one fixture, in-process on a thread with a stack as large as TLC's `-Xss`.
fn run_tlars(case: &str) -> (i32, String) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("spec_golden").join(case);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clear scratch");
    }
    std::fs::create_dir_all(&dir).expect("scratch");
    let args = materialize(case, &dir);
    let out = Captured::default();
    let (captured, scratch) = (out.clone(), dir.clone());
    let status = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            tlars_check::run(tlars_check::Invocation {
                args,
                system_properties: vec![("tlc2.output.MP.noTimestamps".into(), "true".into())],
                cwd: scratch,
                stdout: Box::new(captured),
                stderr: Box::new(std::io::sink()),
                stdin: Box::new(std::io::empty()),
                env: tlars_check::Env::hermetic(),
                unimplemented: None,
            })
        })
        .expect("spawn")
        .join()
        .expect("tlars panicked");
    let text = String::from_utf8_lossy(&out.0.lock().expect("lock")).into_owned();
    (status as i32, transcript::normalize(&text, &dir.display().to_string(), true))
}

#[test]
fn spec_runs_match_tlc() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/specs.txt");
    let text = std::fs::read_to_string(&path).expect("golden file");
    let mut cases = Vec::new();
    let mut failures = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        let case = f[0];
        let expected_status: i32 = f[1].parse().expect("status");
        let expected = unescape(f[2]);
        cases.push(case.to_string());
        let (status, actual) = run_tlars(case);
        if status != expected_status || actual.trim_end() != expected.trim_end() {
            failures.push(format!("{case}\n  tlars ({status}):\n{actual}\n  tlc   ({expected_status}):\n{expected}"));
        }
    }
    let fixtures: Vec<String> = std::fs::read_dir(specs_dir())
        .expect("specs directory")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    let unrecorded: Vec<&String> = fixtures.iter().filter(|f| !cases.contains(f)).collect();
    assert!(unrecorded.is_empty(), "fixtures with no recording; run tools/golden/spec_golden.sh: {unrecorded:?}");
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}
