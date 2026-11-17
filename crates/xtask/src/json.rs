//! Minimal JSON reader for the tlaplus/Examples manifests and benchmark results. Parses the
//! full grammar; numbers are kept as `f64`.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(m) => m.get(key),
            _ => None,
        }
    }
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }
    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(n) => Some(*n),
            _ => None,
        }
    }
    pub(crate) fn as_array(&self) -> &[Json] {
        match self {
            Json::Array(a) => a,
            _ => &[],
        }
    }
}

pub(crate) fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("trailing data at byte {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.s[self.i..].starts_with(lit.as_bytes()) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.i))
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.s.get(self.i) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Json::String),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            _ => self.err("expected a value"),
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut m = BTreeMap::new();
        self.ws();
        if self.eat("}") {
            return Ok(Json::Object(m));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            if !self.eat(":") {
                return self.err("expected ':'");
            }
            let v = self.value()?;
            m.insert(k, v);
            self.ws();
            if self.eat("}") {
                return Ok(Json::Object(m));
            }
            if !self.eat(",") {
                return self.err("expected ',' or '}'");
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.i += 1;
        let mut a = Vec::new();
        self.ws();
        if self.eat("]") {
            return Ok(Json::Array(a));
        }
        loop {
            a.push(self.value()?);
            self.ws();
            if self.eat("]") {
                return Ok(Json::Array(a));
            }
            if !self.eat(",") {
                return self.err("expected ',' or ']'");
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if !self.eat("\"") {
            return self.err("expected a string");
        }
        let mut out = String::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return self.err("unterminated string") };
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(&e) = self.s.get(self.i) else { return self.err("bad escape") };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4).unwrap_or_default()).unwrap_or("");
                            let code = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            self.i += 4;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return self.err("bad escape"),
                    }
                }
                _ => {
                    // Copy the whole UTF-8 sequence.
                    let start = self.i - 1;
                    while self.i < self.s.len() && (self.s[self.i] & 0xC0) == 0x80 {
                        self.i += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?);
                }
            }
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
        text.parse().map(Json::Number).map_err(|e| format!("{text}: {e}"))
    }
}

/// A JSON string literal for `s`.
pub(crate) fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_values() {
        let v = parse(r#"{"a": [1, -2.5e1, "x\"é"], "b": {"c": true, "d": null}}"#).unwrap();
        assert_eq!(v.get("a").unwrap().as_array()[1].as_f64(), Some(-25.0));
        assert_eq!(v.get("a").unwrap().as_array()[2].as_str(), Some("x\"é"));
        assert_eq!(v.get("b").unwrap().get("d"), Some(&Json::Null));
        assert_eq!(parse(&quote("a\"b\n")).unwrap().as_str(), Some("a\"b\n"));
    }
}
