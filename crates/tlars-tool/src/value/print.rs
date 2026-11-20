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
                UserValue::Seq(range) => format!("Seq({})", range.to_tlc_string(ev)?),
                _ => return Err(EvalError::unimplemented("toString of this special set (M2 unit 6)".into())),
            },
            // `S \cup T`, `S \cap T`, `S \ T`: each operand parenthesised, since every set
            // class is an EnumerableValue (`EnumerableValue.java:250-257`).
            Value::SetCup(s) | Value::SetCap(s) | Value::SetDiff(s) => {
                if let Some(expanded) = expanded(ev, self)? {
                    return Ok(expanded);
                }
                let op = match self {
                    Value::SetCup(_) => " \\cup ",
                    Value::SetCap(_) => " \\cap ",
                    _ => " \\ ",
                };
                let (a, b) = (cup_cap_diff_operand(ev, &s.left)?, cup_cap_diff_operand(ev, &s.right)?);
                format!("{a}{op}{b}")
            }
            // `SUBSET S` expands only for a base set of fewer than 7 elements
            // (`SubsetValue.java:317-341`).
            Value::Subset(s) => {
                let unlazy = match ev.tool().globals().expand {
                    false => false,
                    true => match s.base.size(ev) {
                        Ok(n) => n < 7,
                        Err(e) if matches!(e.kind, EvalErrorKind::Unimplemented) => return Err(e),
                        Err(_) => false,
                    },
                };
                if unlazy {
                    let set = self.to_set_enum(ev)?.expect("a lazy set converts");
                    return Value::SetEnum(set).to_tlc_string(ev);
                }
                // `..` binds tighter than SUBSET, and the three binary set operators share
                // its precedence, so those operands are parenthesised.
                let base = s.base.to_tlc_string(ev)?;
                match is_parenthesized_subset(&s.base) {
                    true => format!("SUBSET ({base})"),
                    false => format!("SUBSET {base}"),
                }
            }
            Value::KSubset(_) => return Err(EvalError::unimplemented("toString of a k-subset (M5)".into())),
            Value::Union(u) => {
                if ev.tool().globals().expand {
                    let set = self.to_set_enum(ev)?.expect("a lazy set converts");
                    return Value::SetEnum(set).to_tlc_string(ev);
                }
                format!("UNION({})", u.set.to_tlc_string(ev)?)
            }
            // The three product-shaped classes expand when their cardinality is below
            // `enumBound`, counted with the same saturating loop TLC uses.
            Value::SetOfFcns(s) => {
                if let Some(expanded) = expanded_below_bound(ev, self, &[s.domain.clone(), s.range.clone()])? {
                    return Ok(expanded);
                }
                format!("[{} -> {}]", s.domain.to_tlc_string(ev)?, s.range.to_tlc_string(ev)?)
            }
            Value::SetOfRcds(s) => {
                if let Some(expanded) = expanded_below_bound(ev, self, &s.sets)? {
                    return Ok(expanded);
                }
                let mut out = String::from("[");
                for (i, (n, v)) in s.names.iter().zip(s.sets.iter()).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&interner.text(*n));
                    out.push_str(": ");
                    out.push_str(&v.to_tlc_string(ev)?);
                }
                out.push(']');
                out
            }
            Value::SetOfTuples(s) => {
                if let Some(expanded) = expanded_below_bound(ev, self, &s.sets)? {
                    return Ok(expanded);
                }
                let mut out = String::from("(");
                for (i, v) in s.sets.iter().enumerate() {
                    if i > 0 {
                        out.push_str(" \\X ");
                    }
                    out.push_str(&v.to_tlc_string(ev)?);
                }
                out.push(')');
                out
            }
            Value::SetPred(p) => {
                if let Some(expanded) = expanded(ev, self)? {
                    return Ok(expanded);
                }
                let mut out = String::from("{");
                for (i, &v) in p.vars.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&interner.text(ev.tool().graph().param(v).name));
                }
                let (set, pred) = (p.inner.to_tlc_string(ev)?, ev.tool().graph().expr_tlc_text(p.pred));
                out.push_str(&format!(" \\in {set} : <expression {pred}> }}"));
                out
            }
            _ => return Err(EvalError::unimplemented(format!("toString of {} (M2 unit 6)", self.kind_string()))),
        })
    }

    /// `Values.ppr(v.toString())`: flat string re-parsed and wrapped at `TLCGlobals` width.
    pub(crate) fn ppr(&self, ev: &mut Ev<'_>) -> EvalResult<String> {
        let flat = self.to_tlc_string(ev)?;
        ppr(ev, &flat)
    }
}

/// The expanded form a lazy set prints under `TLCGlobals.expand`, or `None` when it stays
/// symbolic. TLC swallows any `Throwable` from the expansion and falls back to the symbolic
/// form; a construct tlars does not implement is not one of TLC's failures, so it surfaces.
fn expanded(ev: &mut Ev<'_>, v: &Value) -> EvalResult<Option<String>> {
    if !ev.tool().globals().expand {
        return Ok(None);
    }
    match v.to_set_enum(ev).and_then(|s| Value::SetEnum(s.expect("a lazy set converts")).to_tlc_string(ev)) {
        Ok(s) => Ok(Some(s)),
        Err(e) if matches!(e.kind, EvalErrorKind::Unimplemented) => Err(e),
        Err(_) => Ok(None),
    }
}

/// `[D -> R]`, `[f : S]` and `S \X T` expand only when their cardinality is below
/// `TLCGlobals.enumBound`. TLC multiplies the constituents' sizes in a `long` that stops
/// growing at `Integer.MAX_VALUE`, and an empty constituent makes the product 0
/// (`SetOfTuplesValue.java:430-451`).
fn expanded_below_bound(ev: &mut Ev<'_>, v: &Value, constituents: &[Value]) -> EvalResult<Option<String>> {
    if !ev.tool().globals().expand {
        return Ok(None);
    }
    match product_size(ev, v, constituents) {
        Ok(sz) if sz < i64::from(ev.tool().globals().enum_bound) => expanded(ev, v),
        Ok(_) => Ok(None),
        Err(e) if matches!(e.kind, EvalErrorKind::Unimplemented) => Err(e),
        Err(_) => Ok(None),
    }
}

/// The saturating product the three classes' `toString` computes. `[D -> R]` is the range
/// multiplied once per element of the domain, and has one element for an empty domain
/// (`SetOfFcnsValue.java:415-437`).
fn product_size(ev: &mut Ev<'_>, v: &Value, constituents: &[Value]) -> EvalResult<i64> {
    if let Value::SetOfFcns(s) = v {
        if s.domain.is_empty(ev)? {
            return Ok(1);
        }
        let (dsz, rsz) = (s.domain.size(ev)?, s.range.size(ev)?);
        let mut sz: i64 = 1;
        for _ in 0..dsz {
            if sz > i64::from(i32::MAX) {
                break;
            }
            sz *= i64::from(rsz);
        }
        return Ok(sz);
    }
    let mut sz: i64 = 1;
    for c in constituents {
        let csz = c.size(ev)?;
        if csz == 0 {
            return Ok(0);
        }
        if sz <= i64::from(i32::MAX) {
            sz *= i64::from(csz);
        }
    }
    Ok(sz)
}

/// `Value.toStringForCupCapDiff`: every set class is an `EnumerableValue`, which
/// parenthesises itself as an operand of `\cup`, `\cap` and `\`.
fn cup_cap_diff_operand(ev: &mut Ev<'_>, v: &Value) -> EvalResult<String> {
    let text = v.to_tlc_string(ev)?;
    Ok(if v.is_enumerable() { format!("({text})") } else { text })
}

/// `instanceof ParenthesizedSubset`: the classes whose terse form needs brackets after
/// `SUBSET`.
fn is_parenthesized_subset(v: &Value) -> bool {
    matches!(
        v,
        Value::Interval(_) | Value::Subset(_) | Value::KSubset(_) | Value::SetCup(_) | Value::SetCap(_) | Value::SetDiff(_)
    )
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
