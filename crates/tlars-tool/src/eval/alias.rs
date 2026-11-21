//! `Tool.evalAlias` and `RecordValue.PrintTLCState`: the cfg ALIAS as the trace printer
//! sees it.
//!
//! The ALIAS is resolved and evaluated only while a counterexample is printed, so an ALIAS
//! that fails leaves the run's verdict alone and shows up as an `_ALIASEvalError` field on
//! each state of the trace.

use crate::error::{EvalError, EvalErrorKind, EvalResult};
use crate::eval::{EvalControl, Ev};
use crate::fp64::Fp;
use crate::state::{PartialState, State, StateRef};
use crate::value::Value;
use tlars_sany::Atom;

/// `_format`, the field that gives the format string for the others.
const FORMAT: &str = "_format";
/// `RecordValue.PrintTLCState._ALIASEvalError`.
const EVAL_ERROR: &str = "_ALIASEvalError";

/// One trace state as `evalAlias` left it: either the state itself (no ALIAS, or an ALIAS
/// that is not a record) or the alias record with the state its variable-named fields bind.
pub struct Aliased {
    /// `AliasTLCStateInfo.getOriginalState()`.
    pub(crate) original: State,
    pub(crate) record: Option<AliasRecord>,
}

/// `RecordValue.PrintTLCState`: the normalised record, plus `RecordValue.toState()`'s state,
/// which holds only the fields named like a variable.
pub(crate) struct AliasRecord {
    names: Box<[Atom]>,
    values: Box<[Value]>,
    state: PartialState,
}

impl Aliased {
    fn plain(original: State) -> Aliased {
        Aliased { original, record: None }
    }
    /// Whether the state that gets printed is the original one, which decides what
    /// `-difftrace` diffs against (`StatePrinter.getPreviousState`).
    fn is_original(&self) -> bool {
        self.record.is_none()
    }
    /// `lookup(var)` of the state being printed: the derived state first, then the record's
    /// own fields, so a field that is not a variable is still found.
    fn lookup(&self, name: Atom, vars: &crate::state::VarTable) -> Option<Value> {
        match &self.record {
            None => vars.slot_of(name).map(|slot| self.original.get(slot).clone()),
            Some(rcd) => {
                if let Some(slot) = vars.slot_of(name)
                    && let Some(v) = rcd.state.lookup(slot)
                {
                    return Some(v.clone());
                }
                let i = rcd.names.iter().position(|n| *n == name)?;
                Some(rcd.values[i].clone())
            }
        }
    }
}

impl Ev<'_> {
    /// `Tool.evalAlias(TLCStateInfo current, TLCState successor, Context ctxt)`
    /// (`Tool.java:1734-1766`). An `EvalException` or a `TLCRuntimeException` — the
    /// `getAliasSpec` failures among them — becomes the state's record plus an
    /// `_ALIASEvalError` field holding the exception's message; nothing else is caught.
    ///
    /// `trace` is the behaviour up to and including `current`, which the `Supplier` overload
    /// binds to `TLCExt!Trace`. `None` is the `Context.Empty` overload the initial-state
    /// printer and `Worker.doPostCondition` use.
    pub fn eval_alias(&mut self, current: &State, successor: StateRef<'_>, trace: Option<&[State]>) -> EvalResult<Aliased> {
        let Some(alias) = &self.tool.alias else { return Ok(Aliased::plain(current.clone())) };
        let body = alias.body.clone();
        let trace_symbol = alias.trace_symbol;
        let message = match body {
            Err(message) => Some(message),
            Ok(body) => {
                let ctx = match (trace, trace_symbol) {
                    (Some(states), Some(sym)) => {
                        let records: Vec<Value> = states.iter().map(|s| self.state_record(s)).collect();
                        crate::ctx::Ctx::EMPTY.cons(sym, crate::ctx::Binding::Value(Value::tuple(records)))
                    }
                    _ => crate::ctx::Ctx::EMPTY,
                };
                match self.eval(body, &ctx, StateRef::Full(current), successor, EvalControl::CLEAR) {
                    Ok(v) => match self.alias_record(&v)? {
                        Some(record) => return Ok(Aliased { original: current.clone(), record: Some(record) }),
                        // `toState()` returned null: not a record, so TLC prints the state.
                        None => None,
                    },
                    Err(e) if matches!(e.kind, EvalErrorKind::Eval | EvalErrorKind::Runtime | EvalErrorKind::Fingerprint { .. }) => {
                        Some(e.message)
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        match message {
            None => Ok(Aliased::plain(current.clone())),
            // `new RecordValue((RecordValue) current.toRecordValue(), _ALIASEvalError,
            // new StringValue(e.getMessage())).toState()`.
            Some(message) => {
                let interner = self.tool.graph.interner().clone();
                let mut names: Vec<Atom> = Vec::with_capacity(current.len() + 1);
                let mut values: Vec<Value> = Vec::with_capacity(current.len() + 1);
                for i in 0..current.len() {
                    names.push(self.tool.vars.names[i]);
                    values.push(current.get(self.tool.vars.slot(i)).clone());
                }
                names.push(interner.intern(EVAL_ERROR));
                values.push(Value::Str(interner.intern(&message)));
                let rcd = Value::Record(Value::record_arc(names, values, false));
                let record = self.alias_record(&rcd)?.expect("a record converts to a state");
                Ok(Aliased { original: current.clone(), record: Some(record) })
            }
        }
    }

    /// `new RecordValue(state)`: a record of every variable, in `vars[]` order.
    fn state_record(&mut self, s: &State) -> Value {
        let mut names = Vec::with_capacity(s.len());
        let mut values = Vec::with_capacity(s.len());
        for i in 0..s.len() {
            names.push(self.tool.vars.names[i]);
            values.push(s.get(self.tool.vars.slot(i)).clone());
        }
        Value::Record(Value::record_arc(names, values, false))
    }

    /// `Value.toState()` (`RecordValue.java:858-889`, `FcnRcdValue.java:659-666`): a record,
    /// or a function record whose domain is all strings, binds the variables it names and
    /// keeps every field for printing. Anything else is `null`.
    fn alias_record(&mut self, v: &Value) -> EvalResult<Option<AliasRecord>> {
        let rcd = match v {
            Value::Record(r) => r.clone(),
            // `FcnRcdValue.toRcd()`: only an explicit all-string domain converts.
            Value::FcnRcd(_) => match v.to_rcd(self)? {
                Some(r) => r,
                None => return Ok(None),
            },
            // Only `RecordValue` and `FcnRcdValue` override `toState()`, so a tuple or a
            // lazy function is `null` and TLC prints the state itself.
            _ => return Ok(None),
        };
        // `this.snapshot()`, not `normalize()`: the fields keep the order they were written
        // in, and only a record that came from `FcnRcdValue.toRcd()` is sorted.
        let (names, values) = rcd.snapshot();
        let (names, values) = (names.to_vec(), values.to_vec());
        let mut state = PartialState::empty(self.tool.var_count());
        for (name, value) in names.iter().zip(&values) {
            if let Some(slot) = self.tool.vars.slot_of(*name) {
                state.bind(slot, value.clone());
            }
        }
        Ok(Some(AliasRecord { names: names.into(), values: values.into(), state }))
    }

    /// `PrintTLCState.toString()` / `toString(lastState)` (`RecordValue.java:897-935`): every
    /// field but `_format` through the format string, dropping the fields `-difftrace` finds
    /// unchanged. An `_ALIASEvalError` field is always shown.
    pub fn print_aliased(&mut self, a: &Aliased, prev: Option<&Aliased>) -> EvalResult<String> {
        let Some(rcd) = &a.record else {
            return self.print_state(&a.original, prev.map(|p| &p.original));
        };
        let interner = self.tool.graph.interner().clone();
        let format_name = interner.intern(FORMAT);
        let error_name = interner.intern(EVAL_ERROR);
        let format_at = rcd.names.iter().position(|n| *n == format_name);
        let format = match format_at {
            // `(StringValue) rcd.values[idx]`, which casts without checking.
            Some(i) => match &rcd.values[i] {
                Value::Str(s) => interner.text(*s).to_string(),
                other => return Err(EvalError::class_cast(other.java_class(), "StringValue")),
            },
            None if rcd.names.len() == 1 => "%s = %s\n".to_string(),
            None => "/\\ %s = %s\n".to_string(),
        };
        let mut out = String::new();
        for i in 0..rcd.names.len() {
            if Some(i) == format_at {
                continue;
            }
            let (name, value) = (rcd.names[i], rcd.values[i].clone());
            if let Some(prev) = prev
                && name != error_name
            {
                let old = prev.lookup(name, &self.tool.vars);
                let unchanged = match old {
                    None => false,
                    Some(old) => matches!(old.tlc_equals(self, &value), Ok(true)),
                };
                if unchanged {
                    continue;
                }
            }
            let key = interner.text(name).to_string();
            let shown = value.ppr(self)?;
            out.push_str(&java_format(&format, &key, &shown)?);
        }
        Ok(out)
    }

    /// `tool.evalAlias(curState, curState).toString()`, which `DoInitFunctor` prints for an
    /// initial state that violates an invariant or an implied init (`ModelChecker.java:1218`
    /// and `:1230`). It is the `Context.Empty` overload, so `TLCExt!Trace` is not bound.
    pub fn print_initial_violation(&mut self, s: &State) -> EvalResult<String> {
        let aliased = self.eval_alias(s, StateRef::Full(s), None)?;
        self.print_aliased(&aliased, None)
    }

    /// `currentStateInfo.state.allAssigned() ? currentStateInfo.fingerPrint() : -1`: the
    /// alias state holds only the fields named like a variable, so an alias that renames
    /// them has no fingerprint to print.
    pub fn aliased_fingerprint(&mut self, a: &Aliased) -> EvalResult<Option<Fp>> {
        match &a.record {
            None => self.fingerprint(&a.original).map(Some),
            Some(rcd) => match rcd.state.freeze() {
                None => Ok(None),
                Some(state) => self.fingerprint(&state).map(Some),
            },
        }
    }
}

/// `String.format(format, key, value)` for the two `%s` of a `PrintTLCState` field. Any
/// other conversion is left unimplemented rather than guessed.
fn java_format(format: &str, key: &str, value: &str) -> EvalResult<String> {
    let mut out = String::new();
    let mut args = [key, value].into_iter();
    let mut rest = format.chars().peekable();
    while let Some(c) = rest.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match rest.next() {
            Some('%') => out.push('%'),
            Some('n') => out.push('\n'),
            Some('s') => match args.next() {
                Some(arg) => out.push_str(arg),
                None => {
                    return Err(EvalError::unimplemented(
                        "a _format field with more than two conversions (java.util.MissingFormatArgumentException) (M3)".into(),
                    ));
                }
            },
            _ => {
                return Err(EvalError::unimplemented(
                    "a _format field with a conversion other than %s (M3)".into(),
                ));
            }
        }
    }
    Ok(out)
}
