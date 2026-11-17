//! `xtask jj2rs`: translate the JavaCC-generated SANY parser (`TLAplusParser.java`,
//! `TLAplusParserTokenManager.java`) to Rust.
//!
//! `jj2rs survey <File.java>` parses every class member and reports what it could not read.
//! `jj2rs emit <generated dir>` translates the parser into
//! `crates/tlars-sany/src/parser/generated.rs` and lists the constructs it could not emit.

mod ast;
mod emit;
mod externs;
mod lex;
mod parse;

use parse::MemberKind;
use std::path::Path;

/// Finds `class <name>` and returns the token index of its body's `{`.
fn find_class_body(toks: &[lex::Token], name: &str) -> Option<usize> {
    let is_decl = |t: &lex::Tok| *t == lex::Tok::Ident("class".into()) || *t == lex::Tok::Ident("interface".into());
    let pos = toks.windows(2).position(|w| is_decl(&w[0].tok) && w[1].tok == lex::Tok::Ident(name.into()))?;
    toks[pos..].iter().position(|t| t.tok == lex::Tok::Punct("{")).map(|p| p + pos)
}

pub(crate) fn survey(file: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let class = file.file_stem().and_then(|s| s.to_str()).ok_or("bad file name")?;
    let toks = lex::lex(&src)?;
    let start = find_class_body(&toks, class).ok_or_else(|| format!("class {class} not found"))?;
    let mut p = parse::Parser::new(&toks[start..]);
    let members = p.class_body()?;
    let (mut methods, mut statements, mut unparsed) = (0, 0, Vec::new());
    let mut others: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for m in &members {
        match &m.kind {
            MemberKind::Method(method) | MemberKind::Constructor(method) => {
                methods += 1;
                statements += method.body.len();
            }
            MemberKind::Field { .. } => *others.entry("field").or_default() += 1,
            MemberKind::Class { .. } => *others.entry("class").or_default() += 1,
            MemberKind::StaticInit(_) => *others.entry("static init").or_default() += 1,
            MemberKind::Other(kind) => *others.entry(kind).or_default() += 1,
            MemberKind::Unparsed(e) => unparsed.push(format!("  {} (line {}): {e}", m.name, m.line)),
        }
    }
    println!(
        "{}: {methods} methods ({statements} top-level statements), other members {others:?}, {} unparsed",
        file.display(),
        unparsed.len()
    );
    for u in &unparsed {
        println!("{u}");
    }
    Ok(())
}

/// Classes translated from the generated parser directory, plus SANY's constants interface.
/// `(class, file, is_interface, implements)`.
/// `dir` is `gen` for the JavaCC output, otherwise a path under TLC's `src`.
const TRANSLATED: &[(&str, &str, bool, &[&str])] = &[
    ("TLAplusParserConstants", "gen/TLAplusParserConstants.java", true, &[]),
    ("SyntaxTreeConstants", "tla2sany/st/SyntaxTreeConstants.java", true, &[]),
    ("Token", "gen/Token.java", false, &[]),
    ("ParseException", "gen/ParseException.java", false, &["TLAplusParserConstants"]),
    ("TokenMgrError", "gen/TokenMgrError.java", false, &[]),
    ("SyntaxTreeNode", "tla2sany/parser/SyntaxTreeNode.java", false, &["SyntaxTreeConstants", "TLAplusParserConstants"]),
    ("Operator", "tla2sany/parser/Operator.java", false, &["TLAplusParserConstants"]),
    ("Operators", "tla2sany/parser/Operators.java", false, &["TLAplusParserConstants"]),
    ("OSelement", "tla2sany/parser/OSelement.java", false, &[]),
    ("OperatorStack", "tla2sany/parser/OperatorStack.java", false, &["SyntaxTreeConstants", "TLAplusParserConstants"]),
    ("ParseError", "tla2sany/parser/ParseError.java", false, &[]),
    ("ParseErrors", "tla2sany/parser/ParseErrors.java", false, &[]),
    ("TLAplusParser", "gen/TLAplusParser.java", false, &["SyntaxTreeConstants", "TLAplusParserConstants"]),
];

/// Members written by hand in `tlars-sany` instead of translated.
const HAND_WRITTEN: &[&str] = &["<init>", "ReInit", "parse", "enable_tracing", "disable_tracing"];

fn parse_class(path: &Path, class: &str) -> Result<Vec<parse::Member>, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let toks = lex::lex(&src)?;
    let start = find_class_body(&toks, class).ok_or_else(|| format!("class {class} not found in {}", path.display()))?;
    let members = parse::Parser::new(&toks[start..]).class_body()?;
    for m in &members {
        if let MemberKind::Unparsed(e) = &m.kind {
            return Err(format!("{class}.{}: {e}", m.name));
        }
    }
    Ok(members)
}

pub(crate) fn emit(generated_dir: &Path, tlaplus_src: &Path, out: &Path) -> Result<(), String> {
    let tlc_src = tlaplus_src.join("tlatools/org.lamport.tlatools/src");
    let mut parsed = Vec::new();
    for (class, file, is_interface, implements) in TRANSLATED {
        let path = match file.strip_prefix("gen/") {
            Some(f) => generated_dir.join(f),
            None => tlc_src.join(file),
        };
        // Nested classes (`JJCalls`, `LookaheadSuccess`) become top-level classes.
        let mut members = Vec::new();
        let mut nested = Vec::new();
        for m in parse_class(&path, class)? {
            match m.kind {
                MemberKind::Class { members: inner, .. } => nested.push((m.name, inner)),
                _ => members.push(m),
            }
        }
        parsed.push((class.to_string(), members, *is_interface, implements.iter().map(|s| s.to_string()).collect::<Vec<_>>()));
        for (n, ms) in nested {
            parsed.push((n, ms, false, Vec::new()));
        }
    }
    let mut classes = externs::classes();
    for (class, members, is_interface, implements) in &parsed {
        emit::collect_consts(class, members, *is_interface);
        classes.insert(class.clone(), emit::class_info(members, implements.clone(), *is_interface));
    }
    let mut code = String::from(
        "// @generated by `xtask jj2rs emit` from the JavaCC-generated SANY parser. Do not edit.\n\n\
         #![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_mut, unused_variables, unused_assignments, unused_parens, unused_braces, unreachable_code, dead_code, clippy::all)]\n\n\
         use super::java::*;\n\
         use super::support::*;\n\n",
    );
    let mut errors = Vec::new();
    for (class, members, ..) in &parsed {
        emit::emit_class(&classes, class, members, HAND_WRITTEN, &mut code, &mut errors);
    }
    std::fs::write(out, code).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for e in &errors {
        let key = e.split(": ").nth(1).unwrap_or(e).to_string();
        *counts.entry(key).or_default() += 1;
    }
    println!("wrote {} ({} emission errors, {} distinct)", out.display(), errors.len(), counts.len());
    for (k, n) in &counts {
        println!("  {n:4}  {k}");
    }
    Ok(())
}
