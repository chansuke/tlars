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
    /// `AliasTLCStateInfo.getOriginalState()`. The last state of a counterexample may be a
    /// successor the next-state relation left incomplete, which TLC holds as a state with
    /// nulls in it, so this is the partial form.
    pub(crate) original: PartialState,
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
    fn plain(original: PartialState) -> Aliased {
        Aliased { original, record: None }
    }
    /// `new TLCStateInfo(state)`: a trace step no ALIAS ran on.
    pub fn of_state(state: &State) -> Aliased {
        Aliased::plain(PartialState::from_state(state))
    }
    /// `lookup(var)` of the state being printed: the derived state first, then the record's
    /// own fields, so a field that is not a variable is still found.
    fn lookup(&self, name: Atom, vars: &crate::state::VarTable) -> Option<Value> {
        match &self.record {
            None => vars.slot_of(name).and_then(|slot| self.original.lookup(slot)).cloned(),
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
    pub fn eval_alias(&mut self, current: &PartialState, successor: StateRef<'_>, trace: Option<&[PartialState]>) -> EvalResult<Aliased> {
        let Some(alias) = &self.tool.alias else { return Ok(Aliased::plain(current.copy())) };
        let body = alias.body.clone();
        let trace_symbol = alias.trace_symbol;
        // `IdThread.setCurrentState(current.state)` (`Tool.java:1687` and `:1736`): a
        // RandomElement draw inside the ALIAS re-seeds with this state's fingerprint, just
        // as one inside the next-state relation re-seeds with the parent's. A state the
        // next-state relation left incomplete has no fingerprint, and TLC's own re-seeding
        // fails at the first draw, so none is recorded for it.
        if let Some(state) = current.freeze() {
            let fp = self.fingerprint(&state)?;
            self.begin_expansion(&state, fp);
        }
        let message = match body {
            Err(message) => Some(message),
            Ok(body) => {
                let ctx = match (trace, trace_symbol) {
                    (Some(states), Some(sym)) => {
                        let mut records: Vec<Value> = Vec::with_capacity(states.len());
                        for s in states {
                            records.push(self.state_record(s)?);
                        }
                        crate::ctx::Ctx::EMPTY.cons(sym, crate::ctx::Binding::Value(Value::tuple(records)))
                    }
                    _ => crate::ctx::Ctx::EMPTY,
                };
                // `toState()` is inside the `try`, so a normalisation it runs can fail and
                // be caught like any other failure of the ALIAS.
                let aliased = self.eval(body, &ctx, StateRef::Partial(current), successor, EvalControl::CLEAR).and_then(|v| self.alias_record(&v));
                match aliased {
                    Ok(Some(record)) => return Ok(Aliased { original: current.copy(), record: Some(record) }),
                    // `toState()` returned null: not a record, so TLC prints the state.
                    Ok(None) => None,
                    Err(e) if caught_by_eval_alias(&e) => Some(e.into_data().message),
                    Err(e) => return Err(e),
                }
            }
        };
        match message {
            None => Ok(Aliased::plain(current.copy())),
            // `new RecordValue((RecordValue) current.toRecordValue(), _ALIASEvalError,
            // new StringValue(e.getMessage())).toState()`.
            Some(message) => {
                let interner = self.tool.graph.interner().clone();
                let var_count = self.tool.var_count();
                let mut names: Vec<Atom> = Vec::with_capacity(var_count + 1);
                let mut values: Vec<Value> = Vec::with_capacity(var_count + 1);
                for i in 0..var_count {
                    let Some(value) = current.lookup(self.tool.vars.slot(i)) else {
                        // `toRecordValue()` of a state with an unassigned variable puts a
                        // null in the record, which every later read of it treats as its
                        // own kind of value.
                        return Err(EvalError::unimplemented(
                            "an ALIAS that fails on a successor the next-state relation left incomplete (M3)".into(),
                        ));
                    };
                    names.push(self.tool.vars.names[i]);
                    values.push(value.clone());
                }
                names.push(interner.intern(EVAL_ERROR));
                values.push(Value::Str(interner.intern(&message)));
                let rcd = Value::Record(Value::record_arc(names, values, false));
                let record = self.alias_record(&rcd)?.expect("a record converts to a state");
                Ok(Aliased { original: current.copy(), record: Some(record) })
            }
        }
    }

    /// `TLCStateInfo.toRecordValue()` (`TLCStateInfo.java:123-130`): for an aliased state the
    /// alias record as written, and otherwise `new RecordValue(state)`.
    pub fn aliased_record(&mut self, a: &Aliased) -> EvalResult<Value> {
        match &a.record {
            None => self.state_record(&a.original),
            Some(rcd) => Ok(Value::Record(Value::record_arc(rcd.names.to_vec(), rcd.values.to_vec(), false))),
        }
    }

    /// `s.allAssigned()`: `TLCState.Empty` is a state of nulls, so it is fully assigned only
    /// when the spec declares no variable.
    pub(crate) fn all_assigned(&self, s: StateRef<'_>) -> bool {
        match s {
            StateRef::Full(_) => true,
            StateRef::Partial(p) => p.all_assigned(),
            StateRef::Empty | StateRef::Null | StateRef::Fun(_) => self.tool.var_count() == 0,
        }
    }

    /// `s.getUnassigned()`, named as `TLCExt!Trace` prints them.
    pub(crate) fn unassigned_of(&self, s: StateRef<'_>) -> Vec<String> {
        let interner = self.tool.graph.interner();
        match s {
            StateRef::Full(_) => Vec::new(),
            StateRef::Partial(p) => p.unassigned(&self.tool.vars, interner),
            StateRef::Empty | StateRef::Null | StateRef::Fun(_) => {
                let mut names: Vec<String> = self.tool.vars.names.iter().map(|&n| interner.text(n).to_string()).collect();
                names.sort();
                names
            }
        }
    }

    /// `new RecordValue(state)` for one element of `TLCExt!Trace`.
    pub(crate) fn trace_record(&mut self, s: StateRef<'_>) -> EvalResult<Value> {
        let partial = match s {
            StateRef::Full(state) => PartialState::from_state(state),
            StateRef::Partial(p) => p.copy(),
            // `TLCState.Empty` with no variable declared: the empty record.
            StateRef::Empty | StateRef::Null | StateRef::Fun(_) => PartialState::empty(self.tool.var_count()),
        };
        self.state_record(&partial)
    }

    /// `new RecordValue(state)`: a record of every variable, in `vars[]` order.
    fn state_record(&mut self, s: &PartialState) -> EvalResult<Value> {
        let var_count = self.tool.var_count();
        let mut names = Vec::with_capacity(var_count);
        let mut values = Vec::with_capacity(var_count);
        for i in 0..var_count {
            let Some(value) = s.lookup(self.tool.vars.slot(i)) else {
                // A record field holding a Java null, which every read of it then treats as
                // its own kind of value.
                return Err(EvalError::unimplemented(
                    "TLCExt!Trace over a successor the next-state relation left incomplete (M3)".into(),
                ));
            };
            names.push(self.tool.vars.names[i]);
            values.push(value.clone());
        }
        Ok(Value::Record(Value::record_arc(names, values, false)))
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
            // `toString()` / `toString(lastState)` of the state itself. An incomplete
            // successor prints its unassigned variables as `null` and is never diffed,
            // because it is the last state of its trace.
            return match (a.original.freeze(), prev.and_then(|p| p.original.freeze())) {
                (Some(state), prev) => self.print_state(&state, prev.as_ref()),
                (None, _) => Ok(self.print_partial_state(&a.original)),
            };
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
        let aliased = self.eval_alias(&PartialState::from_state(s), StateRef::Full(s), None)?;
        self.print_aliased(&aliased, None)
    }

    /// `currentStateInfo.state.allAssigned() ? currentStateInfo.fingerPrint() : -1`: the
    /// alias state holds only the fields named like a variable, so an alias that renames
    /// them has no fingerprint to print.
    pub fn aliased_fingerprint(&mut self, a: &Aliased) -> EvalResult<Option<Fp>> {
        match &a.record {
            None => match a.original.freeze() {
                None => Ok(None),
                Some(state) => self.fingerprint(&state).map(Some),
            },
            Some(rcd) => match rcd.state.freeze() {
                None => Ok(None),
                Some(state) => self.fingerprint(&state).map(Some),
            },
        }
    }
}

/// `catch (EvalException | TLCRuntimeException e)` of `evalAlias`: anything else, an
/// unimplemented construct among them, leaves the trace printer.
fn caught_by_eval_alias(e: &EvalError) -> bool {
    matches!(e.kind, EvalErrorKind::Eval | EvalErrorKind::Runtime | EvalErrorKind::Fingerprint { .. })
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
