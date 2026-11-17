//! Tokens of the Java subset that JavaCC 4.0 generates (and TLC's semantic actions use).

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    Ident(String),
    /// Integer literal as written (`0x...`, `L` suffix kept).
    Int(String),
    Char(String),
    Str(String),
    /// Operators and separators.
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub(crate) tok: Tok,
    pub(crate) line: usize,
}

/// Longest first, so `>>>=` wins over `>>`.
const PUNCTS: &[&str] = &[
    ">>>=", "<<=", ">>=", ">>>", "...", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=",
    "|=", "^=", "<<", ">>", "->", "::", "{", "}", "(", ")", "[", "]", ";", ",", ".", "@", "=", ">", "<", "!", "~", "?",
    ":", "+", "-", "*", "/", "&", "|", "^", "%",
];

pub(crate) fn lex(src: &str) -> Result<Vec<Token>, String> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut line = 1;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if src[i..].starts_with("/*") {
            let end = src[i + 2..].find("*/").ok_or_else(|| format!("line {line}: unterminated comment"))? + i + 4;
            line += src[i..end].matches('\n').count();
            i = end;
        } else if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            out.push(Token { tok: Tok::Ident(src[start..i].to_string()), line });
        } else if c.is_ascii_digit() {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Token { tok: Tok::Int(src[start..i].to_string()), line });
        } else if c == b'"' || c == b'\'' {
            let start = i;
            i += 1;
            while i < b.len() && b[i] != c {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            let body = src[start + 1..i - 1].to_string();
            out.push(Token { tok: if c == b'"' { Tok::Str(body) } else { Tok::Char(body) }, line });
        } else if let Some(p) = PUNCTS.iter().find(|p| src[i..].starts_with(**p)) {
            out.push(Token { tok: Tok::Punct(p), line });
            i += p.len();
        } else {
            return Err(format!("line {line}: unexpected character {:?}", c as char));
        }
    }
    out.push(Token { tok: Tok::Eof, line });
    Ok(out)
}
