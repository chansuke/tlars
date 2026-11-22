//! `tlc2.module.TLC`.

use crate::ctx::Ctx;
use crate::error::EvalResult;
use crate::eval::{EvalControl, Ev};
use crate::state::StateRef;
use crate::value::Value;
use tlars_msg::Ec;
use tlars_sany::graph::ExprOrOpArg;

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

/// `SortSeq(s, cmp)` (`TLC.java:253-292`): a stable insertion sort that keeps every element,
/// duplicates included. The comparator is an operator, so its argument arrives unevaluated
/// and is resolved as `Sequences!SelectSeq` resolves its own (`evalImplOpArgKind`).
///
/// The first argument's error says "natural number" because the Java passes that string;
/// the message is the operator's, not a description of what a sequence is.
/// The `LazyFn` shape: an `@Evaluation`-style override that always answers, so never falls
/// back to the TLA+ definition.
pub(crate) fn sort_seq(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    sort_seq_value(ev, args, c, s0, s1, control).map(Some)
}

fn sort_seq_value(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    // `MethodValue.eval` receives its arguments already evaluated, so a failure in
    // evaluating them is outside its catch; everything from the method body on is inside it.
    let sval = ev.eval(args[0].expect_expr(), c, s0, s1, control)?;
    let cmp = ev.op_arg_value(args[1], c, control)?;
    let sorted = sort_seq_body(ev, args, c, &sval, cmp.as_ref());
    sorted.map_err(|e| super::wrap_method(ev, "TLC", "SortSeq", e))
}

fn sort_seq_body(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    sval: &Value,
    cmp: Option<&crate::eval::expr::OpArgValue>,
) -> EvalResult<Value> {
    let Some(seq) = sval.to_tuple(ev)? else {
        let shown = sval.ppr(ev)?;
        return Err(super::eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["first", "SortSeq", "natural number", &shown]));
    };
    let Some(cmp) = cmp else {
        // `cmp instanceof OpValue` fails, so what TLC prints is the value the argument
        // evaluated to; `LAMBDA x, y : 7` resolves to the literal 7.
        let shown = match ev.op_arg_as_value(args[1], c)? {
            Some(v) => v.ppr(ev)?,
            None => ev.op_arg_text(args[1], None)?,
        };
        return Err(super::eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "SortSeq", "operator", &shown]));
    };
    let elems = &seq.elems;
    if elems.is_empty() {
        // The converted tuple, not the argument: `SortSeq([i \in {} |-> 0], op)` is `<<>>`.
        return Ok(Value::Tuple(seq));
    }
    let mut sorted: Vec<Value> = Vec::with_capacity(elems.len());
    sorted.push(elems[0].clone());
    for elem in elems.iter().skip(1) {
        let mut j = sorted.len();
        sorted.push(elem.clone());
        while compare(ev, cmp, elem, &sorted[j - 1])? {
            sorted[j] = sorted[j - 1].clone();
            j -= 1;
            if j == 0 {
                break;
            }
        }
        sorted[j] = elem.clone();
    }
    Ok(Value::tuple(sorted))
}

/// `TLC.compare` (`TLC.java:294-303`): `fcmp.eval(args, EvalControl.Clear)` must answer a
/// `BoolValue`; the error prints the *result*, not the comparator.
fn compare(ev: &mut Ev<'_>, cmp: &crate::eval::expr::OpArgValue, x: &Value, y: &Value) -> EvalResult<bool> {
    let res = ev.apply_operator(cmp, &[x.clone(), y.clone()], EvalControl::CLEAR)?;
    match res {
        Value::Bool(b) => Ok(b),
        _ => {
            let shown = res.ppr(ev)?;
            Err(super::eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "SortSeq", "boolean function", &shown]))
        }
    }
}

/// `RandomElement(val)` (`TLC.java:381-438`): one element drawn with
/// `floor(nextDouble() * size)`. The function-, record- and tuple-set kinds are *not*
/// enumerated; they recurse into the component sets, so `[1..5 -> 1..100]` costs five draws
/// and never builds 100^5 functions.
///
/// An empty set reaches the draw with size 0, so `index` is 0 and the read past the end is
/// the JVM exception `MethodValue.eval` rewraps, not a checked error.
pub(crate) fn random_element(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    random_element_of(ev, &a[0])
}

fn not_a_finite_set(ev: &mut Ev<'_>, val: &Value) -> EvalResult<crate::error::EvalError> {
    let shown = val.ppr(ev)?;
    Ok(super::eval_exception(ev, Ec::TLC_MODULE_APPLYING_TO_WRONG_VALUE, &["RandomElement", "a finite set", &shown]))
}

fn random_element_of(ev: &mut Ev<'_>, val: &Value) -> EvalResult<Value> {
    match val {
        Value::SetOfFcns(sfv) => {
            val.normalize(ev)?;
            let domain = sfv.domain.clone();
            let range = sfv.range.clone();
            let Some(dom_set) = domain.to_set_enum(ev)? else {
                // The error names the whole `[S -> T]`, not the domain alone.
                return Err(not_a_finite_set(ev, val)?);
            };
            let domain = crate::value::order::normalized(ev, &dom_set)?.to_vec();
            let mut values = Vec::with_capacity(domain.len());
            for _ in 0..domain.len() {
                values.push(random_element_of(ev, &range)?);
            }
            Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, values, true)))
        }
        Value::SetOfRcds(srv) => {
            val.normalize(ev)?;
            let (names, sets) = (srv.names.to_vec(), srv.sets.to_vec());
            let mut values = Vec::with_capacity(sets.len());
            for set in &sets {
                values.push(random_element_of(ev, set)?);
            }
            Ok(Value::Record(Value::record_arc(names, values, true)))
        }
        Value::SetOfTuples(stv) => {
            val.normalize(ev)?;
            let sets = stv.sets.to_vec();
            let mut values = Vec::with_capacity(sets.len());
            for set in &sets {
                values.push(random_element_of(ev, set)?);
            }
            Ok(Value::tuple(values))
        }
        // `IntervalValue.randomElement()` indexes arithmetically, so the out-of-range read
        // of an empty interval is `IntervalValue.elementAt`'s own `Assert.fail`.
        Value::Interval(iv) => {
            let sz = iv.size(ev)?;
            let index = (ev.rng().next_double() * f64::from(sz)).floor() as i64;
            if index < 0 || index >= i64::from(sz) {
                let shown = val.ppr(ev)?;
                return Err(crate::error::EvalError::detailed(
                    format!("Attempted to retrieve out-of-bounds element from the interval value {shown}."),
                    None,
                ));
            }
            Ok(Value::Int(iv.lo + index as i32))
        }
        _ => {
            let Some(set) = val.to_set_enum(ev)? else {
                return Err(not_a_finite_set(ev, val)?);
            };
            // `normalize()` runs before the draw, so the index is into the sorted elements.
            let len = crate::value::order::normalized(ev, &set)?.len();
            let index = (ev.rng().next_double() * len as f64).floor() as i64;
            let elems = crate::value::order::normalized(ev, &set)?;
            match usize::try_from(index).ok().and_then(|i| elems.get(i)) {
                Some(v) => Ok(v.clone()),
                None => Err(crate::error::EvalError::index_out_of_bounds(index as usize, len)),
            }
        }
    }
}

pub(crate) fn assert(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if matches!(a[0], Value::Bool(true)) {
        return Ok(a[0].clone());
    }
    let shown = a[1].ppr(ev)?;
    Err(super::eval_exception(ev, Ec::TLC_VALUE_ASSERT_FAILED, &[&shown]))
}
