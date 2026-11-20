//! TLC's equality, total order and membership: the single owner of
//! - cardinality-first set order, functions by size/domain/values, records by field count,
//!   field tokens, then values; strings and model values by intern token;
//! - the untyped-model-value rule (less than, and unequal to, any non-model value) and
//!   typed-model-value errors;
//! - every "Attempted to compare ..." / "Attempted to check equality of ..." text.
//!
//! Dispatch is receiver-first, as in Java (`a.equals(b)` runs `a`'s class's code), because
//! the error text names the receiver's class. Cross-representation results (Interval vs
//! SetEnum, Tuple vs FcnRcd, Record vs string-domain FcnRcd) are defined once over
//! [`Canon`] (grafted from candidate 2). A representation fast path (e.g. Interval vs
//! Interval by bounds) goes through [`fast_checked`], which in debug builds recomputes the
//! answer through `Canon` and panics on disagreement, so "fast path agrees" is a structural
//! check, not a promise.

use super::{FcnRcdValue, IntervalValue, ModelValue, RecordValue, SetEnumValue, UserValue, Value};
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use crate::value::enumerate::EnumOrder;
use std::cmp::Ordering;
use std::sync::Arc;
use tlars_msg::Ec;
use tlars_sany::Atom;

impl ModelValue {
    /// `modelValueEquals(obj)` (`ModelValue.java:235-250`): the answer another class's
    /// `equals` returns once its own conversion of this model value failed. An untyped model
    /// value is simply unequal; a typed one is an error.
    fn non_model_equals(&self, ev: &mut Ev<'_>, obj: &Value) -> EvalResult<bool> {
        if self.ty.is_none() {
            return Ok(false);
        }
        let (a, b) = (Value::Model(*self).ppr(ev)?, obj.ppr(ev)?);
        Err(EvalError::detailed(format!("Attempted to check equality of the typed model value {a} and the non-model value\n{b}"), None))
    }

    /// `modelValueCompareTo(obj)` (`ModelValue.java:212-226`): the answer another class's
    /// `compareTo` returns, so `Greater` says the non-model value sorts above.
    fn non_model_compare(&self, ev: &mut Ev<'_>, obj: &Value) -> EvalResult<Ordering> {
        if self.ty.is_none() {
            return Ok(Ordering::Greater);
        }
        let (a, b) = (Value::Model(*self).ppr(ev)?, obj.ppr(ev)?);
        Err(EvalError::detailed(format!("Attempted to compare the typed model value {a} and the non-model value\n{b}"), None))
    }

    /// `modelValueMember(obj)` (`ModelValue.java:252-266`): the answer `obj.member(this)`
    /// returns once `obj` could not read this model value as one of its elements.
    fn non_model_member(&self, ev: &mut Ev<'_>, obj: &Value) -> EvalResult<bool> {
        if self.ty.is_none() {
            return Ok(false);
        }
        let (a, b) = (Value::Model(*self).ppr(ev)?, obj.ppr(ev)?);
        Err(EvalError::detailed(format!("Attempted to check if the typed model value {a} is an element of\n{b}"), None))
    }

    /// The common half of `ModelValue.equals` and `ModelValue.compareTo`: an untyped model
    /// value compares with any model value, a typed one only with its own type and the
    /// untyped ones. `None` means `obj` is not a model value at all.
    fn comparable(&self, ev: &mut Ev<'_>, obj: &Value, verb: &str) -> EvalResult<Option<Ordering>> {
        let Value::Model(m) = obj else { return Ok(None) };
        if self.ty.is_none() || m.ty.is_none() || m.ty == self.ty {
            return Ok(Some(self.name.tok().cmp(&m.name.tok())));
        }
        let (a, b) = (Value::Model(*self).ppr(ev)?, obj.ppr(ev)?);
        Err(EvalError::detailed(format!("Attempted to {verb} the differently-typed model values {a} and {b}"), None))
    }

    /// `ModelValue.equals` (`ModelValue.java:188-210`).
    fn equals(&self, ev: &mut Ev<'_>, obj: &Value) -> EvalResult<bool> {
        match self.comparable(ev, obj, "check equality of")? {
            Some(c) => Ok(c == Ordering::Equal),
            None if self.ty.is_none() => Ok(false),
            None => {
                let (a, b) = (Value::Model(*self).ppr(ev)?, obj.ppr(ev)?);
                Err(EvalError::detailed(format!("Attempted to check equality of typed model value {a} and non-model value\n{b}"), None))
            }
        }
    }

    /// `ModelValue.compareTo` (`ModelValue.java:155-186`): an untyped model value sorts
    /// below every non-model value, whatever its class.
    fn compare(&self, ev: &mut Ev<'_>, obj: &Value) -> EvalResult<Ordering> {
        match self.comparable(ev, obj, "compare")? {
            Some(c) => Ok(c),
            None if self.ty.is_none() => Ok(Ordering::Less),
            None => {
                let (a, b) = (Value::Model(*self).ppr(ev)?, obj.ppr(ev)?);
                Err(EvalError::detailed(format!("Attempted to compare the typed model value {a} and non-model value\n{b}"), None))
            }
        }
    }
}

impl Value {
    /// `IValue.equals`. The evaluator calls `lhs.tlc_equals(ev, rhs)` for `lhs = rhs`.
    pub(crate) fn tlc_equals(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<bool> {
        // The scalar classes accept their own class and model values, and fail otherwise
        // naming the receiver (`BoolValue.java`, `IntValue.java`, `StringValue.java`).
        let mismatch = |ev: &mut Ev<'_>, verb: &str, noun: &str| -> EvalResult<bool> {
            let (a, b) = (self.ppr(ev)?, other.ppr(ev)?);
            Err(EvalError::detailed(format!("Attempted to {verb} equality of {noun} {a} with non-{noun}:\n{b}"), None))
        };
        match (self, other) {
            (Value::Model(a), _) => a.equals(ev, other),
            (Value::Bool(_) | Value::Int(_) | Value::Str(_), Value::Model(m)) => m.non_model_equals(ev, self),
            (Value::Bool(a), Value::Bool(b)) => Ok(a == b),
            (Value::Bool(_), _) => mismatch(ev, "compare", "boolean"),
            (Value::Int(a), Value::Int(b)) => Ok(a == b),
            (Value::Int(_), _) => mismatch(ev, "check", "integer"),
            (Value::Str(a), Value::Str(b)) => Ok(a == b),
            (Value::Str(_), _) => mismatch(ev, "check", "string"),
            (Value::Interval(a), Value::Interval(b)) => interval_equals(ev, *a, *b),
            (Value::Interval(a), _) => Value::SetEnum(a.to_set_enum_arc()).tlc_equals(ev, other),
            (Value::SetEnum(a), _) => {
                let Some(b) = other.to_set_enum(ev)? else {
                    if let Value::Model(m) = other {
                        return m.non_model_equals(ev, self);
                    }
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to check equality of the set {x} with the value:\n{y}"), None));
                };
                let (xs, ys) = (normalized(ev, a)?.to_vec(), normalized(ev, &b)?.to_vec());
                if xs.len() != ys.len() {
                    return Ok(false);
                }
                for (x, y) in xs.iter().zip(&ys) {
                    if !x.tlc_equals(ev, y)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            // A value that is not a tuple is compared as a function instead, so
            // `<<1, 2>> = [i \in 1..2 |-> i]` holds (`TupleValue.java:93-116`).
            (Value::Tuple(a), _) => match other.to_tuple(ev)? {
                Some(b) => {
                    if a.elems.len() != b.elems.len() {
                        return Ok(false);
                    }
                    for (x, y) in a.elems.iter().zip(b.elems.iter()) {
                        if !x.tlc_equals(ev, y)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                None => {
                    let f = self.to_fcn_rcd(ev)?.expect("a tuple converts to a function");
                    Value::FcnRcd(f).tlc_equals(ev, other)
                }
            },
            (Value::Record(a), _) => {
                let Some(b) = other.to_rcd(ev)? else {
                    if let Value::Model(m) = other {
                        return m.non_model_equals(ev, self);
                    }
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to check equality of record:\n{x}\nwith non-record\n{y}"), None));
                };
                let (an, av) = record_normalized(ev, a)?;
                let (bn, bv) = record_normalized(ev, &b)?;
                if an != bn {
                    return Ok(false);
                }
                for (x, y) in av.iter().zip(bv.iter()) {
                    if !x.tlc_equals(ev, y)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Value::FcnRcd(a), _) => {
                let Some(b) = other.to_fcn_rcd(ev)? else {
                    if let Value::Model(m) = other {
                        return m.non_model_equals(ev, self);
                    }
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to check equality of the function {x} with the value:\n{y}"), None));
                };
                fcn_equals(ev, a, &b)
            }
            (Value::FcnLambda(a), _) => {
                let f = a.to_fcn_rcd(ev)?;
                Value::FcnRcd(f).tlc_equals(ev, other)
            }
            (Value::User(_), _) => Ok(self.user_compare(ev, other)? == Ordering::Equal),
            // `SUBSET S = SUBSET T` is decided by `S = T` without enumerating either
            // (`SubsetValue.java:75-86`).
            (Value::Subset(a), Value::Subset(b)) => a.base.tlc_equals(ev, &b.base),
            (Value::KSubset(_), _) | (_, Value::KSubset(_)) => Err(EvalError::unimplemented("equality of a k-subset (M5)".into())),
            // `[S -> T]`: one empty domain makes both the unit set, one empty range makes
            // both empty (`SetOfFcnsValue.java:80-95`).
            (Value::SetOfFcns(a), Value::SetOfFcns(b)) => {
                let (unit1, unit2) = (a.domain.is_empty(ev)?, b.domain.is_empty(ev)?);
                if unit1 || unit2 {
                    return Ok(unit1 == unit2);
                }
                let (empty1, empty2) = (a.range.is_empty(ev)?, b.range.is_empty(ev)?);
                if empty1 || empty2 {
                    return Ok(empty1 == empty2);
                }
                Ok(a.domain.tlc_equals(ev, &b.domain)? && a.range.tlc_equals(ev, &b.range)?)
            }
            (Value::SetOfRcds(a), Value::SetOfRcds(b)) => {
                if self.is_empty(ev)? {
                    return other.is_empty(ev);
                }
                if other.is_empty(ev)? {
                    return Ok(false);
                }
                if a.names.len() != b.names.len() {
                    return Ok(false);
                }
                // The name is compared before the field's set, so a differing name stops
                // the comparison of the sets (`SetOfRcdsValue.java:75-81`).
                for i in 0..a.names.len() {
                    if a.names[i] != b.names[i] || !a.sets[i].tlc_equals(ev, &b.sets[i])? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Value::SetOfTuples(a), Value::SetOfTuples(b)) => {
                if self.is_empty(ev)? {
                    return other.is_empty(ev);
                }
                if other.is_empty(ev)? {
                    return Ok(false);
                }
                if a.sets.len() != b.sets.len() {
                    return Ok(false);
                }
                values_equal(ev, &a.sets, &b.sets)
            }
            // Every other lazy set is enumerated first, so the comparison and its error
            // text are SetEnumValue's.
            _ if self.is_enumerable() => {
                let set = self.convert_and_cache(ev)?;
                Value::SetEnum(set).tlc_equals(ev, other)
            }
            _ => Err(EvalError::unimplemented(format!("equality of {} (M2 unit 5)", self.kind_string()))),
        }
    }

    /// `UserValue.compareTo` (`UserValue.java`): another special set is compared by the
    /// receiver's `UserObj` (`Naturals.java:226-240`, `Integers.java`, `Strings.java`), a
    /// model value is below, anything else is an error.
    fn user_compare(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Ordering> {
        let Value::User(u) = self else { unreachable!("user_compare on a user value") };
        if !matches!(other, Value::User(_)) {
            if matches!(other, Value::Model(_)) {
                return Ok(Ordering::Greater);
            }
            let (a, b) = (self.ppr(ev)?, other.ppr(ev)?);
            return Err(EvalError::detailed(format!("Attempted to compare overridden value {a} with non-overridden value:\n{b}"), None));
        }
        let (rank, name) = match &**u {
            UserValue::Nat => (0, "Nat"),
            UserValue::Int => (1, "Int"),
            UserValue::String => {
                // `Strings.compareTo`: only STRING itself and model values compare.
                return match other {
                    Value::User(o) if matches!(**o, UserValue::String) => Ok(Ordering::Equal),
                    Value::Model(_) => Ok(Ordering::Greater),
                    _ => {
                        let shown = other.ppr(ev)?;
                        Err(crate::modules::eval_exception(ev, Ec::TLC_MODULE_COMPARE_VALUE, &["STRING", &shown]))
                    }
                };
            }
            // `Sequences.compareTo`: another `Seq(T)` compares by `T`, since every `Seq` has
            // the same unbounded length (`Sequences.java:369-390`).
            UserValue::Seq(range) => {
                if let Value::User(o) = other
                    && let UserValue::Seq(other_range) = &**o
                {
                    return range.tlc_compare(ev, other_range);
                }
                if matches!(other, Value::Model(_)) {
                    return Ok(Ordering::Greater);
                }
                let (a, b) = (self.ppr(ev)?, other.ppr(ev)?);
                return Err(crate::modules::eval_exception(ev, Ec::TLC_MODULE_COMPARE_VALUE, &[&a, &b]));
            }
            _ => return Err(EvalError::unimplemented("comparison of this special set (M2 unit 3)".into())),
        };
        match other {
            Value::User(o) if matches!(**o, UserValue::Nat) => Ok(rank.cmp(&0)),
            Value::User(o) if matches!(**o, UserValue::Int) => Ok(rank.cmp(&1)),
            Value::Model(_) => Ok(Ordering::Greater),
            _ => {
                let shown = other.ppr(ev)?;
                Err(crate::modules::eval_exception(ev, Ec::TLC_MODULE_COMPARE_VALUE, &[name, &shown]))
            }
        }
    }

    /// `IValue.compareTo`.
    pub(crate) fn tlc_compare(&self, ev: &mut Ev<'_>, other: &Value) -> EvalResult<Ordering> {
        let mismatch = |ev: &mut Ev<'_>, noun: &str| -> EvalResult<Ordering> {
            let (a, b) = (self.ppr(ev)?, other.ppr(ev)?);
            Err(EvalError::detailed(format!("Attempted to compare {noun} {a} with non-{noun}:\n{b}"), None))
        };
        match (self, other) {
            (Value::Model(a), _) => a.compare(ev, other),
            (Value::Bool(_) | Value::Int(_) | Value::Str(_), Value::Model(m)) => m.non_model_compare(ev, self),
            (Value::Bool(a), Value::Bool(b)) => Ok(a.cmp(b)),
            (Value::Bool(_), _) => mismatch(ev, "boolean"),
            (Value::Int(a), Value::Int(b)) => Ok(a.cmp(b)),
            (Value::Int(_), _) => mismatch(ev, "integer"),
            // `UniqueString.compareTo`: by intern token, not by text.
            (Value::Str(a), Value::Str(b)) => Ok(a.tok().cmp(&b.tok())),
            (Value::Str(_), _) => mismatch(ev, "string"),
            (Value::Interval(a), Value::Interval(b)) => {
                let (sa, sb) = (a.size(ev)?, b.size(ev)?);
                Ok(if sa != sb { sa.cmp(&sb) } else if sa == 0 { Ordering::Equal } else { a.lo.cmp(&b.lo) })
            }
            (Value::Interval(a), _) => Value::SetEnum(a.to_set_enum_arc()).tlc_compare(ev, other),
            (Value::SetEnum(a), _) => {
                let Some(b) = other.to_set_enum(ev)? else {
                    if let Value::Model(m) = other {
                        return m.non_model_compare(ev, self);
                    }
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to compare the set {x} with the value:\n{y}"), None));
                };
                let (xs, ys) = (normalized(ev, a)?.to_vec(), normalized(ev, &b)?.to_vec());
                if xs.len() != ys.len() {
                    return Ok(xs.len().cmp(&ys.len()));
                }
                for (x, y) in xs.iter().zip(&ys) {
                    let c = x.tlc_compare(ev, y)?;
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                Ok(Ordering::Equal)
            }
            // A tuple's domain is 1..Len, so equal lengths mean equal domains
            // (`TupleValue.java:67-91`).
            (Value::Tuple(a), _) => match other.to_tuple(ev)? {
                Some(b) => {
                    if a.elems.len() != b.elems.len() {
                        return Ok(a.elems.len().cmp(&b.elems.len()));
                    }
                    for (x, y) in a.elems.iter().zip(b.elems.iter()) {
                        let c = x.tlc_compare(ev, y)?;
                        if c != Ordering::Equal {
                            return Ok(c);
                        }
                    }
                    Ok(Ordering::Equal)
                }
                None => {
                    let f = self.to_fcn_rcd(ev)?.expect("a tuple converts to a function");
                    Value::FcnRcd(f).tlc_compare(ev, other)
                }
            },
            (Value::Record(a), _) => {
                let Some(b) = other.to_rcd(ev)? else {
                    if let Value::Model(m) = other {
                        return m.non_model_compare(ev, self);
                    }
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to compare record:\n{x}\nwith non-record\n{y}"), None));
                };
                let (an, av) = record_normalized(ev, a)?;
                let (bn, bv) = record_normalized(ev, &b)?;
                if an.len() != bn.len() {
                    return Ok(an.len().cmp(&bn.len()));
                }
                for (x, y) in an.iter().zip(bn.iter()) {
                    let c = x.tok().cmp(&y.tok());
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                for (x, y) in av.iter().zip(bv.iter()) {
                    let c = x.tlc_compare(ev, y)?;
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                Ok(Ordering::Equal)
            }
            (Value::FcnRcd(a), _) => {
                let Some(b) = other.to_fcn_rcd(ev)? else {
                    if let Value::Model(m) = other {
                        return m.non_model_compare(ev, self);
                    }
                    let (x, y) = (self.ppr(ev)?, other.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to compare the function {x} with the value:\n{y}"), None));
                };
                fcn_compare(ev, a, &b)
            }
            (Value::FcnLambda(a), _) => {
                let f = a.to_fcn_rcd(ev)?;
                Value::FcnRcd(f).tlc_compare(ev, other)
            }
            (Value::User(_), _) => self.user_compare(ev, other),
            // `SUBSET S` against `SUBSET T` compares the base sets (`SubsetValue.java:55-67`).
            (Value::Subset(a), Value::Subset(b)) => a.base.tlc_compare(ev, &b.base),
            (Value::KSubset(_), _) | (_, Value::KSubset(_)) => Err(EvalError::unimplemented("comparison of a k-subset (M5)".into())),
            _ if self.is_enumerable() => {
                let set = self.convert_and_cache(ev)?;
                Value::SetEnum(set).tlc_compare(ev, other)
            }
            _ => Err(EvalError::unimplemented(format!("comparison of {} (M2 unit 5)", self.kind_string()))),
        }
    }

    /// `member(elem)`: `elem \in self`.
    pub(crate) fn member(&self, ev: &mut Ev<'_>, elem: &Value) -> EvalResult<bool> {
        let not_a_set = |ev: &mut Ev<'_>, what: &str| -> EvalResult<bool> {
            let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
            Err(EvalError::detailed(format!("Attempted to check if the value:\n{e}\nis an element of the {what} {s}"), None))
        };
        match self {
            Value::Bool(_) => not_a_set(ev, "boolean"),
            Value::Int(_) => not_a_set(ev, "integer"),
            Value::Str(_) => not_a_set(ev, "string"),
            Value::Model(_) => not_a_set(ev, "model value"),
            // `UndefValue.member` leaves out the noun (`UndefValue.java:45-50`).
            Value::Undef => {
                let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                Err(EvalError::detailed(format!("Attempted to check if the value:\n{e}\nis an element {s}"), None))
            }
            // An untyped model value is simply not in the interval; a typed one gets the
            // interval's own text, not `modelValueMember`'s (`IntervalValue.java:74-93`).
            Value::Interval(i) => match elem {
                Value::Int(x) => Ok(*x >= i.lo && *x <= i.hi),
                _ if i.lo <= i.hi && !matches!(elem, Value::Model(m) if m.ty.is_none()) => {
                    let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                    Err(EvalError::detailed(format!("Attempted to check if the value:\n{e}\nis in the integer interval {s}"), None))
                }
                _ => Ok(false),
            },
            // `ValueVec.search`: binary search once normalised, `equals` scan before.
            Value::SetEnum(set) => match set.norm.get() {
                Some(norm) => {
                    let (mut lo, mut hi) = (0, norm.len());
                    while lo < hi {
                        let mid = lo + (hi - lo) / 2;
                        match elem.tlc_compare(ev, &norm[mid])? {
                            Ordering::Equal => return Ok(true),
                            Ordering::Less => hi = mid,
                            Ordering::Greater => lo = mid + 1,
                        }
                    }
                    Ok(false)
                }
                None => {
                    for e in set.built.iter() {
                        if e.tlc_equals(ev, elem)? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
            },
            Value::User(u) => match (&**u, elem) {
                (UserValue::Nat, Value::Int(x)) => Ok(*x >= 0),
                (UserValue::Int, Value::Int(_)) => Ok(true),
                (UserValue::String, Value::Str(_)) => Ok(true),
                // `Naturals.member` / `Integers.member` / `Strings.member` pass the `UserObj`
                // itself, whose `toString` is the same `Nat` / `Int` / `STRING`.
                (UserValue::Nat | UserValue::Int | UserValue::String, Value::Model(m)) => m.non_model_member(ev, self),
                (UserValue::Nat | UserValue::Int | UserValue::String, _) => {
                    let set = match **u {
                        UserValue::Nat => "Nat",
                        UserValue::Int => "Int",
                        _ => "STRING",
                    };
                    let shown = elem.ppr(ev)?;
                    Err(crate::modules::eval_exception(ev, Ec::TLC_MODULE_CHECK_MEMBER_OF, &[&shown, set]))
                }
                // `Sequences.member`: a sequence whose elements are all in the range
                // (`Sequences.java:392-412`).
                (UserValue::Seq(range), _) => {
                    let range = range.clone();
                    let Some(seq) = elem.to_tuple(ev)? else {
                        if let Value::Model(m) = elem {
                            return m.non_model_member(ev, self);
                        }
                        let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                        return Err(crate::modules::eval_exception(ev, Ec::TLC_MODULE_CHECK_MEMBER_OF, &[&e, &s]));
                    };
                    for e in seq.elems.iter() {
                        if !range.member(ev, e)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                _ => Err(EvalError::unimplemented("membership in this special set (M2 unit 5)".into())),
            },
            Value::Tuple(_) => Err(EvalError::detailed("Attempted to check set membership in a tuple value.".into(), None)),
            Value::Record(_) => {
                let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                Err(EvalError::detailed(format!("Attempted to check if element:\n{e}\nis in the record:\n{s}"), None))
            }
            Value::FcnRcd(_) | Value::FcnLambda(_) => {
                let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                Err(EvalError::detailed(format!("Attempted to check if the value:\n{e}\nis an element of the function {s}"), None))
            }
            // The lazy classes answer from their operands, never by enumerating themselves.
            Value::SetCup(s) => Ok(s.left.member(ev, elem)? || s.right.member(ev, elem)?),
            Value::SetCap(s) => Ok(s.left.member(ev, elem)? && s.right.member(ev, elem)?),
            Value::SetDiff(s) => Ok(s.left.member(ev, elem)? && !s.right.member(ev, elem)?),
            Value::Subset(s) => {
                if !elem.is_enumerable() {
                    let (e, p) = (elem.ppr(ev)?, self.ppr(ev)?);
                    return Err(EvalError::detailed(
                        format!("Attempted to check if the non-enumerable value\n{e}\nis element of\n{p}"),
                        None,
                    ));
                }
                let mut elems = elem.elements(ev, EnumOrder::Native)?.expect("an enumerable value enumerates");
                while let Some(e) = elems.next(ev)? {
                    if !s.base.member(ev, &e)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::KSubset(_) => Err(EvalError::unimplemented("membership in a k-subset (M5)".into())),
            Value::Union(u) => {
                if !u.set.is_enumerable() {
                    let (e, s) = (elem.ppr(ev)?, self.ppr(ev)?);
                    return Err(EvalError::detailed(
                        format!("Attempted to check if:\n {e}\nis an element of the non-enumerable set:\n {s}"),
                        None,
                    ));
                }
                let mut elems = u.set.elements(ev, EnumOrder::Native)?.expect("an enumerable value enumerates");
                while let Some(inner) = elems.next(ev)? {
                    if inner.member(ev, elem)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Value::SetPred(p) => {
                if let Some(set) = p.converted.get() {
                    return Value::SetEnum(set.clone()).member(ev, elem);
                }
                match set_pred_member(ev, p, elem) {
                    // `catch (EvalException e)` turns the predicate's own failure into a
                    // message about the membership test (`SetPredValue.java:141-145`).
                    Err(e) if matches!(e.kind, crate::error::EvalErrorKind::Eval) => {
                        let (x, s) = (elem.ppr(ev)?, p.inner.ppr(ev)?);
                        let pred = ev.tool().graph().expr_tlc_text(p.pred);
                        Err(EvalError::detailed(
                            format!("Cannot decide if element:\n{x}\n is element of:\n{s}\nand satisfies the predicate {pred}"),
                            None,
                        ))
                    }
                    other => other,
                }
            }
            // `f \in [D -> R]`: the domains must be equal and every value in the range
            // (`SetOfFcnsValue.java:113-143`).
            Value::SetOfFcns(s) => {
                let Some(fcn) = elem.to_fcn_rcd(ev)? else {
                    if let Value::Model(m) = elem {
                        return m.non_model_member(ev, self);
                    }
                    let (e, p) = (elem.to_tlc_string(ev)?, self.ppr(ev)?);
                    return Err(EvalError::detailed(
                        format!("Attempted to check if \n{e}\nwhich is not a TLC function value, is in the set of functions:\n{p}"),
                        None,
                    ));
                };
                let same_domain = match fcn.interval() {
                    Some(iv) => Value::Interval(iv).tlc_equals(ev, &s.domain)?,
                    None => {
                        let (domain, _) = fcn.normalize(ev)?;
                        let fdom = Value::set_enum_normalized(domain.expect("an explicit domain").to_vec());
                        s.domain.tlc_equals(ev, &fdom)?
                    }
                };
                if !same_domain {
                    return Ok(false);
                }
                for v in fcn.values.iter() {
                    if !s.range.member(ev, v)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::SetOfRcds(s) => {
                let Some(rcd) = elem.to_rcd(ev)? else {
                    if let Value::Model(m) = elem {
                        return m.non_model_member(ev, self);
                    }
                    let (e, p) = (elem.to_tlc_string(ev)?, self.ppr(ev)?);
                    return Err(EvalError::detailed(format!("Attempted to check if non-record\n{e}\nis in the set of records:\n{p}"), None));
                };
                let (names, values) = record_normalized(ev, &rcd)?;
                let (names, values) = (names.to_vec(), values.to_vec());
                if s.names.len() != names.len() {
                    return Ok(false);
                }
                for i in 0..s.names.len() {
                    if s.names[i] != names[i] || !s.sets[i].member(ev, &values[i])? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::SetOfTuples(s) => {
                let Some(tuple) = elem.to_tuple(ev)? else {
                    // A function whose domain is not all integers cannot be a tuple, and TLC
                    // reports it as a non-tuple (`SetOfTuplesValue.java:128-145`).
                    let Some(fcn) = elem.to_fcn_rcd(ev)? else {
                        if let Value::Model(m) = elem {
                            return m.non_model_member(ev, self);
                        }
                        let (e, p) = (elem.to_tlc_string(ev)?, self.ppr(ev)?);
                        return Err(EvalError::detailed(
                            format!("Attempted to check if non-tuple\n{e}\nis in the set of tuples:\n{p}"),
                            None,
                        ));
                    };
                    if fcn.interval().is_some() {
                        return Ok(false);
                    }
                    let (domain, _) = fcn.snapshot();
                    if !domain.expect("an explicit domain").iter().all(|d| matches!(d, Value::Int(_))) {
                        let (e, p) = (elem.to_tlc_string(ev)?, self.ppr(ev)?);
                        return Err(EvalError::detailed(
                            format!("Attempted to check if non-tuple\n{e}\nis in the set of tuples:\n{p}"),
                            None,
                        ));
                    }
                    return Ok(false);
                };
                if tuple.elems.len() != s.sets.len() {
                    return Ok(false);
                }
                for (set, e) in s.sets.iter().zip(tuple.elems.iter()) {
                    if !set.member(ev, e)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    /// `isFinite()`: what `FiniteSets!IsFiniteSet` answers, and what the set classes consult
    /// about their operands. The scalars fail instead of answering.
    pub(crate) fn is_finite(&self, ev: &mut Ev<'_>) -> EvalResult<bool> {
        let not_a_set = |ev: &mut Ev<'_>, noun: &str| -> EvalResult<bool> {
            let shown = self.ppr(ev)?;
            Err(EvalError::detailed(format!("Attempted to check if the {noun} {shown} is a finite set."), None))
        };
        match self {
            Value::Bool(_) => not_a_set(ev, "boolean"),
            Value::Int(_) => not_a_set(ev, "integer"),
            Value::Str(_) => not_a_set(ev, "string"),
            Value::Model(_) => not_a_set(ev, "model value"),
            Value::Undef => not_a_set(ev, "value"),
            Value::Interval(_) | Value::SetEnum(_) | Value::Tuple(_) | Value::Record(_) | Value::FcnRcd(_) => Ok(true),
            Value::FcnLambda(_) => {
                let shown = self.ppr(ev)?;
                Err(EvalError::detailed(format!("Attempted to check if the function:\n{shown}\nis a finite set."), None))
            }
            // `Naturals.Nat`, `Integers.Int`, `Strings.STRING` are infinite; `Seq(S)` is
            // finite only when `S` is empty (`Sequences.java:417-422`).
            Value::User(u) => match &**u {
                UserValue::Nat | UserValue::Int | UserValue::String => Ok(false),
                UserValue::Seq(range) => Ok(range.is_finite(ev)? && range.is_empty(ev)?),
                _ => Err(EvalError::unimplemented("isFinite of this special set (M2 unit 4)".into())),
            },
            Value::Subset(s) => s.base.is_finite(ev),
            Value::KSubset(s) => s.base.is_finite(ev),
            Value::SetCup(s) => Ok(s.left.is_finite(ev)? && s.right.is_finite(ev)?),
            // The text has no space before "is", as the concatenation in TLC lacks one.
            Value::SetCap(s) => {
                if !s.left.is_finite(ev)? && !s.right.is_finite(ev)? {
                    let shown = self.ppr(ev)?;
                    return Err(EvalError::detailed(format!("Attempted to check if the set {shown}is finite."), None));
                }
                Ok(true)
            }
            Value::SetDiff(s) => {
                if s.left.is_finite(ev)? {
                    return Ok(true);
                }
                if !s.right.is_finite(ev)? {
                    let shown = self.ppr(ev)?;
                    return Err(EvalError::detailed(format!("Attempted to check if the set {shown}is finite."), None));
                }
                Ok(false)
            }
            // `[S -> T]` is finite when either side is empty, when both are finite, or when
            // the range has one element (`SetOfFcnsValue.java:171-186`).
            Value::SetOfFcns(s) => {
                let finite_domain = s.domain.is_finite(ev)?;
                if finite_domain && s.domain.is_empty(ev)? {
                    return Ok(true);
                }
                let finite_range = s.range.is_finite(ev)?;
                if finite_range && s.range.is_empty(ev)? {
                    return Ok(true);
                }
                if finite_domain && finite_range {
                    return Ok(true);
                }
                Ok(finite_range && s.range.size(ev)? == 1)
            }
            Value::SetOfRcds(s) => all_finite(ev, &s.sets),
            Value::SetOfTuples(s) => all_finite(ev, &s.sets),
            Value::Union(u) => {
                if !u.set.is_enumerable() {
                    let shown = self.ppr(ev)?;
                    return Err(EvalError::detailed(format!("Attempted to check if the nonenumerable set:\n{shown}\nis a finite set."), None));
                }
                let mut elems = u.set.elements(ev, EnumOrder::Native)?.expect("an enumerable value enumerates");
                while let Some(v) = elems.next(ev)? {
                    if !v.is_finite(ev)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::SetPred(p) => {
                if !p.inner.is_finite(ev)? {
                    let shown = p.inner.ppr(ev)?;
                    return Err(EvalError::detailed(
                        format!("Attempted to check if expression of form {{x \\in S : p(x)}} is a finite set, but cannot check if S:\n{shown}\nis finite."),
                        None,
                    ));
                }
                Ok(true)
            }
        }
    }

    /// `Value.isEmpty()` (`Value.java:150-254`): a per-class rule, cheap where the class
    /// knows the answer without enumerating.
    pub(crate) fn is_empty(&self, ev: &mut Ev<'_>) -> EvalResult<bool> {
        let first_is_none = |ev: &mut Ev<'_>, v: &Value| -> EvalResult<bool> {
            let mut elems = v.elements(ev, EnumOrder::Native)?.expect("an enumerable value enumerates");
            Ok(elems.next(ev)?.is_none())
        };
        match self {
            // Normalisation only drops duplicates, so the constructed elements answer this.
            Value::SetEnum(s) => Ok(s.built.is_empty()),
            Value::Interval(i) => Ok(i.size(ev)? == 0),
            Value::SetCap(_) | Value::SetCup(_) | Value::SetDiff(_) | Value::Union(_) | Value::SetPred(_) => first_is_none(ev, self),
            // `[{} -> T]` is `{<<>>}` for every `T`, which is why an empty domain rules the
            // emptiness out instead of implying it.
            Value::SetOfFcns(s) => Ok(!s.domain.is_empty(ev)? && s.range.is_empty(ev)?),
            Value::SetOfRcds(s) => any_empty(ev, &s.sets),
            Value::SetOfTuples(s) => any_empty(ev, &s.sets),
            // `SUBSET S` always contains `{}`.
            Value::Subset(_) => Ok(false),
            Value::KSubset(_) => Err(EvalError::unimplemented("KSubsetValue.hasNoElements (M5)".into())),
            Value::User(u) => match &**u {
                UserValue::Nat | UserValue::Int | UserValue::String => Ok(false),
                UserValue::Seq(_) => Ok(false),
                _ => Err(EvalError::unimplemented("isEmpty of this special set (M2 unit 4)".into())),
            },
            _ => {
                let shown = self.ppr(ev)?;
                Err(EvalError::detailed(format!("Shouldn't call isEmpty() on value {shown}"), None))
            }
        }
    }

    /// `size()`: the cardinality a set reports and the number of points a function has.
    pub(crate) fn size(&self, ev: &mut Ev<'_>) -> EvalResult<i32> {
        let uncountable = |ev: &mut Ev<'_>, noun: &str| -> EvalResult<i32> {
            let shown = self.ppr(ev)?;
            Err(EvalError::detailed(format!("Attempted to compute the number of elements in the {noun} {shown}."), None))
        };
        match self {
            Value::Bool(_) => uncountable(ev, "boolean"),
            Value::Int(_) => uncountable(ev, "integer"),
            Value::Str(_) => uncountable(ev, "string"),
            Value::Model(_) => uncountable(ev, "model value"),
            Value::User(_) => uncountable(ev, "overridden value"),
            Value::Undef => uncountable(ev, "value"),
            Value::Interval(i) => i.size(ev),
            Value::SetEnum(s) => Ok(normalized(ev, s)?.len() as i32),
            Value::Tuple(t) => Ok(t.elems.len() as i32),
            Value::Record(r) => Ok(r.names.len() as i32),
            Value::FcnRcd(f) => {
                f.normalize(ev)?;
                Ok(f.values.len() as i32)
            }
            Value::FcnLambda(f) => match f.converted.get() {
                Some(r) => Value::FcnRcd(r.clone()).size(ev),
                None => f.params_size(ev),
            },
            // `2^|S|`, refused above 31 elements because the result must fit an int
            // (`SubsetValue.java:184-191`).
            Value::Subset(s) => {
                let sz = s.base.size(ev)?;
                if sz >= 31 {
                    return Err(self.overflow_code(ev)?);
                }
                Ok(1 << sz)
            }
            Value::KSubset(_) => Err(EvalError::unimplemented("the cardinality of a k-subset (M5)".into())),
            // `|R|^|D|`, with the unit and empty cases first (`SetOfFcnsValue.java:242-263`).
            Value::SetOfFcns(s) => {
                if s.domain.is_empty(ev)? {
                    return Ok(1);
                }
                if s.range.is_empty(ev)? {
                    return Ok(0);
                }
                let rsz = s.range.size(ev)?;
                if rsz == 1 {
                    return Ok(1);
                }
                let dsz = s.domain.size(ev)?;
                let mut sz: i64 = 1;
                for _ in 0..dsz {
                    sz *= i64::from(rsz);
                    if sz < i64::from(i32::MIN) || sz > i64::from(i32::MAX) {
                        let shown = self.ppr(ev)?;
                        return Err(EvalError::detailed(format!("Overflow when computing the number of elements in:\n{shown}"), None));
                    }
                }
                Ok(sz as i32)
            }
            Value::SetOfRcds(s) => {
                if self.is_empty(ev)? {
                    return Ok(0);
                }
                self.product_size(ev, &s.sets, None)
            }
            Value::SetOfTuples(s) => {
                if self.is_empty(ev)? {
                    return Ok(0);
                }
                let sets = s.sets.clone();
                self.product_size(ev, &sets, Some("Overflow when computing the number of elements in "))
            }
            // The rest report the cardinality of their enumerated form.
            Value::SetCup(_) | Value::SetCap(_) | Value::SetDiff(_) | Value::Union(_) | Value::SetPred(_) => {
                let set = self.convert_and_cache(ev)?;
                Value::SetEnum(set).size(ev)
            }
        }
    }

    /// `Assert.fail(EC.TLC_MODULE_OVERFLOW, "the number of elements in:\n" + ppr)`.
    fn overflow_code(&self, ev: &mut Ev<'_>) -> EvalResult<EvalError> {
        let shown = self.ppr(ev)?;
        let text = ev.out().text(Ec::TLC_MODULE_OVERFLOW, &[&format!("the number of elements in:\n{shown}")]);
        Ok(EvalError::runtime(Ec::TLC_MODULE_OVERFLOW, text))
    }

    /// The product of the constituents' cardinalities. A set of records reports the overflow
    /// through 2178, a product of sets through its own text (`SetOfTuplesValue.java:231-244`).
    fn product_size(&self, ev: &mut Ev<'_>, sets: &[Value], plain: Option<&str>) -> EvalResult<i32> {
        let mut sz: i64 = 1;
        for set in sets {
            sz *= i64::from(set.size(ev)?);
            if sz < i64::from(i32::MIN) || sz > i64::from(i32::MAX) {
                return match plain {
                    Some(prefix) => {
                        let shown = self.ppr(ev)?;
                        Err(EvalError::detailed(format!("{prefix}{shown}"), None))
                    }
                    None => Err(self.overflow_code(ev)?),
                };
            }
        }
        Ok(sz as i32)
    }
}

/// `SetPredValue.member` without its `EvalException` handler: the element must be in the
/// inner set and satisfy the predicate (`SetPredValue.java:108-146`).
fn set_pred_member(ev: &mut Ev<'_>, p: &Arc<super::SetPredValue>, elem: &Value) -> EvalResult<bool> {
    if !p.inner.member(ev, elem)? {
        return Ok(false);
    }
    let c1 = p.bind(ev, elem)?;
    let (s0, s1) = p.env.states();
    match ev.eval(p.pred, &c1, s0, s1, p.env.control)? {
        Value::Bool(b) => Ok(b),
        _ => {
            let pred = ev.tool().graph().expr_tlc_text(p.pred);
            Err(EvalError::detailed(format!("The evaluation of predicate {pred} yielded non-Boolean value."), None))
        }
    }
}

/// `isFinite()` of a set of records or of tuples: an empty constituent makes the whole set
/// empty, hence finite, even when another constituent is infinite
/// (`SetOfRcdsValue.java:138-149`).
fn all_finite(ev: &mut Ev<'_>, sets: &[Value]) -> EvalResult<bool> {
    let mut all = true;
    for s in sets {
        if s.is_finite(ev)? {
            if s.is_empty(ev)? {
                return Ok(true);
            }
        } else {
            all = false;
        }
    }
    Ok(all)
}

fn any_empty(ev: &mut Ev<'_>, sets: &[Value]) -> EvalResult<bool> {
    for s in sets {
        if s.is_empty(ev)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `SetEnumValue.normalize()` / `ValueVec.sort(true)`: binary insertion, dedup on
/// `compareTo == 0` (the element already placed stays). Publishes into `norm` on success
/// only.
pub(crate) fn normalized<'a>(ev: &mut Ev<'_>, set: &'a SetEnumValue) -> EvalResult<&'a [Value]> {
    if let Some(norm) = set.norm.get() {
        return Ok(norm);
    }
    let mut out: Vec<Value> = Vec::with_capacity(set.built.len());
    let mut elems = set.built.iter();
    if let Some(first) = elems.next() {
        out.push(first.clone());
    }
    for elem in elems {
        let (mut lo, mut hi, mut idx) = (0, out.len(), 0);
        let mut cmp = Ordering::Equal;
        while lo < hi {
            idx = lo + (hi - lo) / 2;
            cmp = elem.tlc_compare(ev, &out[idx])?;
            match cmp {
                Ordering::Equal => break,
                Ordering::Less => hi = idx,
                Ordering::Greater => lo = idx + 1,
            }
        }
        if cmp != Ordering::Equal {
            out.insert(if cmp == Ordering::Less { idx } else { idx + 1 }, elem.clone());
        }
    }
    Ok(set.norm.get_or_init(|| out.into_boxed_slice()))
}

/// `RecordValue.normalize()` / `sort()` (`RecordValue.java:619-651`): the smallest field
/// name is swapped to the front, then insertion sort, both in place, so the duplicate the
/// error names is read from the partly sorted array as TLC reads it.
pub(crate) fn record_normalized<'a>(ev: &mut Ev<'_>, r: &'a RecordValue) -> EvalResult<(&'a [Atom], &'a [Value])> {
    if let Some((names, values)) = r.norm.get() {
        return Ok((names, values));
    }
    let mut names = r.names.to_vec();
    let mut values = r.values.to_vec();
    let duplicate = |ev: &mut Ev<'_>, name: Atom| {
        let text = ev.tool().graph().interner().text(name);
        EvalError::detailed(format!("Field name {text} occurs multiple times in record."), None)
    };
    for i in 1..names.len() {
        match names[0].tok().cmp(&names[i].tok()) {
            Ordering::Equal => return Err(duplicate(ev, names[i])),
            Ordering::Greater => {
                names.swap(0, i);
                values.swap(0, i);
            }
            Ordering::Less => {}
        }
    }
    for i in 2..names.len() {
        let (name, value) = (names[i], values[i].clone());
        let mut j = i;
        let mut cmp = Ordering::Less;
        while j > 0 {
            cmp = name.tok().cmp(&names[j - 1].tok());
            if cmp != Ordering::Less {
                break;
            }
            names[j] = names[j - 1];
            values[j] = values[j - 1].clone();
            j -= 1;
        }
        if cmp == Ordering::Equal {
            return Err(duplicate(ev, names[i]));
        }
        names[j] = name;
        values[j] = value;
    }
    let _ = r.norm.set((names.into_boxed_slice(), values.into_boxed_slice()));
    let (names, values) = r.norm.get().expect("just published");
    Ok((names, values))
}

/// `FcnRcdValue.normalize()` / `sort()` (`FcnRcdValue.java:714-749`), the same algorithm over
/// an explicit domain. Only the explicit case sorts: an interval domain is already ordered.
pub(crate) fn fcn_normalized<'a>(ev: &mut Ev<'_>, f: &'a FcnRcdValue) -> EvalResult<(&'a [Value], &'a [Value])> {
    if let Some((domain, values)) = f.norm.get() {
        return Ok((domain, values));
    }
    let super::FcnDomain::Explicit(built) = &f.domain else { unreachable!("an interval domain is normalised") };
    let mut domain = built.to_vec();
    let mut values = f.values.to_vec();
    fcn_sort(ev, &mut domain, &mut values)?;
    let _ = f.norm.set((domain.into_boxed_slice(), values.into_boxed_slice()));
    let (domain, values) = f.norm.get().expect("just published");
    Ok((domain, values))
}

/// `FcnRcdValue.sort()` in place. `toString` runs it over a private copy, so a function the
/// evaluator never normalises keeps printing in construction order.
pub(crate) fn fcn_sort(ev: &mut Ev<'_>, domain: &mut [Value], values: &mut [Value]) -> EvalResult<()> {
    let duplicate = |ev: &mut Ev<'_>, v: &Value| -> EvalResult<EvalError> {
        let shown = v.to_tlc_string(ev)?;
        Ok(EvalError::detailed(format!("The value\n{shown}\noccurs multiple times in the function domain."), None))
    };
    for i in 1..domain.len() {
        match domain[0].tlc_compare(ev, &domain[i])? {
            Ordering::Equal => return Err(duplicate(ev, &domain[i].clone())?),
            Ordering::Greater => {
                domain.swap(0, i);
                values.swap(0, i);
            }
            Ordering::Less => {}
        }
    }
    for i in 2..domain.len() {
        let (d, v) = (domain[i].clone(), values[i].clone());
        let mut j = i;
        let mut cmp = Ordering::Less;
        while j > 0 {
            cmp = d.tlc_compare(ev, &domain[j - 1])?;
            if cmp != Ordering::Less {
                break;
            }
            domain[j] = domain[j - 1].clone();
            values[j] = values[j - 1].clone();
            j -= 1;
        }
        if cmp == Ordering::Equal {
            return Err(duplicate(ev, &domain[i].clone())?);
        }
        domain[j] = d;
        values[j] = v;
    }
    Ok(())
}

/// `IntervalValue.equals(IntervalValue)`: every empty interval is the same set. The size
/// of the receiver is read first, so an interval too large to count fails here.
fn interval_equals(ev: &mut Ev<'_>, a: IntervalValue, b: IntervalValue) -> EvalResult<bool> {
    if a.size(ev)? == 0 {
        return Ok(b.size(ev)? == 0);
    }
    Ok(a.lo == b.lo && a.hi == b.hi)
}

/// `FcnRcdValue.equals` (`FcnRcdValue.java:261-340`). An interval domain is compared against
/// an explicit one element by element rather than being materialised.
fn fcn_equals(ev: &mut Ev<'_>, a: &FcnRcdValue, b: &FcnRcdValue) -> EvalResult<bool> {
    let (ad, av) = a.normalize(ev)?;
    let (bd, bv) = b.normalize(ev)?;
    let not_integer = |ev: &mut Ev<'_>, v: &Value| -> EvalResult<EvalError> {
        let shown = v.ppr(ev)?;
        Ok(EvalError::detailed(format!("Attempted to compare an integer with non-integer:\n{shown}."), None))
    };
    // The side with the interval walks the other side's domain as lo, lo+1, ...
    let (interval, explicit) = match (a.interval(), b.interval()) {
        (Some(ai), Some(bi)) => {
            if !interval_equals(ev, ai, bi)? {
                return Ok(false);
            }
            return values_equal(ev, av, bv);
        }
        (Some(ai), None) => {
            let bd = bd.expect("an explicit domain");
            if bd.len() as i64 != i64::from(ai.size(ev)?) {
                return Ok(false);
            }
            (ai, bd)
        }
        (None, Some(bi)) => {
            if av.len() != bv.len() {
                return Ok(false);
            }
            (bi, ad.expect("an explicit domain"))
        }
        (None, None) => {
            if av.len() != bv.len() {
                return Ok(false);
            }
            let (ad, bd) = (ad.expect("an explicit domain"), bd.expect("an explicit domain"));
            for (x, y) in ad.iter().zip(bd.iter()) {
                if !x.tlc_equals(ev, y)? {
                    return Ok(false);
                }
            }
            return values_equal(ev, av, bv);
        }
    };
    for (i, d) in explicit.iter().enumerate() {
        let Value::Int(n) = d else { return Err(not_integer(ev, d)?) };
        if i64::from(*n) != i64::from(interval.lo) + i as i64 {
            return Ok(false);
        }
    }
    values_equal(ev, av, bv)
}

fn values_equal(ev: &mut Ev<'_>, av: &[Value], bv: &[Value]) -> EvalResult<bool> {
    for (x, y) in av.iter().zip(bv.iter()) {
        if !x.tlc_equals(ev, y)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `FcnRcdValue.compareTo` (`FcnRcdValue.java:149-259`): size, then domain, then values. The
/// "non-integer" text differs by one colon between the two directions, as in TLC.
fn fcn_compare(ev: &mut Ev<'_>, a: &FcnRcdValue, b: &FcnRcdValue) -> EvalResult<Ordering> {
    let (ad, av) = a.normalize(ev)?;
    let (bd, bv) = b.normalize(ev)?;
    if av.len() != bv.len() {
        return Ok(av.len().cmp(&bv.len()));
    }
    match (a.interval(), b.interval()) {
        (Some(ai), bi) => {
            if av.is_empty() {
                return Ok(Ordering::Equal);
            }
            match bi {
                Some(bi) => {
                    let c = ai.lo.cmp(&bi.lo);
                    if c != Ordering::Equal {
                        return Ok(c);
                    }
                }
                None => {
                    for (i, d) in bd.expect("an explicit domain").iter().enumerate() {
                        let Value::Int(n) = d else {
                            let shown = d.ppr(ev)?;
                            return Err(EvalError::detailed(format!("Attempted to compare integer with non-integer:\n{shown}."), None));
                        };
                        let c = (i64::from(ai.lo) + i as i64).cmp(&i64::from(*n));
                        if c != Ordering::Equal {
                            return Ok(c);
                        }
                    }
                }
            }
            values_compare(ev, av, bv)
        }
        (None, Some(bi)) => {
            for (i, d) in ad.expect("an explicit domain").iter().enumerate() {
                let Value::Int(n) = d else {
                    let shown = d.ppr(ev)?;
                    return Err(EvalError::detailed(format!("Attempted to compare integer with non-integer\n{shown}."), None));
                };
                let c = i64::from(*n).cmp(&(i64::from(bi.lo) + i as i64));
                if c != Ordering::Equal {
                    return Ok(c);
                }
            }
            values_compare(ev, av, bv)
        }
        (None, None) => {
            let (ad, bd) = (ad.expect("an explicit domain"), bd.expect("an explicit domain"));
            for (x, y) in ad.iter().zip(bd.iter()) {
                let c = x.tlc_compare(ev, y)?;
                if c != Ordering::Equal {
                    return Ok(c);
                }
            }
            values_compare(ev, av, bv)
        }
    }
}

fn values_compare(ev: &mut Ev<'_>, av: &[Value], bv: &[Value]) -> EvalResult<Ordering> {
    for (x, y) in av.iter().zip(bv.iter()) {
        let c = x.tlc_compare(ev, y)?;
        if c != Ordering::Equal {
            return Ok(c);
        }
    }
    Ok(Ordering::Equal)
}

/// The one representation-independent view that `tlc_equals`, `tlc_compare` and
/// `fingerprint` are written against: sets as normalized element slices, functions
/// (tuples, records, FcnRcd, materialized FcnLambda) as normalized `(key, value)` pairs.
pub(crate) enum Canon<'v> {
    Scalar(&'v Value),
    Set(Arc<SetEnumValue>),
    Fcn(Arc<super::FcnRcdValue>),
}

pub(crate) fn canon<'v>(ev: &mut Ev<'_>, v: &'v Value) -> EvalResult<Canon<'v>> {
    todo!("to_set_enum / to_fcn_rcd, error text from the receiver's class")
}

/// Run `fast`; in debug builds also run `slow` (the `Canon` path) and assert equality.
pub(crate) fn fast_checked<T: PartialEq + std::fmt::Debug>(
    ev: &mut Ev<'_>,
    fast: impl FnOnce(&mut Ev<'_>) -> EvalResult<T>,
    slow: impl FnOnce(&mut Ev<'_>) -> EvalResult<T>,
) -> EvalResult<T> {
    let answer = fast(ev)?;
    if cfg!(debug_assertions) {
        assert_eq!(answer, slow(ev)?, "representation fast path disagrees with Canon");
    }
    Ok(answer)
}
