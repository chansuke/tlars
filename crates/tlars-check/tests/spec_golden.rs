//! Small specs run end to end and compared with the TLC transcript recorded by
//! `tools/golden/spec_golden.sh`: same exit status and the same normalized `-tool` output.
//!
//! The cases are the ones a corpus case does not cover, mostly the scenarios behind review
//! findings. Normalization is the conformance harness's own (`transcript::normalize`), so
//! timestamps, durations, build identity, paths and the volatile progress messages are out.
//!
//! A case whose directory holds an `unimplemented` file is one where TLC does something tlars
//! must not guess at. There is no transcript to match; the file names what tlars has to stop
//! on, and the run has to say exactly that.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tlars_check::{Env, Invocation, run};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn tests_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests"))
}

struct Setup {
    args: Vec<String>,
    /// What `unimplemented` says tlars has to stop on, where the case has that file.
    unimplemented: Option<String>,
}

/// The case's files in a directory of their own, so the run writes nothing into the repository.
fn materialize(case: &Path, into: &Path) -> Setup {
    let mut setup = Setup { args: Vec::new(), unimplemented: None };
    for entry in std::fs::read_dir(case).expect("case directory") {
        let path = entry.expect("case entry").path();
        let name = path.file_name().expect("file name").to_string_lossy().to_string();
        let text = || std::fs::read_to_string(&path).expect("case control file");
        match name.as_str() {
            "args" => setup.args = text().split_whitespace().map(String::from).collect(),
            "unimplemented" => setup.unimplemented = Some(text().trim().to_string()),
            _ => {
                std::fs::copy(&path, into.join(name)).expect("copy case file");
            }
        }
    }
    setup
}

#[test]
fn small_specs_match_tlc() {
    let cases = tests_dir().join("specs");
    let golden = tests_dir().join("golden/specs");
    let mut names: Vec<String> = std::fs::read_dir(&cases)
        .expect("specs directory")
        .map(|e| e.expect("case").file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(!names.is_empty(), "there is at least one case");
    let mut failures = Vec::new();
    for name in names {
        let dir = std::env::temp_dir().join(format!("tlars-spec-golden-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("case directory");
        let Setup { mut args, unimplemented } = materialize(&cases.join(&name), &dir);
        args.extend(
            ["-tool", "-workers", "1", "-fp", "0", "-seed", "4", "-lncheck", "final", "-noGenerateSpecTE", "-config"]
                .map(String::from),
        );
        args.push(dir.join("Spec.cfg").to_string_lossy().to_string());
        args.push(dir.join("Spec.tla").to_string_lossy().to_string());

        let out = Captured::default();
        let status = run(Invocation {
            args,
            system_properties: Vec::new(),
            cwd: dir.clone(),
            stdout: Box::new(out.clone()),
            stderr: Box::new(std::io::sink()),
            stdin: Box::new(std::io::empty()),
            env: Env::hermetic(),
            unimplemented: None,
        })
        .code();
        let raw = String::from_utf8(out.0.lock().unwrap().clone()).expect("utf-8 output");
        let actual = tlars_conformance::transcript::normalize(&raw, &dir.to_string_lossy(), true);
        let _ = std::fs::remove_dir_all(&dir);

        if let Some(what) = unimplemented {
            let wanted = format!("tlars does not implement {what} yet");
            if !raw.contains(&wanted) {
                failures.push(format!("{name}:\n  wanted: {wanted}\n  tlars (exit {status}):\n{actual}"));
            }
            continue;
        }
        let text = std::fs::read_to_string(golden.join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("golden file for {name}: {e}"));
        let (head, expected) = text.split_once('\n').expect("the golden starts with its exit status");
        let expected_status: i32 = head.strip_prefix("exit ").expect("exit line").trim().parse().expect("exit status");
        let expected_text = tlars_conformance::transcript::normalize(expected, "<dir>", true);
        if status != expected_status || actual != expected_text {
            failures.push(format!(
                "{name}:\n  tlars (exit {status}):\n{actual}\n  tlc   (exit {expected_status}):\n{expected_text}"
            ));
        }
    }
    assert!(failures.is_empty(), "{} case(s) differ from TLC:\n{}", failures.len(), failures.join("\n\n"));
}
