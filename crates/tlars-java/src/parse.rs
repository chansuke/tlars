//! `Integer.parseInt`, `Long.parseLong` and `Double.parseDouble` as TLC's CLI and config
//! parser use them, with Java's `NumberFormatException` message.

/// `NumberFormatException.forInputString(s, 10)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberFormatError {
    pub input: String,
}

impl NumberFormatError {
    pub fn message(&self) -> String {
        format!("For input string: \"{}\"", self.input)
    }
}

/// An optional sign then ASCII digits, as Java accepts them. (Java also accepts other
/// Unicode decimal digits; TLC's inputs never contain them.)
fn parse_signed<T: std::str::FromStr>(s: &str) -> Result<T, NumberFormatError> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(NumberFormatError { input: s.to_string() });
    }
    s.parse().map_err(|_| NumberFormatError { input: s.to_string() })
}

/// `Integer.parseInt(s)`.
pub fn parse_int(s: &str) -> Result<i32, NumberFormatError> {
    parse_signed(s)
}

/// `Long.parseLong(s)`.
pub fn parse_long(s: &str) -> Result<i64, NumberFormatError> {
    parse_signed(s)
}

/// `Double.parseDouble(s)`: surrounding whitespace (chars `<= ' '`) is ignored and a
/// trailing `f`/`F`/`d`/`D` type suffix is allowed. Hexadecimal floats are not supported.
pub fn parse_double(s: &str) -> Result<f64, NumberFormatError> {
    let t = s.trim_matches(|c: char| c <= ' ');
    let err = || NumberFormatError { input: s.to_string() };
    let unsigned = t.strip_prefix(['+', '-']).unwrap_or(t);
    match unsigned {
        "NaN" | "Infinity" => return t.replace("Infinity", "inf").parse().map_err(|_| err()),
        _ => {}
    }
    let body = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    let numeric = body.strip_prefix(['+', '-']).unwrap_or(body);
    let valid = !numeric.is_empty()
        && numeric.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        && numeric.bytes().any(|b| b.is_ascii_digit());
    if !valid {
        return Err(err());
    }
    body.parse().map_err(|_| err())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_accepts_what_java_accepts() {
        assert_eq!(parse_int("+7"), Ok(7));
        assert_eq!(parse_int("-0"), Ok(0));
        assert_eq!(parse_int("2147483647"), Ok(i32::MAX));
        assert!(parse_int("2147483648").is_err());
        assert!(parse_int(" 1").is_err());
        assert!(parse_int("").is_err());
        assert!(parse_int("-").is_err());
        assert_eq!(parse_int("abc").unwrap_err().message(), "For input string: \"abc\"");
    }

    #[test]
    fn double_accepts_suffixes_and_whitespace() {
        assert_eq!(parse_double(" 0.5d "), Ok(0.5));
        assert_eq!(parse_double("1e3"), Ok(1000.0));
        assert_eq!(parse_double("-Infinity"), Ok(f64::NEG_INFINITY));
        assert!(parse_double("inf").is_err());
        assert!(parse_double("x").is_err());
    }
}
