//! `toString` per representation and `Values.ppr`. The only way to turn a value into text.
//!
//! - SetEnum normalises first; Interval `lo..hi` (empty `{}`); Tuple `<<a, b>>`;
//! - FcnRcd: `<<>>` if empty; `[a |-> 1, ...]` if all keys are identifier-like strings;
//!   `<<...>>` for interval domain with lo = 1 or keys exactly 1..n; else `(k :> v @@ ...)`;
//! - Record `[a |-> 1, b |-> 2]` in token order; String with `\" \\ \t \n \f \r` escapes;
//! - lazy sets expand when `TLCGlobals.expand` (off with `-terse`): Subset if |S| < 7,
//!   SetOfFcns/Rcds/Tuples if size < `enumBound` (2000), Union/SetCup/SetPred always.

use super::{FcnRcdValue, IntervalValue, UserValue, Value};
use crate::error::{EvalError, EvalErrorKind, EvalResult};
use crate::eval::Ev;
use std::sync::Arc;

impl Value {
    /// `toString()` (flat).
    pub(crate) fn to_tlc_string(&self, ev: &mut Ev<'_>) -> EvalResult<String> {
        let interner = ev.tool().graph().interner().clone();
        Ok(match self {
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Str(a) => format!("\"{}\"", print_version(&interner.text(*a))),
            Value::Model(m) => interner.text(m.name).to_string(),
            Value::Interval(i) if i.lo <= i.hi => format!("{}..{}", i.lo, i.hi),
            Value::Interval(_) => "{}".to_string(),
            Value::SetEnum(set) => {
                // `toString` normalises first (`SetEnumValue.java:347-372`).
                let elems = super::order::normalized(ev, set)?.to_vec();
                let mut out = String::from("{");
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&e.to_tlc_string(ev)?);
                }
                out.push('}');
                out
            }
            Value::Tuple(t) => {
                let mut out = String::from("<<");
                for (i, e) in t.elems.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&e.to_tlc_string(ev)?);
                }
                out.push_str(">>");
                out
            }
            // The unsorted snapshot, so an unnormalised record prints in the order its
            // fields were written (`RecordValue.java:780-802`).
            Value::Record(r) => {
                let (names, values) = r.snapshot();
                let (names, values) = (names.to_vec(), values.to_vec());
                let mut out = String::from("[");
                for (i, (n, v)) in names.iter().zip(values.iter()).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&interner.text(*n));
                    out.push_str(" |-> ");
                    out.push_str(&v.to_tlc_string(ev)?);
                }
                out.push(']');
                out
            }
            Value::FcnRcd(f) => fcn_rcd_string(ev, f)?,
            Value::FcnLambda(f) => {
                if ev.tool().globals().expand {
                    match f.to_fcn_rcd(ev).and_then(|r| Value::FcnRcd(r).to_tlc_string(ev)) {
                        Ok(s) => return Ok(s),
                        // TLC swallows any Throwable here and falls back to the symbolic form
                        // (`FcnLambdaValue.java:785-791`). An unimplemented construct is not
                        // one of TLC's failures, so it surfaces rather than printing
                        // something TLC would not.
                        Err(e) if matches!(e.kind, EvalErrorKind::Unimplemented) => return Err(e),
                        Err(_) => {}
                    }
                }
                let params = f.params_string(ev)?;
                let body = ev.tool().graph().expr_tlc_text(f.body);
                format!("[{params} |-> <expression {body}>]")
            }
            Value::User(u) => match &**u {
                UserValue::Nat => "Nat".to_string(),
                UserValue::Int => "Int".to_string(),
                UserValue::String => "STRING".to_string(),
                _ => return Err(EvalError::unimplemented("toString of this special set (M2 unit 6)".into())),
            },
            _ => return Err(EvalError::unimplemented(format!("toString of {} (M2 unit 6)", self.kind_string()))),
        })
    }

    /// `Values.ppr(v.toString())`: flat string re-parsed and wrapped at `TLCGlobals` width.
    pub(crate) fn ppr(&self, ev: &mut Ev<'_>) -> EvalResult<String> {
        let flat = self.to_tlc_string(ev)?;
        ppr(ev, &flat)
    }
}

/// `Values.ppr(String)`. A value that fits the width prints flat; wrapping is not ported yet.
pub(crate) fn ppr(ev: &Ev<'_>, flat: &str) -> EvalResult<String> {
    if flat.chars().count() <= ev.tool().globals().width {
        Ok(flat.to_string())
    } else {
        Err(EvalError::unimplemented("Values.ppr of a value wider than the line (M2 unit 6)".into()))
    }
}

/// `FcnRcdValue.snapshot()` materialised: the function's own arrays when it is normalised, a
/// private copy otherwise. `isTuple` sorts the copy, which is why printing an unnormalised
/// function leaves it unnormalised (`FcnRcdValue.java:956-1004`).
struct Snapshot {
    intv: Option<IntervalValue>,
    domain: Vec<Value>,
    values: Vec<Value>,
    is_norm: bool,
}

/// `FcnRcdValue.toString`: `<<>>` when empty, a record when every domain element is an
/// identifier-like string, a sequence when the domain is exactly `1..n`, else `(d :> v @@ ...)`.
fn fcn_rcd_string(ev: &mut Ev<'_>, f: &Arc<FcnRcdValue>) -> EvalResult<String> {
    let (domain, values) = f.snapshot();
    let mut snap = Snapshot {
        intv: f.interval(),
        domain: domain.unwrap_or_default().to_vec(),
        values: values.to_vec(),
        is_norm: f.is_normalized(),
    };
    if snap.values.is_empty() {
        return Ok("<<>>".to_string());
    }
    if is_rcd(ev, &snap) {
        let mut out = String::from("[");
        for (i, (d, v)) in snap.domain.iter().zip(snap.values.iter()).enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let Value::Str(name) = d else { unreachable!("isRcd accepted only strings") };
            out.push_str(&ev.tool().graph().interner().text(*name));
            out.push_str(" |-> ");
            out.push_str(&v.to_tlc_string(ev)?);
        }
        out.push(']');
        return Ok(out);
    }
    if is_tuple(ev, &mut snap)? {
        let mut out = String::from("<<");
        for (i, v) in snap.values.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&v.to_tlc_string(ev)?);
        }
        out.push_str(">>");
        return Ok(out);
    }
    let domain = match snap.intv {
        Some(iv) => (i64::from(iv.lo)..=i64::from(iv.hi)).map(|n| Value::Int(n as i32)).collect(),
        None => snap.domain.clone(),
    };
    let mut out = String::from("(");
    for (i, (d, v)) in domain.iter().zip(snap.values.iter()).enumerate() {
        if i > 0 {
            out.push_str(" @@ ");
        }
        out.push_str(&d.to_tlc_string(ev)?);
        out.push_str(" :> ");
        out.push_str(&v.to_tlc_string(ev)?);
    }
    out.push(')');
    Ok(out)
}

/// `isRcd()` (`FcnRcdValue.java:926-935`).
fn is_rcd(ev: &Ev<'_>, snap: &Snapshot) -> bool {
    if snap.intv.is_some() {
        return false;
    }
    snap.domain.iter().all(|d| match d {
        Value::Str(name) => is_name(&ev.tool().graph().interner().text(*name)),
        _ => false,
    })
}

/// `isName(String)` (`FcnRcdValue.java:912-924`).
fn is_name(name: &str) -> bool {
    let mut has_letter = false;
    for ch in name.chars() {
        if ch == '_' {
            continue;
        }
        if !ch.is_alphanumeric() {
            return false;
        }
        has_letter = has_letter || ch.is_alphabetic();
    }
    has_letter && (name.encode_utf16().count() < 4 || (!name.starts_with("WF_") && !name.starts_with("SF_")))
}

/// `isTuple()` (`FcnRcdValue.java:937-953`), which normalises the snapshot it is given.
fn is_tuple(ev: &mut Ev<'_>, snap: &mut Snapshot) -> EvalResult<bool> {
    if let Some(iv) = snap.intv {
        return Ok(iv.lo == 1 || iv.is_empty());
    }
    if !snap.domain.iter().all(|d| matches!(d, Value::Int(_))) {
        return Ok(false);
    }
    if !snap.is_norm {
        super::order::fcn_sort(ev, &mut snap.domain, &mut snap.values)?;
    }
    Ok(snap.domain.iter().enumerate().all(|(i, d)| matches!(d, Value::Int(n) if i64::from(*n) == i as i64 + 1)))
}

/// `StringValue.PrintVersion`.
fn print_version(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}
