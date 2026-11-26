//! `tlc2.module.Sequences`. `Len`, `Tail`, `SubSeq` and `\o` also accept strings, which they
//! index in UTF-16 units as Java does.

use crate::ctx::Ctx;
use crate::error::{EvalError, EvalResult};
use crate::eval::{Ev, EvalControl};
use crate::modules::eval_exception;
use crate::state::StateRef;
use crate::value::{UserValue, Value};
use std::sync::Arc;
use tlars_msg::Ec;
use tlars_sany::graph::ExprOrOpArg;

/// The argument as a sequence, or the error naming the operator's only argument.
fn one_seq(ev: &mut Ev<'_>, v: &Value, name: &str) -> EvalResult<Arc<crate::value::TupleValue>> {
    match v.to_tuple(ev)? {
        Some(t) => Ok(t),
        None => {
            let shown = v.ppr(ev)?;
            Err(eval_exception(ev, Ec::TLC_MODULE_ONE_ARGUMENT_ERROR, &[name, "sequence", &shown]))
        }
    }
}

/// A string's characters as Java indexes them.
fn utf16(ev: &mut Ev<'_>, v: &Value) -> Option<Vec<u16>> {
    match v {
        Value::Str(a) => Some(ev.tool().graph().interner().text(*a).encode_utf16().collect()),
        _ => None,
    }
}

fn string_of(ev: &mut Ev<'_>, units: &[u16]) -> EvalResult<Value> {
    let Ok(text) = String::from_utf16(units) else {
        return Err(EvalError::unimplemented("a string cut between the halves of a surrogate pair".into()));
    };
    Ok(Value::Str(ev.tool().graph().interner().intern(&text)))
}

/// `Seq(S)`: the infinite set of finite sequences over `S`.
pub(crate) fn seq(_ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    Ok(Value::User(Arc::new(UserValue::Seq(a[0].clone()))))
}

/// `Len` accepts strings as well as sequences (`Sequences.java:59-73`).
pub(crate) fn len(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if let Some(units) = utf16(ev, &a[0]) {
        return Ok(Value::Int(units.len() as i32));
    }
    let seq = one_seq(ev, &a[0], "Len")?;
    Ok(Value::Int(seq.elems.len() as i32))
}

pub(crate) fn head(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let seq = one_seq(ev, &a[0], "Head")?;
    match seq.elems.first() {
        Some(v) => Ok(v.clone()),
        None => Err(eval_exception(ev, Ec::TLC_MODULE_APPLY_EMPTY_SEQ, &["Head"])),
    }
}

pub(crate) fn tail(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if let Some(units) = utf16(ev, &a[0]) {
        if units.is_empty() {
            return Err(eval_exception(ev, Ec::TLC_MODULE_APPLY_EMPTY_SEQ, &["Tail"]));
        }
        return string_of(ev, &units[1..]);
    }
    let seq = one_seq(ev, &a[0], "Tail")?;
    if seq.elems.is_empty() {
        return Err(eval_exception(ev, Ec::TLC_MODULE_APPLY_EMPTY_SEQ, &["Tail"]));
    }
    Ok(Value::tuple(seq.elems[1..].to_vec()))
}

pub(crate) fn append(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(seq) = a[0].to_tuple(ev)? else {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_EVALUATING, &["Append(s, v)", "sequence", &shown]));
    };
    let mut elems = seq.elems.to_vec();
    elems.push(a[1].clone());
    Ok(Value::tuple(elems))
}

/// `s \o t`: two strings concatenate, two sequences join, and a mixed pair is an error that
/// names the second operand (`Sequences.java:147-189`).
pub(crate) fn concat(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    if let Some(mut first) = utf16(ev, &a[0]) {
        let Some(second) = utf16(ev, &a[1]) else {
            let shown = a[1].ppr(ev)?;
            return Err(eval_exception(ev, Ec::TLC_MODULE_EVALUATING, &["t \\o s", "string", &shown]));
        };
        first.extend_from_slice(&second);
        return string_of(ev, &first);
    }
    let Some(seq1) = a[0].to_tuple(ev)? else {
        let shown = a[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_EVALUATING, &["s \\o t", "sequence", &shown]));
    };
    let Some(seq2) = a[1].to_tuple(ev)? else {
        let shown = a[1].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_EVALUATING, &["t \\o s", "sequence", &shown]));
    };
    if seq1.elems.is_empty() {
        return Ok(Value::Tuple(seq2));
    }
    if seq2.elems.is_empty() {
        return Ok(Value::Tuple(seq1));
    }
    Ok(Value::tuple(seq1.elems.iter().chain(seq2.elems.iter()).cloned().collect()))
}

/// `SubSeq(s, m, n)`: the bounds are checked after the empty case, so `SubSeq(s, 2, 1)` is
/// the empty sequence whatever `s` holds (`Sequences.java:262-326`).
pub(crate) fn sub_seq(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let units = utf16(ev, &a[0]);
    let seq = match &units {
        Some(_) => None,
        None => match a[0].to_tuple(ev)? {
            Some(t) => Some(t),
            None => {
                let shown = a[0].ppr(ev)?;
                return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["first", "SubSeq", "sequence", &shown]));
            }
        },
    };
    for (which, arg) in [("second", &a[1]), ("third", &a[2])] {
        if !matches!(arg, Value::Int(_)) {
            let shown = arg.ppr(ev)?;
            return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &[which, "SubSeq", "natural number", &shown]));
        }
    }
    let (Value::Int(beg), Value::Int(end)) = (&a[1], &a[2]) else { unreachable!("both checked above") };
    let (beg, end) = (*beg, *end);
    if beg > end {
        return match units {
            Some(_) => string_of(ev, &[]),
            None => Ok(Value::tuple(Vec::new())),
        };
    }
    let len = match (&units, &seq) {
        (Some(u), _) => u.len() as i64,
        (None, Some(s)) => s.elems.len() as i64,
        (None, None) => unreachable!("one of the two forms"),
    };
    for (which, bound) in [("second", beg), ("third", end)] {
        if i64::from(bound) < 1 || i64::from(bound) > len {
            let (s, shown) = (a[0].ppr(ev)?, Value::Int(bound).ppr(ev)?);
            return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_NOT_IN_DOMAIN, &[which, "SubSeq", "first", &s, &shown]));
        }
    }
    let (from, to) = (beg as usize - 1, end as usize);
    match (units, seq) {
        (Some(u), _) => string_of(ev, &u[from..to]),
        (None, Some(s)) => Ok(Value::tuple(s.elems[from..to].to_vec())),
        (None, None) => unreachable!("one of the two forms"),
    }
}

/// `SelectSeq`, whose Java method never returns null and so never falls back to the TLA+
/// definition.
pub(crate) fn select_seq(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    s0: StateRef<'_>,
    s1: StateRef<'_>,
    control: EvalControl,
) -> EvalResult<Option<Value>> {
    select_seq_impl(ev, args, c, s0, s1, control).map(Some)
}

/// `SelectSeq(s, Test)`: the test is an operator, so the argument arrives unevaluated and is
/// resolved the way `OpArgNode` evaluation resolves it (`Tool.java:1884-1892`).
fn select_seq_impl(
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
    let test = ev.op_arg_value(args[1], c, s0, s1, control)?;
    let kept = select_seq_body(ev, args, c, &sval, test.as_ref());
    kept.map_err(|e| super::wrap_method(ev, "Sequences", "SelectSeq", e))
}

fn select_seq_body(
    ev: &mut Ev<'_>,
    args: &[ExprOrOpArg],
    c: &Ctx,
    sval: &Value,
    test: Option<&crate::eval::expr::OpArgValue<'_>>,
) -> EvalResult<Value> {
    let Some(seq) = sval.to_tuple(ev)? else {
        let shown = sval.ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["first", "SelectSeq", "sequence", &shown]));
    };
    if seq.elems.is_empty() {
        return Ok(Value::tuple(Vec::new()));
    }
    let Some(test) = test else {
        // `test` is not an `OpValue`, so what TLC prints is the value the argument
        // evaluated to, not the operator the source named.
        let shown = match ev.op_arg_as_value(args[1], c)? {
            Some(v) => v.ppr(ev)?,
            None => ev.op_arg_text(args[1], None)?,
        };
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "SelectSeq", "operator", &shown]));
    };
    let mut kept = Vec::new();
    for elem in seq.elems.iter() {
        // `ftest.eval(args, EvalControl.Clear)`: the test runs with a cleared control
        // whatever the caller's was.
        match ev.apply_operator(test, std::slice::from_ref(elem), EvalControl::CLEAR)? {
            Value::Bool(true) => kept.push(elem.clone()),
            Value::Bool(false) => {}
            _ => {
                let shown = ev.op_arg_text(args[1], Some(test))?;
                return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "SelectSeq", "boolean-valued operator", &shown]));
            }
        }
    }
    Ok(Value::tuple(kept))
}
