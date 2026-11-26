//! `transcript.ts` `normalizeTranscript`: framed `-tool` output to one `code:severity body`
//! entry per message and `| text` per unframed line, with volatile messages, Java stack
//! frames, paths, dates, durations, build versions and object identities taken out.

use regex::Regex;
use std::sync::LazyLock;

/// JavaScript's `\w`, `\d` and `\b` are ASCII (spelled out below); its `\s` is Unicode, as
/// Rust's is.
static MARKER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@!@!@(?:STARTMSG ([0-9]+):([0-9]+)|ENDMSG [0-9]+) @!@!@").expect("regex"));
static VOLATILE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(Parsing file |Semantic processing of module |Linting of module )").expect("regex"));
static STACK_FRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s+at [0-9A-Za-z_.$<>]+\(.*\)$").expect("regex"));
static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}").expect("regex"));
static BUILD_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)Version [0-9]{4}\.[0-9]{2}\.[0-9]{2}\.[0-9]{6}(?-u:\b)").expect("regex"));
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)(in) ([0-9]+h )?([0-9]+min )?[0-9]+(s|ms)(?-u:\b)").expect("regex"));
static OBJECT_IDENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u:\b)([0-9A-Za-z_.$]+)@[0-9a-f]{5,8}(?-u:\b)").expect("regex"));
static GENERATED_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)(__DebuggerExpr__|__DebuggerModule__|_PossibleModule_|neg_prop_)[0-9]+").expect("regex"));
static INSTALL_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"install \|->\s+"[^"]*""#).expect("regex"));

const VOLATILE_CODES: &[u32] = &[2185, 2186, 2187, 2188, 2189, 2200, 2219, 2220, 2262, 2271, 2401];
const STATE_SPACE_CODES: &[u32] = &[2190, 2192, 2193, 2194, 2199, 2267, 2268];

fn clean_line(line: &str, directories: &[String]) -> String {
    let mut text = line.to_string();
    for d in directories {
        text = text.replace(&format!("{d}/"), "");
    }
    let text = TIMESTAMP.replace_all(&text, "<date>");
    let text = BUILD_VERSION.replace_all(&text, "Version <build>");
    let text = DURATION.replace_all(&text, "$1 <time>");
    let text = GENERATED_NAME.replace_all(&text, "$1<id>");
    OBJECT_IDENTITY.replace_all(&text, "$1@<id>").into_owned()
}

struct Open {
    code: u32,
    severity: u32,
    lines: Vec<String>,
    slot: usize,
}

/// `directory` is where the case ran (its path is removed from the text);
/// `keeps_state_space` is false in the expression contexts.
pub fn normalize(output: &str, directory: &str, keeps_state_space: bool) -> String {
    let directories = [directory.to_string(), directory.strip_prefix("/private").unwrap_or(directory).to_string()];
    let mut kept: Vec<Option<String>> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let add_text = |text: &str, kept: &mut Vec<Option<String>>, open: &mut Vec<Open>| {
        if let Some(message) = open.last_mut() {
            message.lines.push(text.to_string());
            return;
        }
        if !text.trim().is_empty() && !VOLATILE_LINE.is_match(text) && !STACK_FRAME.is_match(text) {
            kept.push(Some(format!("| {}", clean_line(text, &directories))));
        }
    };
    let close = |kept: &mut Vec<Option<String>>, open: &mut Vec<Open>| {
        let Some(m) = open.pop() else { return };
        let dropped = VOLATILE_CODES.contains(&m.code) || (!keeps_state_space && STATE_SPACE_CODES.contains(&m.code));
        let body: Vec<String> = m.lines.iter().filter(|l| !STACK_FRAME.is_match(l)).map(|l| clean_line(l, &directories)).collect();
        let body = body.join("\n");
        let body = INSTALL_FIELD.replace_all(&body, r#"install |-> "<install>""#);
        kept[m.slot] = if dropped { None } else { Some(format!("{}:{} {}", m.code, m.severity, body.trim_end())) };
    };
    for line in output.split('\n') {
        let mut offset = 0;
        let mut any = false;
        for caps in MARKER.captures_iter(line) {
            let whole = caps.get(0).expect("match");
            any = true;
            let before = &line[offset..whole.start()];
            if !before.is_empty() {
                add_text(before, &mut kept, &mut open);
            }
            match (caps.get(1), caps.get(2)) {
                (Some(code), Some(severity)) => {
                    open.push(Open {
                        code: code.as_str().parse().expect("code"),
                        severity: severity.as_str().parse().expect("severity"),
                        lines: Vec::new(),
                        slot: kept.len(),
                    });
                    kept.push(None);
                }
                _ => close(&mut kept, &mut open),
            }
            offset = whole.end();
        }
        let rest = &line[offset..];
        if !any || !rest.is_empty() {
            add_text(rest, &mut kept, &mut open);
        }
    }
    kept.into_iter().flatten().collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn frames_drop_and_clean() {
        let raw = "@!@!@STARTMSG 2262:0 @!@!@\nTLC2 Version 2026.01.02.123456\n@!@!@ENDMSG 2262 @!@!@\n\
                   Parsing file /tmp/x/M.tla\n\
                   @!@!@STARTMSG 2105:1 @!@!@\nEvaluating assumption in /tmp/x/M.tla failed.\n@!@!@ENDMSG 2105 @!@!@\n\
                   stray text\n";
        assert_eq!(normalize(raw, "/tmp/x", false), "2105:1 Evaluating assumption in M.tla failed.\n| stray text");
    }

    #[test]
    fn nested_and_inline_markers() {
        let raw = "@!@!@STARTMSG 1000:1 @!@!@ outer @!@!@STARTMSG 2179:0 @!@!@inner\n@!@!@ENDMSG 2179 @!@!@\n@!@!@ENDMSG 1000 @!@!@\n";
        assert_eq!(normalize(raw, "/d", false), "1000:1  outer\n2179:0 inner");
    }
}
