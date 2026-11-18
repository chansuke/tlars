//! tla-ts's TLC conformance corpus (`packages/conformance/test`, MIT), run against TLC and
//! tlars with the same invocation and compared after the same normalisation.
//!
//! Ports of tla-ts's `test-file.ts` (the YAML header, the five expression contexts, the
//! files a case writes) and `transcript.ts` (normalising `-tool` output). The expected
//! outcomes come from TLC at the pinned revision (`xtask conformance record`), not from the
//! corpus's own recordings, which another TLC revision made; and `correct:` overrides do
//! not apply, because tlars reproduces TLC.

pub mod transcript;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// `hosts.ts` `DETERMINISTIC_ARGUMENTS`.
pub const DETERMINISTIC_ARGS: &[&str] =
    &["-workers", "1", "-fp", "0", "-seed", "4", "-lncheck", "final", "-noGenerateSpecTE"];

/// `hosts.ts`: the JVM TLC runs in for the recordings.
pub const TLC_JVM_ARGS: &[&str] =
    &["-XX:+UseSerialGC", "-XX:TieredStopAtLevel=1", "-XX:-UsePerfData", "-Xmx1g", "-Xss2m"];

const STATE: &str = "conformance_state";

/// Where a case runs: one of `EXPRESSION_CONTEXTS` for `flags: [expression]` cases, else
/// the module as written (`model`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Context {
    Assume,
    Init,
    Next,
    Invariant,
    Property,
    Model,
}

impl Context {
    pub const EXPRESSION: [Context; 5] = [Context::Assume, Context::Init, Context::Next, Context::Invariant, Context::Property];

    pub fn name(self) -> &'static str {
        match self {
            Context::Assume => "assume",
            Context::Init => "init",
            Context::Next => "next",
            Context::Invariant => "invariant",
            Context::Property => "property",
            Context::Model => "model",
        }
    }

    pub fn from_name(name: &str) -> Option<Context> {
        Context::EXPRESSION.into_iter().chain([Context::Model]).find(|c| c.name() == name)
    }

    /// Text appended before the module's last `====` line, and the cfg it runs with.
    fn wrapping(self) -> (String, &'static str) {
        let state = |init: &str, next: &str, check: &str| {
            format!("VARIABLE {STATE}\nInit == {STATE} = {init}\nNext == {next}\nInv == {check}")
        };
        let state_cfg = "INIT Init\nNEXT Next\nINVARIANT Inv\n";
        match self {
            Context::Assume => ("ASSUME Test".into(), ""),
            Context::Init => (state("Test", &format!("UNCHANGED {STATE}"), STATE), state_cfg),
            Context::Next => (state("TRUE", &format!("{STATE}' = Test"), STATE), state_cfg),
            Context::Invariant => (state("TRUE", &format!("UNCHANGED {STATE}"), &format!("{STATE} /\\ Test")), state_cfg),
            Context::Property => (
                format!("VARIABLE {STATE}\nSpec == {STATE} = TRUE /\\ [][UNCHANGED {STATE}]_{STATE}\nProp == []({STATE} /\\ Test)"),
                "SPECIFICATION Spec\nPROPERTY Prop\n",
            ),
            Context::Model => (String::new(), ""),
        }
    }

    /// `isStateSpaceRelevant`: only the module as written keeps state-space statistics.
    pub fn keeps_state_space(self) -> bool {
        self == Context::Model
    }
}

/// The header fields that decide how a case runs.
#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub outcome: String,
    pub flags: Vec<String>,
    /// `contexts:` narrows the expression contexts.
    pub contexts: Option<Vec<String>>,
    pub config: Option<String>,
    pub args: Vec<String>,
    /// `-Xss` for this case.
    pub stack: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Case {
    /// Path under the corpus root without `.tla`, e.g. `tlc/expressions/let/foo`.
    pub id: String,
    pub module: String,
    /// The file as written, without its `(*--- recorded ... ---*)` block.
    pub source: String,
    pub metadata: Metadata,
}

/// A case run: the exit code and the normalised transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub exit: i32,
    pub transcript: String,
}

impl Case {
    pub fn is_expression(&self) -> bool {
        self.metadata.flags.iter().any(|f| f == "expression")
    }

    pub fn is_exit_only(&self) -> bool {
        self.metadata.flags.iter().any(|f| f == "exit-only")
    }

    /// `getContexts`.
    pub fn contexts(&self) -> Vec<Context> {
        if !self.is_expression() {
            return vec![Context::Model];
        }
        Context::EXPRESSION
            .into_iter()
            .filter(|c| self.metadata.contexts.as_ref().is_none_or(|names| names.iter().any(|n| n == c.name())))
            .collect()
    }

    /// The module text and cfg for `context` (`appendToModule`).
    pub fn files(&self, context: Context) -> (String, String) {
        let (appended, config) = context.wrapping();
        let config = format!("{config}{}", self.metadata.config.as_deref().unwrap_or(""));
        if context == Context::Model {
            return (self.source.clone(), config);
        }
        let end = last_module_end(&self.source).expect("the case has a module end line");
        (format!("{}{appended}\n{}", &self.source[..end], &self.source[end..]), config)
    }

    /// Writes the case's files into `dir` and returns TLC's arguments (after `tlc2.TLC`).
    pub fn materialize(&self, context: Context, dir: &Path) -> std::io::Result<Vec<String>> {
        let (module, config) = self.files(context);
        std::fs::write(dir.join(format!("{}.tla", self.module)), module)?;
        let cfg = format!("{}.cfg", self.module);
        std::fs::write(dir.join(&cfg), config)?;
        let mut args: Vec<String> = Vec::new();
        if !self.is_exit_only() {
            args.push("-tool".into());
        }
        args.extend(DETERMINISTIC_ARGS.iter().map(|a| a.to_string()));
        args.push("-metadir".into());
        args.push(dir.join(".meta").display().to_string());
        args.extend(self.metadata.args.iter().cloned());
        if !self.metadata.args.iter().any(|a| a == "-config") {
            args.push("-config".into());
            args.push(cfg);
        }
        args.push(format!("{}.tla", self.module));
        Ok(args)
    }

    /// `-Xss` for the JVM.
    pub fn stack(&self) -> String {
        format!("-Xss{}", self.metadata.stack.as_deref().unwrap_or("2m"))
    }
}

/// Byte offset of the last line that is `====` (four or more `=`, then blanks).
fn last_module_end(source: &str) -> Option<usize> {
    let mut offset = 0;
    let mut found = None;
    for line in source.split_inclusive('\n') {
        let text = line.trim_end_matches('\n');
        let eqs = text.len() - text.trim_start_matches('=').len();
        if eqs >= 4 && text[eqs..].chars().all(|c| c == ' ' || c == '\t') {
            found = Some(offset);
        }
        offset += line.len();
    }
    found
}

/// The corpus checkout (`$TLA_TS_CORPUS`, i.e. `packages/conformance/test`).
pub struct Corpus {
    root: PathBuf,
}

impl Corpus {
    pub fn open(root: &Path) -> Corpus {
        Corpus { root: root.to_path_buf() }
    }

    /// The cases under `dir` (e.g. `tlc/expressions`), in `listFiles` order: entries sorted
    /// at each level.
    pub fn cases(&self, dir: &str) -> std::io::Result<Vec<Case>> {
        let mut files = Vec::new();
        list_files(&self.root.join(dir), &mut files)?;
        let mut cases = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path)?;
            let rel = path.strip_prefix(&self.root).expect("under root").with_extension("");
            if let Some(case) = parse_case(&rel.to_string_lossy(), &text) {
                cases.push(case);
            }
        }
        Ok(cases)
    }
}

fn list_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?.map(|e| e.map(|e| e.path())).collect::<Result<_, _>>()?;
    entries.sort();
    for e in entries {
        if e.is_dir() {
            list_files(&e, out)?;
        } else if e.extension().is_some_and(|x| x == "tla") {
            out.push(e);
        }
    }
    Ok(())
}

/// `parseTest`: the `(*--- ... ---*)` header, and the source without the recorded block.
pub fn parse_case(id: &str, text: &str) -> Option<Case> {
    let header_body = text.strip_prefix("(*---\n")?;
    let header_end = header_body.find("\n---*)\n")?;
    let metadata = parse_header(&header_body[..header_end]);
    let source = match text.rfind("\n(*--- recorded\n") {
        Some(i) if text.ends_with("\n---*)\n") => &text[..i],
        _ => text,
    };
    let module = id.rsplit('/').next().unwrap_or(id).to_string();
    Some(Case { id: id.to_string(), module, source: format!("{}\n", source.trim_end()), metadata })
}

/// The YAML subset the headers use: `key: scalar`, `key: [a, b]`, `key: |` blocks. Keys this
/// harness does not read are skipped with their indented lines.
fn parse_header(text: &str) -> Metadata {
    let mut m = Metadata::default();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((key, value)) = line.split_once(':') else { continue };
        if key.starts_with(' ') {
            continue;
        }
        let value = value.trim();
        let mut block = String::new();
        if value == "|" || value == "|-" || value.is_empty() {
            while let Some(next) = lines.peek().filter(|l| l.starts_with(' ') || l.is_empty()) {
                block.push_str(next.trim_start_matches("  "));
                block.push('\n');
                lines.next();
            }
            if value == "|-" {
                block.truncate(block.trim_end_matches('\n').len());
            }
        }
        let list = || -> Vec<String> {
            value
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        match key {
            "outcome" => m.outcome = value.trim_matches('"').to_string(),
            "flags" => m.flags = list(),
            "contexts" => m.contexts = Some(list()),
            "args" => m.args = list(),
            "stack" => m.stack = Some(value.trim_matches('"').to_string()),
            "config" if value.starts_with('|') => m.config = Some(block),
            "config" => m.config = Some(value.trim_matches('"').to_string()),
            _ => {}
        }
    }
    m
}

/// Expected outcomes by `(case id, context)`, as written by `xtask conformance record`.
#[derive(Default)]
pub struct Expectations(pub BTreeMap<(String, Context), Outcome>);

impl Expectations {
    /// The file format: `== <id> <context>`, `exit <n>`, then each transcript line as
    /// `> <line>`.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for ((id, context), o) in &self.0 {
            let _ = writeln!(out, "== {id} {}\nexit {}", context.name(), o.exit);
            if !o.transcript.is_empty() {
                for line in o.transcript.split('\n') {
                    let _ = writeln!(out, "> {line}");
                }
            }
        }
        out
    }

    pub fn parse(text: &str) -> Expectations {
        let mut map = BTreeMap::new();
        let mut current: Option<((String, Context), i32, Vec<&str>)> = None;
        let mut finish = |entry: Option<((String, Context), i32, Vec<&str>)>| {
            if let Some((k, exit, lines)) = entry {
                map.insert(k, Outcome { exit, transcript: lines.join("\n") });
            }
        };
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("== ") {
                finish(current.take());
                let (id, ctx) = rest.rsplit_once(' ').expect("== <id> <context>");
                let context = Context::from_name(ctx).expect("context name");
                current = Some(((id.to_string(), context), 0, Vec::new()));
            } else if let Some(n) = line.strip_prefix("exit ") {
                current.as_mut().expect("entry").1 = n.parse().expect("exit code");
            } else if let Some(l) = line.strip_prefix("> ") {
                current.as_mut().expect("entry").2.push(l);
            }
        }
        finish(current);
        Expectations(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_contexts() {
        let text = "(*---\ndescription: \"d\"\nflags: [expression]\noutcome: success\nconfig: |\n  CONSTANT N = 3\n---*)\n---- MODULE m ----\nTest == TRUE\n====\n\n(*--- recorded\nall:\n  exit: 0\n---*)\n";
        let case = parse_case("tlc/expressions/x/m", text).expect("case");
        assert_eq!(case.module, "m");
        assert_eq!(case.metadata.config.as_deref(), Some("CONSTANT N = 3\n"));
        assert_eq!(case.contexts().len(), 5);
        let (module, cfg) = case.files(Context::Assume);
        assert!(module.ends_with("Test == TRUE\nASSUME Test\n====\n"), "{module}");
        assert_eq!(cfg, "CONSTANT N = 3\n");
        assert!(!case.source.contains("recorded"));
    }

    #[test]
    fn expectations_round_trip() {
        let mut e = Expectations::default();
        e.0.insert(("a/b".into(), Context::Assume), Outcome { exit: 10, transcript: "2105:1 x\n  y".into() });
        e.0.insert(("a/c".into(), Context::Assume), Outcome { exit: 0, transcript: String::new() });
        let again = Expectations::parse(&e.render());
        assert_eq!(again.0, e.0);
    }
}
