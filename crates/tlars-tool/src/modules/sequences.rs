//! `tlc2.module.Sequences`.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::modules::eval_exception;
use crate::value::Value;
use tlars_msg::Ec;

pub(crate) fn append(ev: &mut Ev<'_>, args: &[Value]) -> EvalResult<Value> {
    let Some(seq) = args[0].to_tuple(ev)? else {
        let shown = args[0].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_EVALUATING, &["Append(s, v)", "sequence", &shown]));
    };
    let mut elems = seq.elems.to_vec();
    elems.push(args[1].clone());
    Ok(Value::tuple(elems))
}

/// `Len` also accepts strings.
pub(crate) fn len(ev: &mut Ev<'_>, args: &[Value]) -> EvalResult<Value> {
    todo!()
}
