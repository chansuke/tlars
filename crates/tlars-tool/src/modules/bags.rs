//! `tlc2.module.Bags`. `SubBag` has no Java override and stays an ordinary TLA+ definition.
//!
//! A bag is a function record, so every operator here is a walk over one or two
//! `(domain, values)` pairs read from [`to_fcn_rcd`]. What differs between them — which
//! argument is checked, in which order, with which message, and where a cast or a
//! dereference is left unguarded — is the whole of the module, so each one walks its own
//! pair instead of sharing an engine.

use crate::ctx::Ctx;
use crate::error::{EvalError, EvalResult};
use crate::eval::{EvalControl, Ev};
use crate::modules::{eval_exception, wrap_method};
use crate::state::StateRef;
use crate::value::{FcnRcdValue, Value};
use std::sync::Arc;
use tlars_msg::Ec;
use tlars_sany::graph::ExprOrOpArg;

/// `Bags.toFcnRcd(b)` (`Bags.java:52-57`): `b.toFcnRcd()` and then `snapshot()`, which
/// hands back a private copy while the value is unnormalised so a worker sorting the same
/// function cannot be caught half-way. The copy decides more than concurrency here:
/// `BagToSet` normalises what it is handed, and on a copy that leaves the argument's own
/// order alone.
fn to_fcn_rcd(ev: &mut Ev<'_>, b: &Value) -> EvalResult<Option<Arc<FcnRcdValue>>> {
    let Some(fcn) = b.to_fcn_rcd(ev)? else { return Ok(None) };
    if fcn.is_normalized() {
        return Ok(Some(fcn));
    }
    let (domain, values) = fcn.snapshot();
    let domain = domain.expect("an interval domain is normalised").to_vec();
    Ok(Some(Value::fcn_rcd_arc(domain, values.to_vec(), false)))
}

/// A snapshot's `(getDomainAsValues(), values)`: the domain materialised out of the interval
/// where the function holds one (`FcnRcdValue.java:590-596`).
fn domain_and_values(fcn: &FcnRcdValue) -> (Vec<Value>, Vec<Value>) {
    let (domain, values) = fcn.snapshot();
    let domain = match (domain, fcn.interval()) {
        (Some(d), _) => d.to_vec(),
        (None, Some(iv)) => (i64::from(iv.lo)..=i64::from(iv.hi)).map(|n| Value::Int(n as i32)).collect(),
        (None, None) => unreachable!("a function has an explicit domain or an interval"),
    };
    (domain, values.to_vec())
}

/// `addCounts` (`Bags.java:41-50`): `Math.addExact`, with the overflow named `n1+n2`.
fn add_counts(ev: &Ev<'_>, n1: i32, n2: i32) -> EvalResult<i32> {
    n1.checked_add(n2).ok_or_else(|| eval_exception(ev, Ec::TLC_MODULE_OVERFLOW, &[&format!("{n1}+{n2}")]))
}

/// `(IntValue) v`, which `BagDiff`, `SqSubseteq`, `BagUnion` and `BagOfAll` do without
/// checking the class first, so a non-integer count is a `ClassCastException`.
fn int_cast(v: &Value) -> EvalResult<i32> {
    match v {
        Value::Int(n) => Ok(*n),
        _ => Err(EvalError::class_cast(v.java_class(), "IntValue")),
    }
}

/// `MethodValue.eval`'s `NullPointerException` arm (`MethodValue.java:152-154`): a method
/// that dereferences a conversion it did not check fails with the message the JDK builds
/// out of the bytecode, which names the local slot or the parameter position and not the
/// source variable. The arm raises an `EvalException` where every other throwable goes
/// through `Assert.fail` and becomes a `TLCRuntimeException`, so the two print the same
/// 2154 but are caught differently: only the runtime exception is caught on the way out of
/// init-state generation, where it is what records the failing state
/// (`ModelChecker.java:390-397`).
fn null_pointer(ev: &Ev<'_>, tla_name: &str, cause: &str) -> EvalError {
    let signature = super::NativeId::of_module("Bags", tla_name).expect("a registered native").op().signature;
    eval_exception(ev, Ec::TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE, &[signature, cause])
}

/// What `BagIn` and `CopiesIn` fail on: `fcn.values`, read before the domain.
const NULL_FCN_VALUES: &str = r#"Cannot read field "values" because "<local2>" is null"#;

/// What `BagCup` fails on: `b.toFcnRcd()` inside `toFcnRcd`, for either argument.
const NULL_TO_FCN_RCD: &str = r#"Cannot invoke "tlc2.value.impl.Value.toFcnRcd()" because "<parameter1>" is null"#;

/// `IsABag`'s loop over the counts (`Bags.java:70-78`).
fn counts_are_positive(values: &[Value]) -> bool {
    values.iter().all(|v| matches!(v, Value::Int(n) if *n > 0))
}

/// `FcnRcdValue.EmptyFcn`: the empty function, built normalised with an explicit domain.
fn empty_fcn() -> Value {
    Value::FcnRcd(Value::fcn_rcd_arc(Vec::new(), Vec::new(), true))
}

pub(crate) fn empty_bag(_ev: &mut Ev<'_>, _a: &[Value]) -> EvalResult<Value> {
    Ok(empty_fcn())
}

/// `IsABag` (`Bags.java:62-81`): a value that does not convert is not a bag rather than an
/// error, which is how `BagCup` reaches it with a value it has already converted.
pub(crate) fn is_a_bag(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(fcn) = to_fcn_rcd(ev, &a[0])? else { return Ok(Value::Bool(false)) };
    let (_, values) = fcn.snapshot();
    Ok(Value::Bool(counts_are_positive(values)))
}

/// `BagCardinality` (`Bags.java:83-112`): the two failures name the same operator with
/// different nouns, and a count of zero is as wrong as a string.
pub(crate) fn bag_cardinality(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(fcn) = to_fcn_rcd(ev, &a[0])? else {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_APPLYING_TO_WRONG_VALUE, &["BagCardinality", "a function with a finite domain", &shown]));
    };
    let (_, values) = fcn.snapshot();
    let mut num = 0;
    for v in values {
        match v {
            Value::Int(cnt) if *cnt > 0 => num = add_counts(ev, num, *cnt)?,
            _ => {
                let shown = a[0].ppr(ev)?;
                return Err(eval_exception(ev, Ec::TLC_MODULE_APPLYING_TO_WRONG_VALUE, &["BagCardinality", "a bag", &shown]));
            }
        }
    }
    Ok(Value::Int(num))
}

/// `BagIn` (`Bags.java:114-134`): the conversion is dereferenced without a null check, so a
/// second argument that is not a function is a `NullPointerException` and never the
/// argument error below. `fcn.values` is read before the domain, which is the field the JDK
/// names.
pub(crate) fn bag_in(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(fcn) = to_fcn_rcd(ev, &a[1])? else { return Err(null_pointer(ev, "BagIn", NULL_FCN_VALUES)) };
    let (domain, values) = domain_and_values(&fcn);
    for (i, key) in domain.iter().enumerate() {
        if a[0].tlc_equals(ev, key)? {
            return match &values[i] {
                Value::Int(n) => Ok(Value::Bool(*n > 0)),
                _ => {
                    let shown = a[1].ppr(ev)?;
                    Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "BagIn", "bag", &shown]))
                }
            };
        }
    }
    Ok(Value::Bool(false))
}

/// `CopiesIn` (`Bags.java:136-156`): `BagIn`'s walk, answering the count itself, so a
/// negative count in the domain is reported rather than rejected.
pub(crate) fn copies_in(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(fcn) = to_fcn_rcd(ev, &a[1])? else { return Err(null_pointer(ev, "CopiesIn", NULL_FCN_VALUES)) };
    let (domain, values) = domain_and_values(&fcn);
    for (i, key) in domain.iter().enumerate() {
        if a[0].tlc_equals(ev, key)? {
            return match &values[i] {
                Value::Int(_) => Ok(values[i].clone()),
                _ => {
                    let shown = a[1].ppr(ev)?;
                    Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "CopiesIn", "bag", &shown]))
                }
            };
        }
    }
    Ok(Value::Int(0))
}

/// `(+)` (`Bags.java:158-213`): the counts of the first bag, each one raised by the second
/// bag's count for the same key, then the second bag's own keys. Both arguments are
/// converted before either is checked, and the check runs `IsABag` on the *converted*
/// value, so a non-function argument raises the `NullPointerException` from inside
/// `toFcnRcd` and the argument error only ever reports a function whose counts are wrong.
pub(crate) fn bag_cup(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let fcn1 = to_fcn_rcd(ev, &a[0])?;
    let fcn2 = to_fcn_rcd(ev, &a[1])?;
    let Some(fcn1) = fcn1 else { return Err(null_pointer(ev, "\\oplus", NULL_TO_FCN_RCD)) };
    let (domain1, values1) = domain_and_values(&fcn1);
    if !counts_are_positive(&values1) {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["first", "(+)", "bag", &shown]));
    }
    let Some(fcn2) = fcn2 else { return Err(null_pointer(ev, "\\oplus", NULL_TO_FCN_RCD)) };
    let (domain2, values2) = domain_and_values(&fcn2);
    if !counts_are_positive(&values2) {
        let shown = a[1].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "(+)", "bag", &shown]));
    }
    let mut domain = domain1.clone();
    let mut values = values1.clone();
    for (i, key) in domain2.iter().enumerate() {
        let mut found = false;
        // Only the first bag's keys are scanned, so a key the second bag repeats is added
        // once per repeat.
        for j in 0..domain1.len() {
            if key.tlc_equals(ev, &domain1[j])? {
                let (v1, v2) = (int_cast(&values1[j])?, int_cast(&values2[i])?);
                values[j] = Value::Int(add_counts(ev, v1, v2)?);
                found = true;
                break;
            }
        }
        if !found {
            domain.push(key.clone());
            values.push(values2[i].clone());
        }
    }
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, values, false)))
}

/// `(-)` (`Bags.java:215-262`): the first bag's counts less the second's, keeping the keys
/// whose remainder is positive. Neither bag is checked, so the counts are cast unchecked
/// and the subtraction wraps as Java's `int` does: `2147483647 (-) -1` turns negative and
/// drops its key.
pub(crate) fn bag_diff(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let fcn1 = to_fcn_rcd(ev, &a[0])?;
    let fcn2 = to_fcn_rcd(ev, &a[1])?;
    let Some(fcn1) = fcn1 else {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["first", "(-)", "bag", &shown]));
    };
    let Some(fcn2) = fcn2 else {
        let shown = a[1].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "(-)", "bag", &shown]));
    };
    let (domain1, values1) = domain_and_values(&fcn1);
    let (domain2, values2) = domain_and_values(&fcn2);
    let mut domain = Vec::new();
    let mut values = Vec::new();
    for i in 0..domain1.len() {
        let mut v1 = int_cast(&values1[i])?;
        for j in 0..domain2.len() {
            if domain1[i].tlc_equals(ev, &domain2[j])? {
                v1 = v1.wrapping_sub(int_cast(&values2[j])?);
                break;
            }
        }
        if v1 > 0 {
            domain.push(domain1[i].clone());
            values.push(Value::Int(v1));
        }
    }
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, values, fcn1.is_normalized())))
}

/// `BagUnion` (`Bags.java:264-336`): the set is normalised first, so a set holding one bag
/// twice is that bag and not the bag added to itself, and a set of one element is answered
/// unchecked. From two elements on, the counts of equal keys are added.
pub(crate) fn bag_union(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(set) = a[0].to_set_enum(ev)? else {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_APPLYING_TO_WRONG_VALUE, &["BagUnion", "a finite enumerable set", &shown]));
    };
    let elems = crate::value::order::normalized(ev, &set)?.to_vec();
    if elems.is_empty() {
        return Ok(empty_fcn());
    }
    if elems.len() == 1 {
        return Ok(elems[0].clone());
    }
    let not_a_bag = |ev: &mut Ev<'_>| -> EvalResult<EvalError> {
        let shown = a[0].ppr(ev)?;
        Ok(eval_exception(ev, Ec::TLC_MODULE_BAG_UNION1, &[&shown]))
    };
    let Some(fcn) = to_fcn_rcd(ev, &elems[0])? else { return Err(not_a_bag(ev)?) };
    let (mut domain, mut values) = domain_and_values(&fcn);
    for elem in &elems[1..] {
        let Some(fcn) = to_fcn_rcd(ev, elem)? else { return Err(not_a_bag(ev)?) };
        let (next_domain, next_values) = domain_and_values(&fcn);
        for j in 0..next_domain.len() {
            let mut found = false;
            for k in 0..domain.len() {
                if next_domain[j].tlc_equals(ev, &domain[k])? {
                    let (v1, v2) = (int_cast(&values[k])?, int_cast(&next_values[j])?);
                    values[k] = Value::Int(add_counts(ev, v1, v2)?);
                    found = true;
                    break;
                }
            }
            if !found {
                domain.push(next_domain[j].clone());
                values.push(next_values[j].clone());
            }
        }
    }
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, values, false)))
}

/// `\sqsubseteq` (`Bags.java:338-381`): no key of the first bag may be left over once the
/// second bag's count for it is taken away. Like `(-)` it casts the counts unchecked, and
/// like `(-)` it treats a count of zero as nothing to answer for, which the TLA+ body does
/// not.
pub(crate) fn sq_subseteq(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let fcn1 = to_fcn_rcd(ev, &a[0])?;
    let fcn2 = to_fcn_rcd(ev, &a[1])?;
    let wrong = |ev: &mut Ev<'_>, b: &Value| -> EvalResult<EvalError> {
        let shown = b.ppr(ev)?;
        Ok(eval_exception(ev, Ec::TLC_MODULE_APPLYING_TO_WRONG_VALUE, &["\\sqsubseteq", "a function with a finite domain", &shown]))
    };
    let Some(fcn1) = fcn1 else { return Err(wrong(ev, &a[0])?) };
    let Some(fcn2) = fcn2 else { return Err(wrong(ev, &a[1])?) };
    let (domain1, values1) = domain_and_values(&fcn1);
    let (domain2, values2) = domain_and_values(&fcn2);
    for i in 0..domain1.len() {
        let mut v1 = int_cast(&values1[i])?;
        for j in 0..domain2.len() {
            if domain1[i].tlc_equals(ev, &domain2[j])? {
                v1 = v1.wrapping_sub(int_cast(&values2[j])?);
                break;
            }
        }
        if v1 > 0 {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

/// `BagOfAll(F(_), B)` (`Bags.java:383-427`): one copy of `F(e)` for every copy of `e`.
/// The first argument is an operator, which in tlars is a binding rather than a value, so
/// the override receives its argument expressions; TLC has evaluated both arguments by the
/// time the method runs, and the method checks the operator before it converts the bag.
/// The `LazyFn` shape: an `@Evaluation`-style override that always answers, so never falls
/// back to the TLA+ definition.
pub(crate) fn bag_of_all(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    bag_of_all_value(ev, args, c, s0, s1, control).map(Some)
}

fn bag_of_all_value(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    let r = bag_of_all_impl(ev, args, c, s0, s1, control);
    r.map_err(|e| wrap_method(ev, "Bags", "BagOfAll", e))
}

fn bag_of_all_impl(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Value> {
    let f = ev.op_arg_value(args[0], c, control)?;
    let b = ev.eval(args[1].expect_expr(), c, s0, s1, control)?;
    let Some(f) = f else {
        // Not an `OpValue`, so what TLC prints is the value the argument evaluated to: an
        // operator whose body folded to a constant reports that constant.
        let shown = match ev.op_arg_as_value(args[0], c)? {
            Some(v) => v.ppr(ev)?,
            None => ev.op_arg_text(args[0], None)?,
        };
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR_AN, &["first", "BagOfAll", "operator", &shown]));
    };
    let Some(fcn) = to_fcn_rcd(ev, &b)? else {
        let shown = b.ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "BagOfAll", "function with a finite domain", &shown]));
    };
    let (keys, counts) = domain_and_values(&fcn);
    let mut domain: Vec<Value> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    for i in 0..keys.len() {
        // `ff.eval(args, EvalControl.Clear)`: the operator runs with a cleared control
        // whatever the caller's was.
        let image = ev.apply_operator(&f, std::slice::from_ref(&keys[i]), EvalControl::CLEAR)?;
        let mut found = false;
        for j in 0..domain.len() {
            if image.tlc_equals(ev, &domain[j])? {
                let (v1, v2) = (int_cast(&values[j])?, int_cast(&counts[i])?);
                values[j] = Value::Int(add_counts(ev, v1, v2)?);
                found = true;
                break;
            }
        }
        if !found {
            // The count is copied over untouched, so an image reached once keeps whatever
            // the bag held for its key.
            domain.push(image);
            values.push(counts[i].clone());
        }
    }
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, values, false)))
}

/// `BagToSet` (`Bags.java:441-451`): the domain, with the counts left unexamined.
pub(crate) fn bag_to_set(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(fcn) = to_fcn_rcd(ev, &a[0])? else {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_APPLYING_TO_WRONG_VALUE, &["BagToSet", "a function with a finite domain", &shown]));
    };
    Value::FcnRcd(fcn).get_domain(ev)
}

/// `SetToBag` (`Bags.java:453-478`): one copy of every element. The set is normalised first
/// so that a duplicate cannot reach the bag's domain, and the error names `BagToSet`
/// because the Java method does.
pub(crate) fn set_to_bag(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(set) = a[0].to_set_enum(ev)? else {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_APPLYING_TO_WRONG_VALUE, &["BagToSet", "a function with a finite domain", &shown]));
    };
    // `if (!s1.isNormalized()) s1.normalize();`
    if set.norm.get().is_none() {
        crate::value::order::normalized(ev, &set)?;
    }
    let elems = set.snapshot().to_vec();
    let values = vec![Value::Int(1); elems.len()];
    Ok(Value::FcnRcd(Value::fcn_rcd_arc(elems, values, set.norm.get().is_some())))
}
