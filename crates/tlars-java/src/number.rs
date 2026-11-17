//! Number formatting TLC prints: `Double.toString` (JDK 19+ algorithm), the two-digit
//! `BigDecimal` reciprocal of the collision line, and `DecimalFormat("###,###.###")`.

/// A positive decimal `digits × 10^(exp + 1 - digits.len())`, i.e. `d.ddd × 10^exp`.
struct Decimal {
    digits: String,
    exp: i32,
}

/// Parses Rust's `{:e}` / `{:.Ne}` output of a finite, non-negative value.
fn parse_sci(s: &str) -> Decimal {
    let (mantissa, exp) = s.split_once('e').expect("scientific notation");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let trimmed = digits.trim_end_matches('0');
    let digits = if trimmed.is_empty() { "0".to_string() } else { trimmed.to_string() };
    Decimal { digits, exp: exp.parse().expect("exponent") }
}

fn to_f64(d: &Decimal) -> f64 {
    format!("{}e{}", d.digits, d.exp + 1 - d.digits.len() as i32).parse().expect("decimal")
}

/// Move an `n`-significant-digit decimal one unit in its last place up or down.
fn step(d: &Decimal, n: usize, up: bool) -> Decimal {
    let mut m: u64 = format!("{:0<n$}", d.digits).parse().expect("digits");
    let mut exp = d.exp;
    let low = 10u64.pow(n as u32 - 1);
    if up {
        m += 1;
        if m == low * 10 {
            m = low;
            exp += 1;
        }
    } else {
        m -= 1;
        if m < low {
            m = low * 10 - 1;
            exp -= 1;
        }
    }
    let m = m.to_string();
    parse_sci(&format!("{}.{}e{exp}", &m[..1], &m[1..]))
}

/// `Double.toString(v)` as in JDK 19 and later (Giulietti's algorithm, JDK-4511638).
///
/// Let `n` be the length of the shortest decimal that rounds to `v`, but at least 2 (the
/// JDK lets length-2 decimals compete with length 1). Among length-`n` decimals that round
/// to `v`, the one closest to `v` wins, ties to an even last digit. Then render plainly for
/// `10^-3 <= |v| < 10^7`, otherwise as `d.dddE±n`.
pub fn double_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    let shortest = parse_sci(&format!("{a:e}"));
    let n = shortest.digits.len().max(2);
    // The exactly rounded length-n decimal (ties to even) is the closest one. If it does
    // not round back to `a`, it lies on the far side of `a` from `shortest`, so the best
    // candidate is its neighbour toward `shortest` when that rounds back, else `shortest`.
    let nearest = parse_sci(&format!("{a:.prec$e}", prec = n - 1));
    let nearest_f = to_f64(&nearest);
    let chosen = if nearest_f == a {
        nearest
    } else {
        let toward = step(&nearest, n, nearest_f < a);
        if to_f64(&toward) == a { toward } else { shortest }
    };
    format!("{sign}{}", render(&chosen))
}

fn render(d: &Decimal) -> String {
    let digits = d.digits.as_bytes();
    if (-3..7).contains(&d.exp) {
        if d.exp < 0 {
            let zeros = "0".repeat((-d.exp - 1) as usize);
            return format!("0.{zeros}{}", d.digits);
        }
        let int_len = d.exp as usize + 1;
        if digits.len() <= int_len {
            return format!("{}{}.0", d.digits, "0".repeat(int_len - digits.len()));
        }
        return format!("{}.{}", &d.digits[..int_len], &d.digits[int_len..]);
    }
    let frac = if digits.len() > 1 { &d.digits[1..] } else { "0" };
    format!("{}.{}E{}", &d.digits[..1], frac, d.exp)
}

/// `BigDecimal.valueOf(1d).divide(BigDecimal.valueOf(n), new MathContext(2)).doubleValue()`
/// (`AbstractChecker.reportSuccess`): `1/n` rounded half-up to two significant digits.
pub fn reciprocal_two_digits(n: i64) -> f64 {
    assert!(n > 0, "TLC only divides by a positive minimum distance");
    let n = n as u128;
    // Smallest k with 10^k / n >= 10, so the quotient has exactly two digits.
    let mut k: u32 = 0;
    while 10u128.pow(k) / n < 10 {
        k += 1;
    }
    let num = 10u128.pow(k);
    let mut q = num / n;
    let r = num % n;
    let mut k = k as i32;
    if 2 * r >= n {
        q += 1;
        if q == 100 {
            q = 10;
            k -= 1;
        }
    }
    format!("{q}e{}", -k).parse().expect("decimal")
}

/// `new DecimalFormat("###,###.###").format(n)` in the default (en) locale.
pub fn decimal_format_grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
