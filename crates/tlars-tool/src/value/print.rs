//! `toString` per representation and `Values.ppr`. The only way to turn a value into text.
//!
//! - SetEnum normalises first; Interval `lo..hi` (empty `{}`); Tuple `<<a, b>>`;
//! - FcnRcd: `<<>>` if empty; `[a |-> 1, ...]` if all keys are identifier-like strings;
//!   `<<...>>` for interval domain with lo = 1 or keys exactly 1..n; else `(k :> v @@ ...)`;
//! - Record `[a |-> 1, b |-> 2]` in token order; String with `\" \\ \t \n \f \r` escapes;
//! - lazy sets expand when `TLCGlobals.expand` (off with `-terse`): Subset if |S| < 7,
//!   SetOfFcns/Rcds/Tuples if size < `enumBound` (2000), Union/SetCup/SetPred always.

use super::{UserValue, Value};
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;

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
