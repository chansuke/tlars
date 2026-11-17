//! `xtask jj2rs`: translate the JavaCC-generated SANY parser (`TLAplusParser.java`,
//! `TLAplusParserTokenManager.java`) to Rust.
//!
//! Status: survey only. `jj2rs survey <File.java>` parses every class member with the Java
//! subset parser and reports what it could not read, which is the tractability measure for
//! the M1 spike. Emission comes next.

// The AST is complete for the emitter (next step); the survey reads only part of it.
#[allow(dead_code)]
mod ast;
mod lex;
mod parse;

use parse::MemberKind;
use std::path::Path;

/// Finds `class <name>` and returns the token index of its body's `{`.
fn find_class_body(toks: &[lex::Token], name: &str) -> Option<usize> {
    let pos = toks.windows(2).position(|w| w[0].tok == lex::Tok::Ident("class".into()) && w[1].tok == lex::Tok::Ident(name.into()))?;
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
            MemberKind::Method(method) => {
                methods += 1;
                statements += method.body.len();
            }
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
