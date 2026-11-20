//! `FunctionValue`: the four classes TLA+ applies as functions, and what EXCEPT does to any
//! value. The single owner of:
//! - the conversions between the function representations (`toTuple`, `toRcd`, `toFcnRcd`).
//!   They narrow to `None` instead of converting, which is why `<<1, 2>> = [i \in 1..2 |-> i]`
//!   holds while `[a |-> 1] = <<1>>` is an error;
//! - `apply` / `select` / `getDomain` per representation, including FcnRcd's binary search
//!   over a normalised domain of at least `LINEAR_SEARCH_THRESHOLD` elements;
//! - `FcnLambdaValue`'s laziness: the materialisation of its domain, the EXCEPT clauses it
//!   carries unapplied, and recursion through `rec_name`;
//! - `takeExcept` for every value, and every "Attempted to apply ..." / "In applying the
//!   function ..." text.

use super::order::{fcn_normalized, normalized, record_normalized};
use super::{Closure, FcnDomain, FcnLambdaValue, FcnRcdValue, IntervalValue, RecordValue, SetOfTuplesValue, TupleValue, Value};
use crate::ctx::{Binding, Ctx};
use crate::error::{EvalError, EvalResult};
use crate::eval::{EvalControl, Ev};
use crate::value::enumerate::{EnumOrder, ValueEnumeration};
use std::cmp::Ordering;
use std::sync::{Arc, OnceLock};
use tlars_msg::Ec;
use tlars_sany::Atom;
use tlars_sany::graph::{ParamId, SymbolRef};

/// `-Dtlc2.value.impl.FcnRcdValue.threshold`, whose default decides whether `select` binary
/// searches a normalised domain (`FcnRcdValue.java:35`).
const LINEAR_SEARCH_THRESHOLD: usize = 32;

/// `ValueExcept`: one `![a][b] = v` clause of an EXCEPT, with the cursor into its path that
/// `takeExcept` advances as it descends.
#[derive(Clone)]
pub(crate) struct Except {
    pub(crate) path: Arc<[Value]>,
    pub(crate) value: Value,
    pub(crate) idx: usize,
}

impl Except {
    pub(crate) fn new(path: Vec<Value>, value: Value) -> Except {
        Except { path: path.into(), value, idx: 0 }
    }
    fn current(&self) -> &Value {
        &self.path[self.idx]
    }
    fn is_last(&self) -> bool {
        self.idx + 1 == self.path.len()
    }
    fn done(&self) -> bool {
        self.idx >= self.path.len()
    }
    fn at(&self, idx: usize) -> Except {
        Except { path: self.path.clone(), value: self.value.clone(), idx }
    }
}

/// `(TupleValue) v` on a value that is not one.
fn not_a_tuple_cast(v: &Value) -> EvalError {
    EvalError::class_cast(v.java_class(), "TupleValue")
}

impl Value {
    /// `FunctionValue`: what `DOMAIN`, `f[e]` and EXCEPT accept.
    pub(crate) fn is_function(&self) -> bool {
        matches!(self, Value::Tuple(_) | Value::Record(_) | Value::FcnRcd(_) | Value::FcnLambda(_))
    }

    /// `toTuple()`.
    pub(crate) fn to_tuple(&self, ev: &mut Ev<'_>) -> EvalResult<Option<Arc<TupleValue>>> {
        match self {
            Value::Tuple(t) => Ok(Some(t.clone())),
            Value::Record(r) => Ok(r.names.is_empty().then(empty_tuple)),
            Value::FcnRcd(f) => Ok(f.to_tuple()),
            Value::FcnLambda(f) => f.to_tuple(ev),
            _ => Ok(None),
        }
    }

    /// `toRcd()`.
    pub(crate) fn to_rcd(&self, ev: &mut Ev<'_>) -> EvalResult<Option<Arc<RecordValue>>> {
        match self {
            Value::Record(r) => Ok(Some(r.clone())),
            Value::Tuple(t) => Ok(t.elems.is_empty().then(empty_rcd)),
            Value::FcnRcd(f) => f.to_rcd(ev),
            // `FcnLambdaValue.toRcd` materialises first (`FcnLambdaValue.java:635-649`).
            Value::FcnLambda(f) => f.to_fcn_rcd(ev)?.to_rcd(ev),
            _ => Ok(None),
        }
    }

    /// `toFcnRcd()`.
    pub(crate) fn to_fcn_rcd(&self, ev: &mut Ev<'_>) -> EvalResult<Option<Arc<FcnRcdValue>>> {
        match self {
            Value::FcnRcd(f) => Ok(Some(f.clone())),
            Value::Tuple(t) => {
                let iv = IntervalValue { lo: 1, hi: t.elems.len() as i32 };
                Ok(Some(Value::fcn_rcd_interval_arc(iv, t.elems.to_vec())))
            }
            Value::Record(r) => {
                let (names, values) = record_normalized(ev, r)?;
                let domain = names.iter().map(|n| Value::Str(*n)).collect();
                Ok(Some(Value::fcn_rcd_arc(domain, values.to_vec(), true)))
            }
            Value::FcnLambda(f) => f.to_fcn_rcd(ev).map(Some),
            _ => Ok(None),
        }
    }

    /// `FunctionValue.apply(arg, control)`.
    pub(crate) fn apply(&self, ev: &mut Ev<'_>, arg: &Value, control: EvalControl) -> EvalResult<Value> {
        match self {
            Value::Tuple(t) => t.apply(ev, arg),
            Value::Record(r) => r.apply(ev, arg),
            Value::FcnRcd(f) => match f.select(ev, arg)? {
                Some(v) => Ok(v),
                None => {
                    let (shown, a) = (self.ppr(ev)?, arg.ppr(ev)?);
                    Err(EvalError::detailed(
                        format!("Attempted to apply function:\n{shown}\nto argument {a}, which is not in the domain of the function."),
                        None,
                    ))
                }
            },
            Value::FcnLambda(f) => f.apply(ev, arg, control),
            _ => unreachable!("apply is reached through is_function"),
        }
    }

    /// `FunctionValue.select(arg)`: `None` = not in the domain.
    pub(crate) fn select(&self, ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Option<Value>> {
        match self {
            Value::Tuple(t) => t.select(ev, arg),
            Value::Record(r) => r.select(ev, arg),
            Value::FcnRcd(f) => f.select(ev, arg),
            Value::FcnLambda(f) => f.select(ev, arg),
            _ => unreachable!("select is reached through is_function"),
        }
    }

    /// `Value.select(path)` (`Value.java:319-337`): the `@` of an EXCEPT component.
    pub(crate) fn select_path(&self, ev: &mut Ev<'_>, path: &[Value]) -> EvalResult<Option<Value>> {
        let mut result = self.clone();
        for elem in path {
            if !result.is_function() {
                let shown = result.ppr(ev)?;
                return Err(EvalError::detailed(format!("Attempted to apply EXCEPT construct to the value {shown}."), None));
            }
            match result.select(ev, elem)? {
                Some(v) => result = v,
                None => return Ok(None),
            }
        }
        Ok(Some(result))
    }

    /// `FunctionValue.getDomain()`.
    pub(crate) fn get_domain(&self, ev: &mut Ev<'_>) -> EvalResult<Value> {
        match self {
            Value::Tuple(t) => Ok(Value::Interval(IntervalValue { lo: 1, hi: t.elems.len() as i32 })),
            // The unsorted snapshot's names, so the domain is normalised exactly when the
            // record is (`RecordValue.java:543-557`).
            Value::Record(r) => {
                let elems = r.snapshot().0.iter().map(|n| Value::Str(*n)).collect();
                Ok(if r.is_normalized() { Value::set_enum_normalized(elems) } else { Value::set_enum(elems) })
            }
            Value::FcnRcd(f) => match f.domain {
                FcnDomain::Interval(iv) => Ok(Value::Interval(iv)),
                FcnDomain::Explicit(_) => {
                    let (domain, _) = f.normalize(ev)?;
                    Ok(Value::set_enum_normalized(domain.expect("an explicit domain").to_vec()))
                }
            },
            Value::FcnLambda(f) => f.get_domain(ev),
            _ => unreachable!("get_domain is reached through is_function"),
        }
    }

    /// `takeExcept(ValueExcept)`.
    pub(crate) fn take_except(&self, ev: &mut Ev<'_>, ex: &mut Except) -> EvalResult<Value> {
        match self {
            Value::Tuple(t) => t.take_except(ev, ex),
            Value::Record(r) => r.take_except(ev, ex),
            Value::FcnRcd(f) => f.take_except(ev, ex),
            Value::FcnLambda(f) => f.take_except(ev, ex),
            _ => {
                if !ex.done() {
                    return Err(self.not_exceptable(ev)?);
                }
                Ok(ex.value.clone())
            }
        }
    }

    /// `takeExcept(ValueExcept[])`.
    pub(crate) fn take_excepts(&self, ev: &mut Ev<'_>, exs: &mut [Except]) -> EvalResult<Value> {
        match self {
            Value::FcnLambda(f) => f.take_excepts(ev, exs),
            Value::Tuple(_) | Value::Record(_) | Value::FcnRcd(_) => {
                let mut res = self.clone();
                for ex in exs.iter_mut() {
                    res = res.take_except(ev, ex)?;
                }
                Ok(res)
            }
            _ => {
                if !exs.is_empty() {
                    return Err(self.not_exceptable(ev)?);
                }
                Ok(self.clone())
            }
        }
    }

    /// The `Assert.fail` text of the classes that have no EXCEPT: a per-class noun, and
    /// "EXCEPT" or "EXCEPT construct" depending on the class.
    fn not_exceptable(&self, ev: &mut Ev<'_>) -> EvalResult<EvalError> {
        let shown = self.ppr(ev)?;
        let reason = match self {
            Value::Bool(_) => format!("Attempted to apply EXCEPT construct to the boolean {shown}."),
            Value::Int(_) => format!("Attempted to apply EXCEPT construct to the integer {shown}."),
            Value::Str(_) => format!("Attempted to apply EXCEPT construct to the string {shown}."),
            Value::Interval(_) => format!("Attempted to apply EXCEPT construct to the interval value {shown}."),
            Value::Model(_) => format!("Attempted to apply EXCEPT construct to the model value {shown}."),
            Value::Undef => format!("Attempted to apply EXCEPT construct to the value {shown}."),
            Value::User(_) => format!("Attempted to apply EXCEPT to the overridden value {shown}."),
            Value::SetOfFcns(_) => format!("Attempted to apply EXCEPT to the set of functions:\n{shown}"),
            Value::SetOfRcds(_) => format!("Attempted to apply EXCEPT to the set of records:\n{shown}"),
            Value::SetOfTuples(_) => format!("Attempted to apply EXCEPT construct to the set of tuples:\n{shown}"),
            Value::Union(_) => format!("Attempted to apply EXCEPT to the set:\n{shown}"),
            _ => format!("Attempted to apply EXCEPT to the set {shown}."),
        };
        Ok(EvalError::detailed(reason, None))
    }
}

/// `TupleValue.EmptyTuple`.
fn empty_tuple() -> Arc<TupleValue> {
    Arc::new(TupleValue { elems: Box::new([]) })
}

/// `RecordValue.EmptyRcd`.
fn empty_rcd() -> Arc<RecordValue> {
    Value::record_arc(Vec::new(), Vec::new(), true)
}

/// `apply` and `select` share the index check (`TupleValue.java:134-182`).
fn tuple_index(ev: &mut Ev<'_>, arg: &Value) -> EvalResult<i32> {
    match arg {
        Value::Int(i) => Ok(*i),
        _ => {
            let shown = arg.ppr(ev)?;
            Err(EvalError::detailed(format!("Attempted to access tuple at a non integral index: {shown}"), None))
        }
    }
}

/// `apply` and `select` share the field check (`RecordValue.java:473-521`).
fn record_field(ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Atom> {
    match arg {
        Value::Str(a) => Ok(*a),
        _ => {
            let shown = arg.ppr(ev)?;
            Err(EvalError::detailed(format!("Attempted to access record by a non-string argument: {shown}"), None))
        }
    }
}

impl TupleValue {
    fn apply(self: &Arc<Self>, ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Value> {
        let idx = tuple_index(ev, arg)?;
        if idx <= 0 || idx as usize > self.elems.len() {
            let shown = Value::Tuple(self.clone()).ppr(ev)?;
            return Err(EvalError::detailed(format!("Attempted to access index {idx} of tuple\n{shown}\nwhich is out of bounds."), None));
        }
        Ok(self.elems[idx as usize - 1].clone())
    }

    fn select(&self, ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Option<Value>> {
        let idx = tuple_index(ev, arg)?;
        Ok(if idx > 0 && idx as usize <= self.elems.len() { Some(self.elems[idx as usize - 1].clone()) } else { None })
    }

    fn take_except(&self, ev: &mut Ev<'_>, ex: &mut Except) -> EvalResult<Value> {
        if !ex.done() {
            let arc = ex.current().clone();
            if let Value::Int(n) = arc {
                let idx = i64::from(n) - 1;
                if idx < 0 || idx >= self.elems.len() as i64 {
                    // TLC returns a tuple of Java nulls here (`TupleValue.java:189-201`),
                    // which fails on first use with a JVM message. `$Except` cannot reach
                    // it: `select` of the same path answers null first and only warns.
                    return Err(EvalError::unimplemented("TLC's tuple of nulls from an out-of-bounds EXCEPT index".into()));
                }
                let mut elems = self.elems.to_vec();
                ex.idx += 1;
                elems[idx as usize] = self.elems[idx as usize].take_except(ev, ex)?;
                return Ok(Value::tuple(elems));
            }
            let shown = arc.ppr(ev)?;
            warn(ev, Ec::TLC_WRONG_TUPLE_FIELD_NAME, &[&shown])?;
        }
        Ok(ex.value.clone())
    }
}

impl RecordValue {
    /// `isNormalized()`.
    pub(crate) fn is_normalized(&self) -> bool {
        self.norm.get().is_some()
    }

    /// `snapshot()`: the sorted pair once `normalize()` has run, the constructed one before.
    pub(crate) fn snapshot(&self) -> (&[Atom], &[Value]) {
        match self.norm.get() {
            Some((names, values)) => (names, values),
            None => (&self.names, &self.values),
        }
    }

    /// `find(name)`: a linear scan, with no normalisation.
    fn find(&self, name: Atom) -> Option<&Value> {
        let (names, values) = self.snapshot();
        names.iter().position(|n| *n == name).map(|i| &values[i])
    }

    fn apply(self: &Arc<Self>, ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Value> {
        let name = record_field(ev, arg)?;
        if let Some(v) = self.find(name) {
            return Ok(v.clone());
        }
        let text = ev.tool().graph().interner().text(name);
        let shown = Value::Record(self.clone()).ppr(ev)?;
        Err(EvalError::detailed(format!("Attempted to access nonexistent field '{text}' of record\n{shown}"), None))
    }

    fn select(&self, ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Option<Value>> {
        let name = record_field(ev, arg)?;
        Ok(self.find(name).cloned())
    }

    fn take_except(&self, ev: &mut Ev<'_>, ex: &mut Except) -> EvalResult<Value> {
        if !ex.done() {
            let arc = ex.current().clone();
            if let Value::Str(name) = arc {
                let (names, values) = self.snapshot();
                let (names, mut new_values) = (names.to_vec(), values.to_vec());
                for i in 0..names.len() {
                    if names[i] == name {
                        ex.idx += 1;
                        new_values[i] = values[i].take_except(ev, ex)?;
                    }
                }
                return Ok(Value::Record(Value::record_arc(names, new_values, self.is_normalized())));
            }
            let shown = arc.ppr(ev)?;
            warn(ev, Ec::TLC_WRONG_RECORD_FIELD_NAME, &[&shown])?;
        }
        Ok(ex.value.clone())
    }
}

impl FcnRcdValue {
    /// `intv`.
    pub(crate) fn interval(&self) -> Option<IntervalValue> {
        match self.domain {
            FcnDomain::Interval(iv) => Some(iv),
            FcnDomain::Explicit(_) => None,
        }
    }

    /// `isNormalized()`.
    pub(crate) fn is_normalized(&self) -> bool {
        matches!(self.domain, FcnDomain::Interval(_)) || self.norm.get().is_some()
    }

    /// `snapshot()`: the sorted pair once `normalize()` has run, the constructed one before.
    /// The domain is `None` for an interval domain, which TLC keeps as `intv` and no array.
    pub(crate) fn snapshot(&self) -> (Option<&[Value]>, &[Value]) {
        match (&self.domain, self.norm.get()) {
            (FcnDomain::Interval(_), _) => (None, &self.values),
            (FcnDomain::Explicit(_), Some((domain, values))) => (Some(domain), values),
            (FcnDomain::Explicit(domain), None) => (Some(domain), &self.values),
        }
    }

    /// `normalize()`; a no-op for an interval domain.
    pub(crate) fn normalize(&self, ev: &mut Ev<'_>) -> EvalResult<(Option<&[Value]>, &[Value])> {
        match &self.domain {
            FcnDomain::Interval(_) => Ok((None, &self.values)),
            FcnDomain::Explicit(_) => {
                let (domain, values) = fcn_normalized(ev, self)?;
                Ok((Some(domain), values))
            }
        }
    }

    /// `toTuple()` (`FcnRcdValue.java:620-642`); reads the snapshot, so it does not sort.
    fn to_tuple(&self) -> Option<Arc<TupleValue>> {
        if let FcnDomain::Interval(iv) = self.domain {
            if iv.lo != 1 && iv.size() != 0 {
                return None;
            }
            return Some(Arc::new(TupleValue { elems: self.values.clone() }));
        }
        let (domain, values) = self.snapshot();
        let domain = domain.expect("an explicit domain");
        let len = values.len();
        let mut elems: Vec<Option<Value>> = vec![None; len];
        for i in 0..len {
            let Value::Int(idx) = domain[i] else { return None };
            if idx <= 0 || i64::from(idx) > len as i64 {
                return None;
            }
            let slot = &mut elems[idx as usize - 1];
            if slot.is_some() {
                return None;
            }
            *slot = Some(values[i].clone());
        }
        Some(Arc::new(TupleValue { elems: elems.into_iter().map(|v| v.expect("len distinct indices fill 1..len")).collect() }))
    }

    /// `toRcd()` (`FcnRcdValue.java:644-657`).
    fn to_rcd(&self, ev: &mut Ev<'_>) -> EvalResult<Option<Arc<RecordValue>>> {
        if matches!(self.domain, FcnDomain::Interval(_)) {
            return Ok(None);
        }
        let (domain, values) = self.normalize(ev)?;
        let domain = domain.expect("an explicit domain");
        let mut names = Vec::with_capacity(domain.len());
        for d in domain {
            let Value::Str(a) = d else { return Ok(None) };
            names.push(*a);
        }
        Ok(Some(Value::record_arc(names, values.to_vec(), true)))
    }

    /// `select(arg)` (`FcnRcdValue.java:375-455`).
    fn select(&self, ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Option<Value>> {
        if let FcnDomain::Interval(iv) = self.domain {
            let Value::Int(idx) = arg else {
                let shown = arg.ppr(ev)?;
                return Err(EvalError::detailed(
                    format!("Attempted to apply function with integer domain to the non-integer argument {shown}"),
                    None,
                ));
            };
            if *idx < iv.lo || *idx > iv.hi {
                return Ok(None);
            }
            let offset = i64::from(*idx) - i64::from(iv.lo);
            return Ok(self.values.get(offset as usize).cloned());
        }
        let (domain, values) = self.snapshot();
        let domain = domain.expect("an explicit domain");
        if self.is_normalized() && domain.len() >= LINEAR_SEARCH_THRESHOLD {
            // `Arrays.binarySearch(domain, arg, Value::compareTo)`: the domain element is the
            // receiver, and the hit is confirmed with `equals` (`FcnRcdValue.java:433-455`).
            let (mut lo, mut hi) = (0usize, domain.len());
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                match domain[mid].tlc_compare(ev, arg)? {
                    Ordering::Less => lo = mid + 1,
                    Ordering::Greater => hi = mid,
                    Ordering::Equal => {
                        return Ok(if domain[mid].tlc_equals(ev, arg)? { Some(values[mid].clone()) } else { None });
                    }
                }
            }
            return Ok(None);
        }
        for i in 0..domain.len() {
            if domain[i].tlc_equals(ev, arg)? {
                return Ok(Some(values[i].clone()));
            }
        }
        Ok(None)
    }

    fn take_except(self: &Arc<Self>, ev: &mut Ev<'_>, ex: &mut Except) -> EvalResult<Value> {
        if ex.done() {
            return Ok(ex.value.clone());
        }
        let (domain, values) = self.snapshot();
        let mut new_values = values.to_vec();
        let arg = ex.current().clone();
        if let FcnDomain::Interval(iv) = self.domain {
            if let Value::Int(n) = arg {
                if n >= iv.lo && n <= iv.hi {
                    let offset = (i64::from(n) - i64::from(iv.lo)) as usize;
                    if offset >= new_values.len() {
                        return Ok(Value::FcnRcd(self.clone()));
                    }
                    ex.idx += 1;
                    new_values[offset] = values[offset].take_except(ev, ex)?;
                }
                return Ok(Value::FcnRcd(Value::fcn_rcd_interval_arc(iv, new_values)));
            }
        } else {
            let domain = domain.expect("an explicit domain").to_vec();
            for i in 0..new_values.len() {
                if arg.tlc_equals(ev, &domain[i])? {
                    ex.idx += 1;
                    new_values[i] = new_values[i].take_except(ev, ex)?;
                    return Ok(Value::FcnRcd(Value::fcn_rcd_arc(domain, new_values, self.is_normalized())));
                }
            }
        }
        Ok(Value::FcnRcd(self.clone()))
    }
}

impl FcnLambdaValue {
    /// `params.length()`: one argument per non-tuple formal, one per tuple formal list.
    pub(crate) fn arg_len(&self) -> usize {
        self.params.iter().map(|(formals, is_tuple, _)| if *is_tuple { 1 } else { formals.len() }).sum()
    }

    /// `this.con` with `makeRecursive`'s `fname |-> this` (`FcnLambdaValue.java:91-100`).
    /// TLC conses the function into its own context, a cycle `Arc` would leak, so the
    /// binding is rebuilt here every time the body is evaluated.
    fn base_ctx(self: &Arc<Self>) -> Ctx {
        match self.rec_name {
            Some(name) => self.env.ctx.cons(name, Binding::Value(Value::FcnLambda(self.clone()))),
            None => self.env.ctx.clone(),
        }
    }

    fn eval_body(self: &Arc<Self>, ev: &mut Ev<'_>, c1: &Ctx, control: EvalControl) -> EvalResult<Value> {
        let (s0, s1) = self.env.states();
        ev.eval(self.body, c1, s0, s1, control)
    }

    /// `FcnParams.toString()`.
    pub(crate) fn params_string(&self, ev: &mut Ev<'_>) -> EvalResult<String> {
        let interner = ev.tool().graph().interner().clone();
        let name = |ev: &Ev<'_>, p: ParamId| interner.text(ev.tool().graph().param(p).name);
        let mut out = String::new();
        for (i, (formals, is_tuple, domain)) in self.params.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            if *is_tuple {
                out.push_str("<<");
                for (j, &p) in formals.iter().enumerate() {
                    if j > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&name(ev, p));
                }
                out.push_str(">>");
            } else {
                out.push_str(&name(ev, formals[0]));
            }
            out.push_str(" \\in ");
            out.push_str(&domain.to_tlc_string(ev)?);
        }
        Ok(out)
    }

    /// `FcnParams.size()`: the number of points the function has, which must fit an int.
    pub(crate) fn params_size(&self, ev: &mut Ev<'_>) -> EvalResult<i32> {
        let mut sz: i64 = 1;
        for (formals, is_tuple, domain) in self.params.iter() {
            let sz1 = i64::from(domain.size(ev)?);
            let repeats = if *is_tuple { 1 } else { formals.len() };
            for _ in 0..repeats {
                sz *= sz1;
                if sz < i64::from(i32::MIN) || sz > i64::from(i32::MAX) {
                    let params = self.params_string(ev)?;
                    let text = ev.out().text(Ec::TLC_MODULE_OVERFLOW, &[&format!("the number of elements in:\n{params}")]);
                    return Err(EvalError::runtime(Ec::TLC_MODULE_OVERFLOW, text));
                }
            }
        }
        Ok(sz as i32)
    }

    /// `FcnParams.elements()`.
    fn param_elements(&self, ev: &mut Ev<'_>) -> EvalResult<ParamEnum> {
        if self.arg_len() == 1 {
            return match self.params[0].2.elements(ev, EnumOrder::Native)? {
                Some(e) => Ok(ParamEnum::Single(e)),
                None => Err(EvalError::detailed("The domains of formal parameters must be enumerable.".into(), None)),
            };
        }
        let mut enums = Vec::with_capacity(self.arg_len());
        for (formals, is_tuple, domain) in self.params.iter() {
            let repeats = if *is_tuple { 1 } else { formals.len() };
            for _ in 0..repeats {
                match domain.elements(ev, EnumOrder::Native)? {
                    Some(e) => enums.push(e),
                    None => return Err(EvalError::detailed("The domains of the parameters must be enumerable.".into(), None)),
                }
            }
        }
        let mut current = Vec::with_capacity(enums.len());
        let mut done = false;
        for e in enums.iter_mut() {
            match e.next(ev)? {
                Some(v) => current.push(v),
                None => {
                    done = true;
                    break;
                }
            }
        }
        Ok(ParamEnum::Odometer { enums, current, done })
    }

    /// `toFcnRcd()` (`FcnLambdaValue.java:651-744`): enumerate the domain, evaluate the body
    /// at each point, apply the carried EXCEPTs, normalise, and cache.
    pub(crate) fn to_fcn_rcd(self: &Arc<Self>, ev: &mut Ev<'_>) -> EvalResult<Arc<FcnRcdValue>> {
        if let Some(f) = self.converted.get() {
            return Ok(f.clone());
        }
        let sz = self.params_size(ev)?.max(0) as usize;
        let mut domain = Vec::with_capacity(sz);
        let mut values = Vec::with_capacity(sz);
        let mut e = self.param_elements(ev)?;
        let base = self.base_ctx();
        let single = self.arg_len() == 1;
        while let Some(arg) = e.next(ev)? {
            let mut c1 = base.clone();
            if single {
                let (formals, is_tuple, _) = &self.params[0];
                c1 = if *is_tuple {
                    bind_tuple(c1, formals, &arg)?
                } else {
                    c1.cons(SymbolRef::Param(formals[0]), Binding::Value(arg.clone()))
                };
            } else {
                let Value::Tuple(args) = &arg else { return Err(not_a_tuple_cast(&arg)) };
                let mut argn = 0;
                for (formals, is_tuple, _) in self.params.iter() {
                    if *is_tuple {
                        c1 = bind_tuple(c1, formals, &args.elems[argn])?;
                        argn += 1;
                    } else {
                        for &id in formals.iter() {
                            c1 = c1.cons(SymbolRef::Param(id), Binding::Value(args.elems[argn].clone()));
                            argn += 1;
                        }
                    }
                }
            }
            domain.push(arg);
            values.push(self.eval_body(ev, &c1, self.env.control)?);
        }
        let mut fcn = match self.params.first() {
            Some((_, _, Value::Interval(iv))) if single => Value::fcn_rcd_interval_arc(*iv, values),
            _ => Value::fcn_rcd_arc(domain, values, false),
        };
        if !self.excepts.is_empty() {
            // Fresh cursors, so `toFcnRcd` leaves the carried clauses untouched
            // (`FcnLambdaValue.java:714-733`).
            let mut exs: Vec<Except> = self.excepts.to_vec();
            match Value::FcnRcd(fcn).take_excepts(ev, &mut exs)? {
                Value::FcnRcd(f) => fcn = f,
                other => return Err(EvalError::class_cast(other.java_class(), "FcnRcdValue")),
            }
        }
        fcn.normalize(ev)?;
        let _ = self.converted.set(fcn.clone());
        Ok(fcn)
    }

    /// `toTuple()` (`FcnLambdaValue.java:601-633`).
    fn to_tuple(self: &Arc<Self>, ev: &mut Ev<'_>) -> EvalResult<Option<Arc<TupleValue>>> {
        if self.arg_len() != 1 {
            return Ok(None);
        }
        let elems = match &self.params[0].2 {
            Value::Interval(iv) => {
                let sz = iv.size();
                if iv.lo != 1 && sz != 0 {
                    return Ok(None);
                }
                let mut elems = Vec::with_capacity(sz.max(0) as usize);
                for i in 0..sz {
                    elems.push(self.selected(ev, &Value::Int(i + 1))?);
                }
                elems
            }
            dom => {
                let Some(set) = dom.to_set_enum(ev)? else {
                    return Err(EvalError::detailed(
                        "To convert a function of form [x \\in S |-> f(x)] to a tuple, the set S must be enumerable.".into(),
                        None,
                    ));
                };
                let members = normalized(ev, &set)?.to_vec();
                let mut elems = Vec::with_capacity(members.len());
                for (i, arg) in members.iter().enumerate() {
                    match arg {
                        Value::Int(n) if i64::from(*n) == i as i64 + 1 => {}
                        _ => return Ok(None),
                    }
                    elems.push(self.selected(ev, arg)?);
                }
                elems
            }
        };
        Ok(Some(Arc::new(TupleValue { elems: elems.into_boxed_slice() })))
    }

    /// `select(arg)` where TLC stores the result in a `Value[]` without a null check.
    fn selected(self: &Arc<Self>, ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Value> {
        match self.select(ev, arg)? {
            Some(v) => Ok(v),
            None => Err(EvalError::unimplemented("TLC's null element in a tuple converted from a function".into())),
        }
    }

    /// `getDomain()` (`FcnLambdaValue.java:467-501`).
    fn get_domain(self: &Arc<Self>, ev: &mut Ev<'_>) -> EvalResult<Value> {
        if let Some(f) = self.converted.get() {
            return Value::FcnRcd(f.clone()).get_domain(ev);
        }
        if self.arg_len() == 1 {
            return Ok(self.params[0].2.clone());
        }
        let mut sets = Vec::with_capacity(self.arg_len());
        for (formals, is_tuple, domain) in self.params.iter() {
            let repeats = if *is_tuple { 1 } else { formals.len() };
            for _ in 0..repeats {
                sets.push(domain.clone());
            }
        }
        Ok(Value::SetOfTuples(Arc::new(SetOfTuplesValue { sets: sets.into_boxed_slice(), converted: OnceLock::new() })))
    }

    /// `apply(args, control)` (`FcnLambdaValue.java:152-270`).
    fn apply(self: &Arc<Self>, ev: &mut Ev<'_>, args: &Value, control: EvalControl) -> EvalResult<Value> {
        if let Some(f) = self.converted.get() {
            return Value::FcnRcd(f.clone()).apply(ev, args, control);
        }
        let (matched, mut pending) = self.matching_excepts(ev, args)?;
        let res = match matched {
            Some(v) => v,
            None => match self.bind_args(ev, args, true)? {
                Some(c1) => self.eval_body(ev, &c1, control)?,
                // A tuple argument of the wrong length makes `apply` return null
                // (`FcnLambdaValue.java:201`), which TLC then uses as a value.
                None => return Err(EvalError::unimplemented("TLC's null from applying a function to a tuple of the wrong length".into())),
            },
        };
        if pending.is_empty() {
            return Ok(res);
        }
        pending.reverse();
        res.take_excepts(ev, &mut pending)
    }

    /// `select(arg)` (`FcnLambdaValue.java:272-380`): `apply` with the domain checks
    /// answering `None` instead of failing, and the captured control.
    fn select(self: &Arc<Self>, ev: &mut Ev<'_>, arg: &Value) -> EvalResult<Option<Value>> {
        if let Some(f) = self.converted.get() {
            return Value::FcnRcd(f.clone()).select(ev, arg);
        }
        let (matched, mut pending) = self.matching_excepts(ev, arg)?;
        let res = match matched {
            Some(v) => v,
            None => match self.bind_args(ev, arg, false)? {
                Some(c1) => self.eval_body(ev, &c1, self.env.control)?,
                None => return Ok(None),
            },
        };
        if pending.is_empty() {
            return Ok(Some(res));
        }
        pending.reverse();
        res.take_excepts(ev, &mut pending).map(Some)
    }

    /// The carried EXCEPT clauses whose current path element is `args`, scanned from the last
    /// one: either the replacement value itself, or the clauses to apply to the body's value.
    fn matching_excepts(&self, ev: &mut Ev<'_>, args: &Value) -> EvalResult<(Option<Value>, Vec<Except>)> {
        let mut pending = Vec::new();
        for ex in self.excepts.iter().rev() {
            if ex.current().tlc_equals(ev, args)? {
                if ex.is_last() {
                    return Ok((Some(ex.value.clone()), pending));
                }
                pending.push(ex.at(ex.idx + 1));
            }
        }
        Ok((None, pending))
    }

    /// The formals bound to `args`. `strict` is `apply`, which fails where `select` answers
    /// `None`: a domain miss, and a tuple argument whose length is not the formals'.
    fn bind_args(self: &Arc<Self>, ev: &mut Ev<'_>, args: &Value, strict: bool) -> EvalResult<Option<Ctx>> {
        let mut c1 = self.base_ctx();
        if self.arg_len() == 1 {
            let (formals, is_tuple, domain) = &self.params[0];
            if !domain.member(ev, args)? {
                if !strict {
                    return Ok(None);
                }
                let (f, a) = (Value::FcnLambda(self.clone()).ppr(ev)?, args.ppr(ev)?);
                return Err(EvalError::detailed(
                    format!("In applying the function\n{f},\nthe first argument is:\n{a}\nwhich is not in its domain.\n"),
                    None,
                ));
            }
            if *is_tuple {
                let Some(tuple) = args.to_tuple(ev)? else {
                    return Err(self.no_match(ev, "the first argument", args)?);
                };
                if tuple.elems.len() != formals.len() {
                    return Ok(None);
                }
                for (&id, v) in formals.iter().zip(tuple.elems.iter()) {
                    c1 = c1.cons(SymbolRef::Param(id), Binding::Value(v.clone()));
                }
            } else {
                c1 = c1.cons(SymbolRef::Param(formals[0]), Binding::Value(args.clone()));
            }
            return Ok(Some(c1));
        }
        let Some(tuple) = args.to_tuple(ev)? else {
            return Err(self.no_match(ev, "the argument list", args)?);
        };
        let elems = &tuple.elems;
        let mut argn = 0;
        for (formals, is_tuple, domain) in self.params.iter() {
            // TLC reads past a short argument list (`FcnLambdaValue.java:224, 244`).
            if argn >= elems.len() {
                return Err(EvalError::index_out_of_bounds(argn, elems.len()));
            }
            if *is_tuple {
                if !domain.member(ev, &elems[argn])? {
                    if !strict {
                        return Ok(None);
                    }
                    return Err(self.not_in_domain(ev, argn + 1, &elems[argn])?);
                }
                let inner = elems[argn].to_tuple(ev)?;
                argn += 1;
                let Some(inner) = inner else {
                    return Err(self.no_match(ev, &format!("the argument number {argn}"), &elems[argn - 1])?);
                };
                if inner.elems.len() != formals.len() {
                    if !strict {
                        return Ok(None);
                    }
                    return Err(self.no_match(ev, &format!("the argument number {argn}"), &elems[argn - 1])?);
                }
                for (&id, v) in formals.iter().zip(inner.elems.iter()) {
                    c1 = c1.cons(SymbolRef::Param(id), Binding::Value(v.clone()));
                }
            } else {
                for &id in formals.iter() {
                    if argn >= elems.len() {
                        return Err(EvalError::index_out_of_bounds(argn, elems.len()));
                    }
                    if !domain.member(ev, &elems[argn])? {
                        if !strict {
                            return Ok(None);
                        }
                        let f = Value::FcnLambda(self.clone()).ppr(ev)?;
                        let a = elems[argn].ppr(ev)?;
                        let dom = self.get_domain(ev)?.to_tlc_string(ev)?;
                        let n = argn + 1;
                        return Err(EvalError::detailed(
                            format!("In applying the function\n{f},\nthe argument number {n} is:\n{a}\nwhich is not in the function's domain {dom}.\n"),
                            None,
                        ));
                    }
                    c1 = c1.cons(SymbolRef::Param(id), Binding::Value(elems[argn].clone()));
                    argn += 1;
                }
            }
        }
        Ok(Some(c1))
    }

    fn no_match(self: &Arc<Self>, ev: &mut Ev<'_>, what: &str, arg: &Value) -> EvalResult<EvalError> {
        let (f, a) = (Value::FcnLambda(self.clone()).ppr(ev)?, arg.ppr(ev)?);
        Ok(EvalError::detailed(
            format!("In applying the function\n{f},\n{what} is:\n{a}\nwhich does not match its formal parameter.\n"),
            None,
        ))
    }

    fn not_in_domain(self: &Arc<Self>, ev: &mut Ev<'_>, n: usize, arg: &Value) -> EvalResult<EvalError> {
        let (f, a) = (Value::FcnLambda(self.clone()).ppr(ev)?, arg.ppr(ev)?);
        Ok(EvalError::detailed(
            format!("In applying the function\n{f},\nthe argument number {n} is:\n{a}\nwhich is not in its domain.\n"),
            None,
        ))
    }

    /// `takeExcept(ValueExcept)` (`FcnLambdaValue.java:391-423`): the clause is carried, not
    /// applied, so the body is evaluated only at the points that are asked for.
    fn take_except(self: &Arc<Self>, ev: &mut Ev<'_>, ex: &mut Except) -> EvalResult<Value> {
        if ex.done() {
            return Ok(ex.value.clone());
        }
        if let Some(f) = self.converted.get() {
            return Value::FcnRcd(f.clone()).take_except(ev, ex);
        }
        let mut excepts = self.excepts.to_vec();
        excepts.push(ex.clone());
        Ok(self.with_excepts(excepts))
    }

    /// `takeExcept(ValueExcept[])` (`FcnLambdaValue.java:426-465`): a clause whose path is
    /// already consumed drops every clause before it, the carried ones included.
    fn take_excepts(self: &Arc<Self>, ev: &mut Ev<'_>, exs: &mut [Except]) -> EvalResult<Value> {
        if let Some(f) = self.converted.get() {
            return Value::FcnRcd(f.clone()).take_excepts(ev, exs);
        }
        if exs.is_empty() {
            return Ok(self.with_excepts(self.excepts.to_vec()));
        }
        let excepts = match exs.iter().rposition(Except::done) {
            Some(i) => exs[i + 1..].to_vec(),
            None => self.excepts.iter().chain(exs.iter()).cloned().collect(),
        };
        Ok(self.with_excepts(excepts))
    }

    /// `new FcnLambdaValue(this)` with `fcnRcd` cleared: the cache lacks the new clauses.
    fn with_excepts(self: &Arc<Self>, excepts: Vec<Except>) -> Value {
        Value::FcnLambda(Arc::new(FcnLambdaValue {
            params: self.params.clone(),
            body: self.body,
            env: Closure { ctx: self.env.ctx.clone(), s0: self.env.s0.clone(), s1: self.env.s1.clone(), control: self.env.control },
            rec_name: self.rec_name,
            excepts: excepts.into_boxed_slice(),
            converted: OnceLock::new(),
        }))
    }
}

/// `c1.cons(ids[j], ((TupleValue) arg).elems[j])` for a `<<x, y>>` formal list.
fn bind_tuple(mut c1: Ctx, formals: &[ParamId], arg: &Value) -> EvalResult<Ctx> {
    let Value::Tuple(t) = arg else { return Err(not_a_tuple_cast(arg)) };
    if t.elems.len() < formals.len() {
        // TLC reads past the tuple (`FcnLambdaValue.java:672-676`).
        return Err(EvalError::index_out_of_bounds(t.elems.len(), t.elems.len()));
    }
    for (&id, v) in formals.iter().zip(t.elems.iter()) {
        c1 = c1.cons(SymbolRef::Param(id), Binding::Value(v.clone()));
    }
    Ok(c1)
}

/// `MP.printWarning(ec, params)`, which `-messagesAsErrors` turns into `Assert.fail`.
pub(crate) fn warn(ev: &mut Ev<'_>, ec: Ec, params: &[&str]) -> EvalResult<()> {
    match ev.out().warning(ec, params) {
        Ok(()) => Ok(()),
        Err(_) => Err(EvalError::runtime(ec, ev.out().text(ec, params))),
    }
}

/// `FcnParams.elements()`: the domain's own enumeration for a single argument, else an
/// odometer over the parameter domains with the first component fastest, yielding one tuple
/// per combination (`FcnParams.java:100-175`).
enum ParamEnum {
    Single(ValueEnumeration),
    Odometer { enums: Vec<ValueEnumeration>, current: Vec<Value>, done: bool },
}

impl ParamEnum {
    fn next(&mut self, ev: &mut Ev<'_>) -> EvalResult<Option<Value>> {
        match self {
            ParamEnum::Single(e) => e.next(ev),
            ParamEnum::Odometer { enums, current, done } => {
                if *done {
                    return Ok(None);
                }
                let elems = current.clone();
                for i in 0..enums.len() {
                    if let Some(v) = enums[i].next(ev)? {
                        current[i] = v;
                        return Ok(Some(Value::tuple(elems)));
                    }
                    if i == enums.len() - 1 {
                        *done = true;
                        break;
                    }
                    enums[i].reset();
                    current[i] = enums[i].next(ev)?.expect("a reset enumeration of a non-empty set has an element");
                }
                Ok(Some(Value::tuple(elems)))
            }
        }
    }
}
