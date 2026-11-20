//! `tlc2.module.TLC`.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::value::Value;
use tlars_msg::Ec;

/// `deepCopy`, `deepNormalize`, then `Values.ppr(toStringUnchecked())`.
fn shown(ev: &mut Ev<'_>, v: &Value) -> EvalResult<String> {
    v.deep_normalize(ev)?;
    v.ppr(ev)
}

/// Prints `v1  v2` to `ToolIO.out` and returns `v2` (`TLC.java:63-78`).
pub(crate) fn print(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let line = format!("{}  {}", shown(ev, &a[0])?, shown(ev, &a[1])?);
    ev.out().raw(&format!("{line}\n"));
    Ok(a[1].clone())
}

/// Prints `v1` to `ToolIO.out` and returns TRUE (`TLC.java:86-101`).
pub(crate) fn print_t(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let line = shown(ev, &a[0])?;
    ev.out().raw(&format!("{line}\n"));
    Ok(Value::Bool(true))
}

/// `new StringValue(v.toStringUnchecked())`.
pub(crate) fn to_string(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let text = a[0].to_tlc_string(ev)?;
    Ok(Value::Str(ev.tool().graph().interner().intern(&text)))
}

/// `d :> e`: the one-point function, built normalised (`TLC.java` `MakeFcn`).
pub(crate) fn make_fcn(_ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(vec![a[0].clone()], vec![a[1].clone()], true)))
}

/// `f1 @@ f2`: f1's pairs, then f2's pairs whose key f1 lacks, left unnormalised in that
/// order (`TLC.java` `CombineFcn`).
pub(crate) fn combine_fcn(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let first = pairs_of(ev, &a[0], "first")?;
    let second = pairs_of(ev, &a[1], "second")?;
    let mut domain: Vec<Value> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    for (k, v) in first {
        domain.push(k);
        values.push(v);
    }
    // Only the first function's keys are scanned, so f2's own repeats all survive.
    let len1 = domain.len();
    for (k, v) in second {
        let mut found = false;
        for seen in domain[..len1].iter() {
            if k.tlc_equals(ev, seen)? {
                found = true;
                break;
            }
        }
        if !found {
            domain.push(k);
            values.push(v);
        }
    }
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, values, false)))
}

/// One argument of `@@` as `(key, value)` pairs in the order its snapshot holds them.
fn pairs_of(ev: &mut Ev<'_>, f: &Value, which: &str) -> EvalResult<Vec<(Value, Value)>> {
    let Some(fcn) = f.to_fcn_rcd(ev)? else {
        let shown = f.ppr(ev)?;
        return Err(super::eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &[which, "@@", "function", &shown]));
    };
    let (domain, values) = fcn.snapshot();
    let keys: Vec<Value> = match (domain, fcn.interval()) {
        (Some(d), _) => d.to_vec(),
        (None, Some(iv)) => (i64::from(iv.lo)..=i64::from(iv.hi)).map(|n| Value::Int(n as i32)).collect(),
        (None, None) => unreachable!("a function has an explicit domain or an interval"),
    };
    Ok(keys.into_iter().zip(values.iter().cloned()).collect())
}

pub(crate) fn assert(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if matches!(a[0], Value::Bool(true)) {
        return Ok(a[0].clone());
    }
    let shown = a[1].ppr(ev)?;
    Err(super::eval_exception(ev, Ec::TLC_VALUE_ASSERT_FAILED, &[&shown]))
}
