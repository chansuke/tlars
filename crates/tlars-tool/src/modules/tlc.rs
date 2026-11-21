//! `tlc2.module.TLC`.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::value::Value;
use tlars_msg::Ec;

/// `deepCopy`, `deepNormalize`, then `Values.ppr(toStringUnchecked())`.
fn shown(ev: &mut Ev<'_>, v: &Value) -> EvalResult<String> {
    v.deep_normalize(ev)?;
    let flat = v.to_tlc_string_unchecked(ev)?;
    Ok(crate::value::print::ppr(ev, &flat))
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
    let text = a[0].to_tlc_string_unchecked(ev)?;
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

/// `Permutations(s)` (`TLC.java:304-379`): every bijection of the normalised `s` onto
/// itself, as unnormalised function records in the order the index-array walk produces
/// them. An empty set gives the set holding the empty function.
pub(crate) fn permutations(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(set) = a[0].to_set_enum(ev)? else {
        let shown = a[0].ppr(ev)?;
        return Err(super::eval_exception(ev, Ec::TLC_MODULE_APPLYING_TO_WRONG_VALUE, &["Permutations", "a finite set", &shown]));
    };
    let domain = crate::value::order::normalized(ev, &set)?.to_vec();
    let len = domain.len();
    if len == 0 {
        let empty = Value::FcnRcd(Value::fcn_rcd_arc(Vec::new(), Vec::new(), true));
        return Ok(Value::SetEnum(crate::value::set_enum_arc(vec![empty], true)));
    }
    let mut idx_array: Vec<usize> = (0..len).collect();
    let mut in_use = vec![true; len];
    let mut fcns: Vec<Value> = Vec::new();
    loop {
        let vals: Vec<Value> = idx_array.iter().map(|&i| domain[i].clone()).collect();
        fcns.push(Value::FcnRcd(Value::fcn_rcd_arc(domain.clone(), vals, true)));
        let mut i = len;
        let mut done = true;
        for j in (0..len).rev() {
            i = j;
            let mut found = false;
            for k in idx_array[j] + 1..len {
                if !in_use[k] {
                    in_use[k] = true;
                    in_use[idx_array[j]] = false;
                    idx_array[j] = k;
                    found = true;
                    break;
                }
            }
            if found {
                done = false;
                break;
            }
            if j == 0 {
                break;
            }
            in_use[idx_array[j]] = false;
        }
        if done {
            break;
        }
        for slot in &mut idx_array[i + 1..] {
            for (k, used) in in_use.iter_mut().enumerate() {
                if !*used {
                    *used = true;
                    *slot = k;
                    break;
                }
            }
        }
    }
    Ok(Value::SetEnum(crate::value::set_enum_arc(fcns, false)))
}

pub(crate) fn assert(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if matches!(a[0], Value::Bool(true)) {
        return Ok(a[0].clone());
    }
    let shown = a[1].ppr(ev)?;
    Err(super::eval_exception(ev, Ec::TLC_VALUE_ASSERT_FAILED, &[&shown]))
}
