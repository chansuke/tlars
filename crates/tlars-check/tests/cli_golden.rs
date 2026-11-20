//! Command-line errors compared with TLC, recorded by `tools/golden/cli_golden.sh`: same
//! exit status and the same stdout, except the version string on the welcome line.

use std::io::Write;
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

/// The welcome line names the build: `TLC2 Version <...>`.
fn normalize_version(text: &str) -> String {
    text.lines()
        .map(|l| if l.starts_with("TLC2 Version ") { "TLC2 Version <version>" } else { l })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn usage_errors_match_tlc() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/cli.txt");
    let text = std::fs::read_to_string(path).expect("golden file");
    let mut failures = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\u{1f}').collect();
        let args: Vec<String> = f[0].split('\t').map(String::from).collect();
        let expected_status: i32 = f[1].parse().unwrap();
        let expected = f[2].replace("\\n", "\n").replace("\\\\", "\\");

        let out = Captured::default();
        let inv = Invocation {
            args: args.clone(),
            system_properties: Vec::new(),
            cwd: std::env::temp_dir(),
            stdout: Box::new(out.clone()),
            stderr: Box::new(std::io::sink()),
            env: Env::hermetic(),
            unimplemented: None,
        };
        let status = run(inv).code();
        let actual = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        if status != expected_status || normalize_version(&actual) != normalize_version(&expected) {
            failures.push(format!("{args:?}\n  tlars ({status}): {actual:?}\n  tlc   ({expected_status}): {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}
