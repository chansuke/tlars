//! `toString` per representation and `Values.ppr`. The only way to turn a value into text.
//!
//! - SetEnum normalises first; Interval `lo..hi` (empty `{}`); Tuple `<<a, b>>`;
//! - FcnRcd: `<<>>` if empty; `[a |-> 1, ...]` if all keys are identifier-like strings;
//!   `<<...>>` for interval domain with lo = 1 or keys exactly 1..n; else `(k :> v @@ ...)`;
//! - Record `[a |-> 1, b |-> 2]` in token order; String with `\" \\ \t \n \f \r` escapes;
//! - lazy sets expand when `TLCGlobals.expand` (off with `-terse`): Subset if |S| < 7,
//!   SetOfFcns/Rcds/Tuples if size < `enumBound` (2000), Union/SetCup/SetPred always.

use super::Value;
use crate::eval::Ev;
use crate::error::EvalResult;

impl Value {
    /// `toString()` (flat).
    pub(crate) fn to_tlc_string(&self, ev: &mut Ev<'_>) -> EvalResult<String> {
        todo!()
    }

    /// `Values.ppr(v.toString())`: flat string re-parsed and wrapped at `TLCGlobals` width.
    pub(crate) fn ppr(&self, ev: &mut Ev<'_>) -> EvalResult<String> {
        let flat = self.to_tlc_string(ev)?;
        Ok(tlars_java::pretty_print(&flat, ev.tool().globals().width))
    }
}
