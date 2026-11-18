//! `xtask jj2rs`: translate the JavaCC-generated SANY parser (`TLAplusParser.java`,
//! `TLAplusParserTokenManager.java`) to Rust.
//!
//! `jj2rs survey <File.java>` parses every class member and reports what it could not read.
//! `jj2rs emit` translates the parser, its lexer and SANY's parser support classes into
//! `crates/tlars-sany/src/parser/generated.rs` and lists the constructs it could not emit;
//! `jj2rs emit --check` fails if that file is not what emission would write.

mod ast;
mod emit;
mod externs;
mod lex;
mod parse;

use parse::MemberKind;
use std::collections::{BTreeMap, BTreeSet};
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

/// Translated classes: `(class, file under TLC's src, is_interface, implements)`. The
/// JavaCC-generated files are checked into TLC's tree (identical to `javacc` output), so
/// everything is read from the pinned source.
const TRANSLATED: &[(&str, &str, bool, &[&str])] = &[
    ("TLAplusParserConstants", "tla2sany/parser/TLAplusParserConstants.java", true, &[]),
    ("SyntaxTreeConstants", "tla2sany/st/SyntaxTreeConstants.java", true, &[]),
    ("Token", "tla2sany/parser/Token.java", false, &[]),
    ("ParseException", "tla2sany/parser/ParseException.java", false, &["TLAplusParserConstants"]),
    ("ParseExceptionExtended", "tla2sany/parser/ParseExceptionExtended.java", false, &[]),
    ("TokenMgrError", "tla2sany/parser/TokenMgrError.java", false, &[]),
    ("SimpleCharStream", "tla2sany/parser/SimpleCharStream.java", false, &[]),
    ("TLAplusParserTokenManager", "tla2sany/parser/TLAplusParserTokenManager.java", false, &["TLAplusParserConstants"]),
    ("SyntaxTreeNode", "tla2sany/parser/SyntaxTreeNode.java", false, &["SyntaxTreeConstants", "TLAplusParserConstants"]),
    ("Operator", "tla2sany/parser/Operator.java", false, &["TLAplusParserConstants"]),
    ("Operators", "tla2sany/parser/Operators.java", false, &["TLAplusParserConstants"]),
    ("OSelement", "tla2sany/parser/OSelement.java", false, &[]),
    ("OperatorStack", "tla2sany/parser/OperatorStack.java", false, &["SyntaxTreeConstants", "TLAplusParserConstants"]),
    ("ParseError", "tla2sany/parser/ParseError.java", false, &[]),
    ("ParseErrors", "tla2sany/parser/ParseErrors.java", false, &[]),
    ("Strings", "tla2sany/utilities/Strings.java", false, &[]),
    ("TLAplusParser", "tla2sany/parser/TLAplusParser.java", false, &["SyntaxTreeConstants", "TLAplusParserConstants"]),
];

/// Members implemented by hand in `tlars-sany/src/parser/support.rs`, by signature key.
/// They stay callable from translated code; only their emission is skipped.
const HAND_WRITTEN: &[(&str, &str)] = &[
    // Reads through a `java.io.InputStream`; the hand-written entry point decodes the
    // source and uses the `Reader` constructor.
    ("TLAplusParser", "<init>(SanyOutput,byte[])"),
    // `catch (Throwable t)` around `System.arraycopy`.
    ("SimpleCharStream", "ExpandBuff(boolean)"),
];

/// Raw generic fields and the element type their uses imply (every read casts to it).
const RAW_OVERRIDES: &[(&str, &str, &str)] = &[
    ("ParseErrors", "loe", "Vector<ParseError>"),
    ("TLAplusParser", "jj_expentries", "Vector<int[]>"),
];

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

pub(crate) fn emit(tlaplus_src: &Path, out: &Path, check: bool) -> Result<(), String> {
    let tlc_src = tlaplus_src.join("tlatools/org.lamport.tlatools/src");
    let mut parsed = Vec::new();
    for (class, file, is_interface, implements) in TRANSLATED {
        // Nested classes (`JJCalls`, `LookaheadSuccess`) become top-level classes.
        let mut members = Vec::new();
        let mut nested = Vec::new();
        for m in parse_class(&tlc_src.join(file), class)? {
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
    let (mut classes, extern_consts) = externs::classes();
    for c in extern_consts {
        emit::CONSTS.with(|s| s.borrow_mut().insert(c));
    }
    let overrides: BTreeMap<(String, String), ast::Type> =
        RAW_OVERRIDES.iter().map(|(c, f, t)| ((c.to_string(), f.to_string()), externs::parse_type(t))).collect();
    let known: BTreeSet<String> = classes.keys().cloned().chain(parsed.iter().map(|p| p.0.clone())).collect();
    let mut skipped = Vec::new();
    for (class, members, is_interface, _) in &parsed {
        emit::collect_consts(class, members, *is_interface);
    }
    for (class, members, is_interface, implements) in &parsed {
        let info = emit::class_info(class, members, implements.clone(), *is_interface, &known, &overrides, &mut skipped);
        classes.insert(class.clone(), info);
    }
    let mut code = String::from(
        "// @generated by `xtask jj2rs emit` from TLC's SANY parser sources. Do not edit.\n\n\
         #![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_mut, unused_variables, unused_assignments, unused_parens, unused_braces, unreachable_code, unused_labels, unused_attributes, dead_code, clippy::all)]\n\n\
         use super::java::*;\n\
         use super::support::*;\n\n",
    );
    let mut errors = Vec::new();
    for (class, members, ..) in &parsed {
        let hand: BTreeSet<String> = HAND_WRITTEN.iter().filter(|(c, _)| c == class).map(|(_, k)| k.to_string()).collect();
        emit::emit_class(&classes, class, members, &hand, &overrides, &mut code, &mut errors);
    }
    let resets: String = parsed
        .iter()
        .filter(|(c, ..)| classes[c].has_statics)
        .map(|(c, ..)| format!("    {c}::reset_statics();\n"))
        .collect();
    code.push_str(&format!(
        "/// Forgets every translated class's statics, as a fresh JVM would (a new session).\npub(crate) fn reset_all_statics() {{\n{resets}}}\n"
    ));
    if check {
        let current = std::fs::read_to_string(out).map_err(|e| format!("{}: {e}", out.display()))?;
        return if current == code { Ok(()) } else { Err(format!("{} is stale; run `xtask jj2rs emit`", out.display())) };
    }
    std::fs::write(out, code).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("skipped (signature mentions an untranslated class):");
    for s in &skipped {
        println!("  {s}");
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
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
