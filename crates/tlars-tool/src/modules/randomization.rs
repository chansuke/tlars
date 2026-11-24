//! `tlc2.module.Randomization`.
//!
//! All three operators check their arguments by Java class and then hand over to the
//! randomized enumerators in `value::random`. Their messages name `RandomSubsetSet` as
//! `RandomSubsetSetProbability`, which is the string the Java passes.

use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use crate::modules::eval_exception;
use crate::value::{Value, random};
use tlars_msg::Ec;

/// `RandomSubset(k, S)` (`Randomization.java:40-53`). There is no check for a negative `k`
/// here, so it reaches `new ValueVec(k)` and fails with the JVM's own message.
pub(crate) fn random_subset(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Value::Int(k) = a[0] else { return Err(not_an_int(ev, &a[0], "first", "RandomSubset")?) };
    // `"a finite set"` where the other two pass `"finite set"`, and the template prepends
    // `a`, so the text reads "should be a a finite set".
    finite_set(ev, &a[1], "second", "RandomSubset", "a finite set")?;
    random::get_random_subset(ev, &a[1], k)
}

/// `RandomSetOfSubsets(k, n, S)` (`Randomization.java:55-96`): `n` is the expected
/// cardinality of each pick, turned into the per-element probability `n / Cardinality(S)`.
pub(crate) fn random_set_of_subsets(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let name = "RandomSetOfSubsets";
    let picks = nonnegative_int(ev, &a[0], "first", name)?;
    let n = nonnegative_int(ev, &a[1], "second", name)?;
    finite_set(ev, &a[2], "third", name, "finite set")?;
    let size = a[2].size(ev)?;
    check_picks_fit(ev, picks, size, name)?;
    let out_of_range = |ev: &mut Ev<'_>| -> EvalResult<EvalError> {
        let shown = a[1].ppr(ev)?;
        let what = "nonnegative integer in range 0..Cardinality(S)";
        Ok(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", name, what, &shown]))
    };
    if size < n {
        return Err(out_of_range(ev)?);
    }
    // `(1d * n) / ev.size()`, which for an empty `S` with `n = 0` is NaN and passes the
    // range check, because `NaN < 0` and `1 < NaN` are both false.
    let probability = f64::from(n) / f64::from(size);
    if probability < 0.0 || 1.0 < probability {
        return Err(out_of_range(ev)?);
    }
    random::get_random_set_of_subsets(ev, &a[2], picks, probability)
}

/// `RandomSubsetSet(k, p, S)` (`Randomization.java:98-142`): the same coin tossing with the
/// probability given directly, as a string Java parses with `Double.valueOf`.
pub(crate) fn random_subset_set(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let name = "RandomSubsetSetProbability";
    let picks = nonnegative_int(ev, &a[0], "first", name)?;
    let Value::Str(text) = &a[1] else {
        let shown = a[1].ppr(ev)?;
        let what = "string literal representing a probability";
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", name, what, &shown]));
    };
    let text = ev.tool().graph().interner().text(*text).to_string();
    // One message for a string `Double.valueOf` rejects and for one outside 0..1. `"NaN"`
    // parses and passes, because `NaN < 0` and `1 < NaN` are both false.
    let unparsable = |ev: &mut Ev<'_>| -> EvalResult<EvalError> {
        let shown = a[1].ppr(ev)?;
        let what = "string literal does not represent a parsable probability";
        Ok(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", name, what, &shown]))
    };
    let Ok(probability) = tlars_java::parse_double(&text) else {
        return Err(unparsable(ev)?);
    };
    if probability < 0.0 || 1.0 < probability {
        return Err(unparsable(ev)?);
    }
    finite_set(ev, &a[2], "third", name, "finite set")?;
    let size = a[2].size(ev)?;
    check_picks_fit(ev, picks, size, name)?;
    random::get_random_set_of_subsets(ev, &a[2], picks, probability)
}

fn not_an_int(ev: &mut Ev<'_>, v: &Value, which: &str, name: &str) -> EvalResult<EvalError> {
    let shown = v.ppr(ev)?;
    Ok(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &[which, name, "nonnegative integer", &shown]))
}

/// `!(v instanceof IntValue)` followed by `val < 0`, which both operators that take a count
/// write out for their first two arguments and report with the same message.
fn nonnegative_int(ev: &mut Ev<'_>, v: &Value, which: &str, name: &str) -> EvalResult<i32> {
    let Value::Int(k) = v else { return Err(not_an_int(ev, v, which, name)?) };
    if *k < 0 {
        return Err(not_an_int(ev, v, which, name)?);
    }
    Ok(*k)
}

/// `!(v instanceof EnumerableValue) || !((EnumerableValue) v).isFinite()`. A set whose
/// finiteness cannot be decided raises from `isFinite` itself, so that failure is
/// propagated instead of becoming the argument error.
fn finite_set(ev: &mut Ev<'_>, v: &Value, which: &str, name: &str, what: &str) -> EvalResult<()> {
    if !v.is_enumerable() || !v.is_finite(ev)? {
        let shown = v.ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &[which, name, what, &shown]));
    }
    Ok(())
}

/// `31 - Integer.numberOfLeadingZeros(k) + 1 > size && k > (1 << size)`: the bit length of
/// `k` is compared with `size` first, so `2^size` is computed only where it cannot overflow
/// the comparison. The shift is Java's, which takes `size` modulo 32.
fn check_picks_fit(ev: &mut Ev<'_>, picks: i32, size: i32, name: &str) -> EvalResult<()> {
    let bit_length = 31 - picks.leading_zeros() as i32 + 1;
    if bit_length > size && picks > 1i32.wrapping_shl(size as u32) {
        let what = format!("nonnegative integer that is smaller than the subset's size of 2^{size}");
        let shown = picks.to_string();
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["first", name, &what, &shown]));
    }
    Ok(())
}
