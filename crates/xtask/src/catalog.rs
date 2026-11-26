//! `tlars-msg`'s message catalog from `EC.java` (codes, exit statuses) and `MP.java`
//! (`getMessage0` templates).
//!
//! A template whose `case` body is only `b.append(<string literals>);` statements becomes
//! `Template::Static`. Anything else (loops, `now()`, conditionals) becomes
//! `Template::Computed(crate::computed::<name>)`, hand-written in `tlars-msg/src/computed.rs`;
//! a missing function fails the build, so the two cannot drift silently.

use std::collections::BTreeMap;
use std::path::Path;

pub(crate) struct Catalog {
    /// `(name, code)` in EC.java declaration order.
    pub(crate) codes: Vec<(String, i64)>,
    /// Name -> `ExitStatus` variant (Rust name); absent = `Error` (255).
    pub(crate) exit: BTreeMap<String, &'static str>,
    pub(crate) templates: BTreeMap<String, TemplateSrc>,
}

pub(crate) enum TemplateSrc {
    Static(String),
    Computed,
}

pub(crate) fn read(tlc: &Path) -> Result<Catalog, String> {
    let ec = read_file(&tlc.join("tlc2/output/EC.java"))?;
    let mp = read_file(&tlc.join("tlc2/output/MP.java"))?;
    let (decls, exit_section) = ec.split_once("class ExitStatus").ok_or("EC.java: no ExitStatus class")?;
    let codes = parse_codes(&strip_comments(decls))?;
    let exit = parse_exit_map(exit_section)?;
    let templates = parse_templates(&mp)?;
    for name in exit.keys().chain(templates.keys()) {
        if !codes.iter().any(|(n, _)| n == name) {
            return Err(format!("{name} is used in EC.ExitStatus or MP but not declared in EC"));
        }
    }
    Ok(Catalog { codes, exit, templates })
}

fn read_file(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
}

/// `public static final int NAME = <int> | <OTHER> + <int>;` (modifiers in any order).
fn parse_codes(src: &str) -> Result<Vec<(String, i64)>, String> {
    let mut codes: Vec<(String, i64)> = Vec::new();
    for line in src.lines() {
        let Some((lhs, expr)) = line.split_once('=') else { continue };
        let words: Vec<&str> = lhs.split_whitespace().collect();
        let Some((name, modifiers)) = words.split_last() else { continue };
        let mut sorted = modifiers.to_vec();
        sorted.sort_unstable();
        if sorted != ["final", "int", "public", "static"] {
            continue;
        }
        let expr = expr.trim().trim_end_matches(';').trim();
        let value = match expr.split_once('+') {
            Some((base, inc)) => {
                let base = base.trim();
                let base_value =
                    codes.iter().find(|(n, _)| n == base).map(|(_, v)| *v).ok_or_else(|| format!("EC.java: unknown base {base}"))?;
                base_value + inc.trim().parse::<i64>().map_err(|e| format!("EC.java: {line}: {e}"))?
            }
            None => expr.parse::<i64>().map_err(|e| format!("EC.java: {line}: {e}"))?,
        };
        codes.push((name.trim().to_string(), value));
    }
    Ok(codes)
}

/// `errorConstantToExitStatus`: groups of `case NAME:` labels followed by `return X;`.
fn parse_exit_map(src: &str) -> Result<BTreeMap<String, &'static str>, String> {
    let body = src.split_once("errorConstantToExitStatus").ok_or("EC.java: no errorConstantToExitStatus")?.1;
    let body = body.split_once("default:").ok_or("EC.java: no default in exit map")?.0;
    let mut map = BTreeMap::new();
    let mut pending: Vec<String> = Vec::new();
    for line in strip_comments(body).lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("case ").and_then(|l| l.strip_suffix(':')) {
            pending.push(name.trim().to_string());
        } else if let Some(ret) = line.strip_prefix("return ").and_then(|l| l.strip_suffix(';')) {
            let status = match ret.trim() {
                "SUCCESS" => "Success",
                "FAILURE_LIVENESS_EVAL" => "FailureLivenessEval",
                "FAILURE_SPEC_EVAL" => "FailureSpecEval",
                "FAILURE_SAFETY_EVAL" => "FailureSafetyEval",
                "VIOLATION_SAFETY" => "ViolationSafety",
                "VIOLATION_LIVENESS" => "ViolationLiveness",
                "VIOLATION_DEADLOCK" => "ViolationDeadlock",
                "VIOLATION_ASSUMPTION" => "ViolationAssumption",
                "VIOLATION_ASSERT" => "ViolationAssert",
                "ERROR_CONFIG_PARSE" => "ErrorConfigParse",
                "ERROR_SPEC_PARSE" => "ErrorSpecParse",
                other => return Err(format!("EC.java: unknown exit status {other}")),
            };
            for name in pending.drain(..) {
                map.insert(name, status);
            }
        }
    }
    Ok(map)
}

/// Removes `//` and `/* */` comments outside string literals.
pub(crate) fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                while let Some(d) = chars.next() {
                    out.push(d);
                    if d == '\\' {
                        if let Some(e) = chars.next() {
                            out.push(e);
                        }
                    } else if d == '"' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for d in chars.by_ref() {
                    if d == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for d in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// `getMessage0`'s switch: each `case EC.NAME:` group and its body.
fn parse_templates(mp: &str) -> Result<BTreeMap<String, TemplateSrc>, String> {
    let start = mp.find("private static String getMessage0").ok_or("MP.java: no getMessage0")?;
    let body = &mp[start..];
    let body = &body[body.find("switch (messageCode)").ok_or("MP.java: no switch")?..];
    let body = &body[..body.find("replaceString(b, parameters)").ok_or("MP.java: no replaceString")?];
    let body = strip_comments(body);

    let mut templates = BTreeMap::new();
    let mut labels: Vec<String> = Vec::new();
    let mut text = String::new();
    let flush = |labels: &mut Vec<String>, text: &mut String, templates: &mut BTreeMap<String, TemplateSrc>| {
        if !text.trim().is_empty() && !labels.is_empty() {
            let template = static_template(text).map_or(TemplateSrc::Computed, TemplateSrc::Static);
            for l in labels.drain(..) {
                let t = match &template {
                    TemplateSrc::Static(s) => TemplateSrc::Static(s.clone()),
                    TemplateSrc::Computed => TemplateSrc::Computed,
                };
                templates.insert(l, t);
            }
            text.clear();
        }
    };
    for line in body.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_prefix("case EC.").and_then(|l| l.strip_suffix(':')) {
            flush(&mut labels, &mut text, &mut templates);
            labels.push(name.trim().to_string());
        } else if t == "default:" {
            flush(&mut labels, &mut text, &mut templates);
            labels.clear();
            text.clear();
        } else if !labels.is_empty() {
            text.push_str(line);
            text.push('\n');
        }
    }
    flush(&mut labels, &mut text, &mut templates);
    Ok(templates)
}

/// `Some(text)` iff the body is `b.append(<literal> [+ <literal>]*);`* then `break;`.
fn static_template(body: &str) -> Option<String> {
    let mut rest = body.trim().strip_suffix("break;")?.trim();
    let mut out = String::new();
    while !rest.is_empty() {
        rest = rest.strip_prefix("b.append(")?.trim_start();
        loop {
            let (lit, after) = string_literal(rest)?;
            out.push_str(&lit);
            rest = after.trim_start();
            match rest.strip_prefix('+') {
                Some(r) => rest = r.trim_start(),
                None => break,
            }
        }
        rest = rest.strip_prefix(')')?.trim_start().strip_prefix(';')?.trim_start();
    }
    Some(out)
}

/// A Java string literal at the start of `s`: its value and the remaining text.
fn string_literal(s: &str) -> Option<(String, &str)> {
    let mut chars = s.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 2..])),
            '\\' => {
                let (_, e) = chars.next()?;
                out.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '"' => '"',
                    '\'' => '\'',
                    '\\' => '\\',
                    'u' => {
                        let hex: String = (0..4).filter_map(|_| chars.next().map(|(_, h)| h)).collect();
                        char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
                    }
                    _ => return None,
                });
            }
            c => out.push(c),
        }
    }
    None
}

pub(crate) fn render(cat: &Catalog) -> Result<String, String> {
    let mut seen = BTreeMap::new();
    for (name, code) in &cat.codes {
        if i32::try_from(*code).is_err() {
            return Err(format!("EC.{name} = {code} does not fit i32"));
        }
        if let Some(other) = seen.insert(*code, name) {
            return Err(format!("EC.{name} and EC.{other} share code {code}"));
        }
    }
    let mut o = String::from(
        "// @generated by `xtask gen-fidelity` from tlc2/output/EC.java and MP.java. Do not edit.\n\n\
         use crate::{ExitStatus, Template};\n\n\
         /// An `EC` constant. The discriminant is TLC's integer code.\n\
         #[allow(non_camel_case_types)]\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]\n\
         #[repr(i32)]\n\
         pub enum Ec {\n",
    );
    for (name, code) in &cat.codes {
        o.push_str(&format!("    {name} = {code},\n"));
    }
    o.push_str("}\n\nimpl Ec {\n    /// Every code, in EC.java declaration order.\n    pub const ALL: &[Ec] = &[\n");
    for (name, _) in &cat.codes {
        o.push_str(&format!("        Ec::{name},\n"));
    }
    o.push_str("    ];\n\n    pub const fn code(self) -> i32 {\n        self as i32\n    }\n\n");
    o.push_str("    pub fn from_code(code: i32) -> Option<Ec> {\n        Ec::ALL.iter().copied().find(|ec| ec.code() == code)\n    }\n\n");
    o.push_str("    pub const fn name(self) -> &'static str {\n        match self {\n");
    for (name, _) in &cat.codes {
        o.push_str(&format!("            Ec::{name} => \"{name}\",\n"));
    }
    o.push_str("        }\n    }\n\n    /// `EC.ExitStatus.errorConstantToExitStatus`.\n    pub const fn exit_status(self) -> ExitStatus {\n        match self {\n");
    for (name, status) in &cat.exit {
        o.push_str(&format!("            Ec::{name} => ExitStatus::{status},\n"));
    }
    o.push_str("            _ => ExitStatus::Error,\n        }\n    }\n\n");
    o.push_str("    /// The `getMessage0` case for this code, before `%i%` replacement.\n    pub fn template(self) -> Template {\n        match self {\n");
    for (name, t) in &cat.templates {
        match t {
            TemplateSrc::Static(s) => o.push_str(&format!("            Ec::{name} => Template::Static({s:?}),\n")),
            TemplateSrc::Computed => {
                o.push_str(&format!("            Ec::{name} => Template::Computed(crate::computed::{}),\n", name.to_ascii_lowercase()))
            }
        }
    }
    o.push_str("            _ => Template::Default,\n        }\n    }\n}\n");
    Ok(o)
}

/// Names whose template must be hand-written in `computed.rs`.
pub(crate) fn computed_names(cat: &Catalog) -> Vec<&str> {
    cat.templates.iter().filter(|(_, t)| matches!(t, TemplateSrc::Computed)).map(|(n, _)| n.as_str()).collect()
}

/// SANY's `ErrorCode` enum (`tla2sany/semantic/ErrorCode.java`): standard value, name and
/// severity, in declaration order. `EC.<NAME>` values resolve through the TLC catalog.
pub(crate) fn sany_error_codes(tlc: &Path, cat: &Catalog) -> Result<String, String> {
    let src = strip_comments(&read_file(&tlc.join("tla2sany/semantic/ErrorCode.java"))?);
    let body = src.split_once("public enum ErrorCode {").ok_or("ErrorCode.java: no enum")?.1;
    let body = body.split_once(';').ok_or("ErrorCode.java: no end of constants")?.0;
    let mut rows = Vec::new();
    for entry in body.split("),") {
        let entry = entry.trim().trim_end_matches(')');
        if entry.is_empty() {
            continue;
        }
        let (name, args) = entry.split_once('(').ok_or_else(|| format!("ErrorCode.java: {entry}"))?;
        let mut args = args.split(',').map(str::trim);
        let value = args.next().ok_or_else(|| format!("ErrorCode.java: {entry}"))?;
        let value = match value.strip_prefix("EC.") {
            Some(ec) => cat.codes.iter().find(|(n, _)| n == ec).map(|(_, v)| *v).ok_or_else(|| format!("unknown EC.{ec}"))?,
            None => value.parse::<i64>().map_err(|e| format!("ErrorCode.java: {entry}: {e}"))?,
        };
        let level = match args.next() {
            Some("ErrorLevel.ERROR") => "Error",
            Some("ErrorLevel.WARNING") => "Warning",
            other => return Err(format!("ErrorCode.java: {entry}: level {other:?}")),
        };
        rows.push((name.trim().to_string(), value, level));
    }
    let mut o = String::from(
        "// @generated by `xtask gen-fidelity` from tla2sany/semantic/ErrorCode.java. Do not edit.\n\n\
         use crate::{ErrorLevel, SanyErrorCode};\n\n\
         /// Every SANY error code, in declaration order.\n\
         pub(crate) static SANY_ERROR_CODES: &[SanyErrorCode] = &[\n",
    );
    for (name, value, level) in &rows {
        o.push_str(&format!("    SanyErrorCode {{ name: \"{name}\", value: {value}, level: ErrorLevel::{level} }},\n"));
    }
    o.push_str("];\n");
    Ok(o)
}
