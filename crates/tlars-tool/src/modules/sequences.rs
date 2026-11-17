//! `tlc2.module.Sequences`.

use crate::error::EvalResult;
use crate::eval::Ev;
use crate::value::Value;

pub(crate) fn append(ev: &mut Ev<'_>, args: &[Value]) -> EvalResult<Value> {
    let mut elems = args[0].to_tuple_elems(ev, "Append", 1)?;
    elems.push(args[1].clone());
    Ok(Value::tuple(elems))
}

/// `Len` also accepts strings.
pub(crate) fn len(ev: &mut Ev<'_>, args: &[Value]) -> EvalResult<Value> {
    todo!()
}
