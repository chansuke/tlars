//! `tlc2.module.Naturals` and `tlc2.module.Integers`. The two classes share most bodies
//! (`Integers.Plus` calls `Naturals.Plus`); where they differ there is one function each.

use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use crate::value::{IntervalValue, UserValue, Value};
use std::sync::Arc;
use tlars_msg::Ec;

fn int_of(v: &Value) -> i32 {
    match v {
        Value::Int(n) => *n,
        _ => unreachable!("modules::apply casts IntValue parameters"),
    }
}

/// `new EvalException(ec, params)`: the message is `MP.getMessage(ec, params)`.
pub(crate) fn eval_exception(ev: &Ev<'_>, ec: Ec, params: &[&str]) -> EvalError {
    EvalError::eval(ec, ev.out().text(ec, params), params.iter().map(|p| p.to_string()).collect())
}

fn overflow(ev: &Ev<'_>, text: String) -> EvalError {
    eval_exception(ev, Ec::TLC_MODULE_OVERFLOW, &[&text])
}

pub(crate) fn nat(_: &mut Ev<'_>, _: &[Value]) -> EvalResult<Value> {
    Ok(Value::User(Arc::new(UserValue::Nat)))
}

pub(crate) fn int(_: &mut Ev<'_>, _: &[Value]) -> EvalResult<Value> {
    Ok(Value::User(Arc::new(UserValue::Int)))
}

pub(crate) fn plus(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let (n1, n2) = (int_of(&a[0]), int_of(&a[1]));
    n1.checked_add(n2).map(Value::Int).ok_or_else(|| overflow(ev, format!("{n1}+{n2}")))
}

pub(crate) fn minus(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let (n1, n2) = (int_of(&a[0]), int_of(&a[1]));
    n1.checked_sub(n2).map(Value::Int).ok_or_else(|| overflow(ev, format!("{n1}-{n2}")))
}

pub(crate) fn times(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let (n1, n2) = (int_of(&a[0]), int_of(&a[1]));
    n1.checked_mul(n2).map(Value::Int).ok_or_else(|| overflow(ev, format!("{n1}*{n2}")))
}

/// The comparisons take any value and name the operator in their error, which for
/// `Naturals.GEQ` is `>` (`Naturals.java:149,155`) and for `Integers.GEQ` is `>=`.
fn compare(ev: &mut Ev<'_>, a: &[Value], name: &str, holds: fn(i32, i32) -> bool) -> EvalResult<Value> {
    for (i, ord) in [(0, "first"), (1, "second")] {
        if !matches!(a[i], Value::Int(_)) {
            let shown = a[i].ppr(ev)?;
            return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR_AN, &[ord, name, "integer", &shown]));
        }
    }
    Ok(Value::Bool(holds(int_of(&a[0]), int_of(&a[1]))))
}

pub(crate) fn lt(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    compare(ev, a, "<", |x, y| x < y)
}

pub(crate) fn le(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    compare(ev, a, "<=", |x, y| x <= y)
}

pub(crate) fn gt(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    compare(ev, a, ">", |x, y| x > y)
}

pub(crate) fn geq_naturals(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    compare(ev, a, ">", |x, y| x >= y)
}

pub(crate) fn geq_integers(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    compare(ev, a, ">=", |x, y| x >= y)
}

pub(crate) fn dot_dot(_: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    Ok(Value::Interval(IntervalValue { lo: int_of(&a[0]), hi: int_of(&a[1]) }))
}

pub(crate) fn neg(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let n = int_of(&a[0]);
    n.checked_neg().map(Value::Int).ok_or_else(|| overflow(ev, "--2147483648".into()))
}

/// `Naturals.Divide`: Java's truncating division, then one down when negative and inexact.
/// `-2147483648 \div -1` wraps, as Java's `/` does.
pub(crate) fn divide_naturals(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let (n1, n2) = (int_of(&a[0]), int_of(&a[1]));
    if n2 == 0 {
        return Err(eval_exception(ev, Ec::TLC_MODULE_DIVISION_BY_ZERO, &[]));
    }
    let mut q = n1.wrapping_div(n2);
    if q < 0 && q.wrapping_mul(n2) != n1 {
        q -= 1;
    }
    Ok(Value::Int(q))
}

pub(crate) fn divide_integers(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let (n1, n2) = (int_of(&a[0]), int_of(&a[1]));
    if n2 == 0 {
        return Err(eval_exception(ev, Ec::TLC_MODULE_DIVISION_BY_ZERO, &[]));
    }
    if n1 == i32::MIN && n2 == -1 {
        return Err(overflow(ev, "-2147483648 \\div -1".into()));
    }
    let mut q = n1 / n2;
    if ((n1 < 0 && n2 > 0) || (n1 > 0 && n2 < 0)) && q * n2 != n1 {
        q -= 1;
    }
    Ok(Value::Int(q))
}

pub(crate) fn modulo(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let (n1, n2) = (int_of(&a[0]), int_of(&a[1]));
    if n2 <= 0 {
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "%", "positive number", &n2.to_string()]));
    }
    let r = n1.wrapping_rem(n2);
    Ok(Value::Int(if r < 0 { r + n2 } else { r }))
}

pub(crate) fn expt(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let (n1, n2) = (int_of(&a[0]), int_of(&a[1]));
    if n2 < 0 {
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "^", "natural number", &n2.to_string()]));
    }
    if n2 == 0 {
        if n1 == 0 {
            return Err(eval_exception(ev, Ec::TLC_MODULE_NULL_POWER_NULL, &[]));
        }
        return Ok(Value::Int(1));
    }
    let mut res = i64::from(n1);
    for _ in 1..n2 {
        res *= i64::from(n1);
        if res < i64::from(i32::MIN) || res > i64::from(i32::MAX) {
            return Err(overflow(ev, format!("{n1}^{n2}")));
        }
    }
    Ok(Value::Int(res as i32))
}
