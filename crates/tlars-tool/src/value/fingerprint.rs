//! FP64 per-kind encoding (`fingerPrint(long)` of every value class): the only place that
//! decides which bytes a value contributes.
//!
//! - Bool: `BOOL, 't'|'f'`; Int: `INT, int32`; String: `STRING, len, low byte per char`;
//!   ModelValue: `MODEL, tok`.
//! - SetEnum: `SETENUM, size, elems` in normalised order; Interval hashes as a SetEnum.
//! - FcnRcd: `FCNRCD, n, (dom, val)*` (interval domain writes `INT, i`); Tuple as FCNRCD with
//!   keys `1..n`; Record as FCNRCD with STRING keys in token order.
//! - Lazy sets / FcnLambda: converted first.
//! So equal values fingerprint equally whatever their representation, and `<<>>` differs
//! from `{}`.

use super::Value;
use crate::eval::Ev;
use crate::error::EvalResult;
use crate::fp64::Fp;

impl Value {
    pub(crate) fn fingerprint(&self, ev: &mut Ev<'_>, fp: Fp) -> EvalResult<Fp> {
        todo!()
    }
}
