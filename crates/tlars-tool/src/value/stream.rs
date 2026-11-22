//! `tlc2.value.ValueOutputStream` / `ValueInputStream`, with the `write` and
//! `createFromExternal` of each value class: the bytes a trace file holds
//! (`tlc2.module._TLCTrace`).
//!
//! Layout, as `ValueConstants` numbers the kinds: a kind byte, then the payload. Bool a byte,
//! Int four bytes, ModelValue its creation index as a short, Interval two ints, String the
//! `UniqueString` (token, variable slot, UTF-16 length, the low byte of each unit). Tuple a
//! `writeNat` length then its elements, SetEnum and Record `±length` (negative when not
//! normalised) then elements or name/value pairs, FcnRcd a length, an info byte (0 interval
//! domain, 1 normalised, 2 not) and then the pairs. Lazy sets and function lambdas convert
//! first, so no file holds one. Everything is big-endian.
//!
//! The one place where reproducing the bytes needs more than the value: TLC writes a
//! `DUMMYVALUE` back-reference for any *object* it has already written, keyed by identity.
//! See [`HandleTable`].

use super::{FcnDomain, Value, ValueKind};
use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use std::collections::BTreeMap;
use std::sync::Arc;
use tlars_sany::Atom;

/// `ValueOutputStream(out, false)` fed one value: the kind bytes, no gzip.
pub(crate) fn write(ev: &mut Ev<'_>, v: &Value) -> EvalResult<Vec<u8>> {
    let mut out = Out { bytes: Vec::new(), handles: HandleTable::default() };
    out.value(ev, v)?;
    Ok(out.bytes)
}

/// `ValueInputStream.readExternal()`, which is what `read(Map)` has done since the intern
/// table parameter was dropped: the string tokens and variable slots in the file belong to
/// the run that wrote it, so they are skipped and the text is interned here.
pub(crate) fn read_external(ev: &mut Ev<'_>, bytes: &[u8]) -> EvalResult<Value> {
    In { bytes, pos: 0, handles: Vec::new() }.value(ev)
}

/// `Value.write`'s `WrongInvocationException` for a class with no `write`: `UserValue` (`Nat`,
/// `Int`, `STRING`) and `UndefValue`. `ioSerialize` fingerprints first, and those classes have
/// no `fingerPrint` either, so a value that reaches this has already failed there.
fn cannot_pickle(ev: &mut Ev<'_>, v: &Value) -> EvalError {
    match v.ppr(ev) {
        Ok(shown) => EvalError::detailed(format!("ValueOutputStream: Can not pickle the value\n{shown}"), None),
        Err(e) => e,
    }
}

/// `ValueInputStream`'s `WrongInvocationException` for a kind byte no value class claims.
fn cannot_unpickle(kind: u8) -> EvalError {
    EvalError::detailed(format!("ValueInputStream: Can not unpickle a value of kind {}", kind as i8), None)
}

/// A file that is not one this stream wrote. TLC answers these with an `EOFException` or a
/// `NullPointerException` whose message is null, which `MethodValue.eval` replaces with the
/// Java stack trace, so there is nothing here to reproduce.
fn malformed() -> EvalError {
    EvalError::unimplemented("reading a trace file that is cut short or otherwise malformed".into())
}

/// `ValueOutputStream.HandleTable`, which keys on `System.identityHashCode`, so what it
/// de-duplicates is objects, not equal values.
///
/// Two of TLC's three identities have one here. A container is an `Arc`, and its address is
/// as good as the Java object's. A record field name is a `UniqueString`, which the intern
/// table has already made unique, so its token *is* its identity.
///
/// A `StringValue` has none: `Value::Str` carries an intern token, and TLC makes one
/// `StringValue` per string literal node, per `DOMAIN`, per `ToString`. Two of them with the
/// same text are different objects it writes out twice, and a shared one it writes once. This
/// port cannot tell the two apart, so it takes a fresh slot for every occurrence and writes
/// the string each time. The file stays self-describing, TLC reads it, and a value whose
/// `StringValue`s are all distinct objects - which is what a counterexample record is, since
/// `CounterExample` builds a fresh one per edge - comes out byte for byte TLC's.
#[derive(Default)]
struct HandleTable {
    seen: BTreeMap<Handle, usize>,
    len: usize,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Handle {
    /// The address of the `Arc` payload, which lives as long as the value being written.
    Object(usize),
    Name(Atom),
}

impl HandleTable {
    /// `put(obj)`: the slot it already has, or `None` after giving it the next one.
    fn put(&mut self, handle: Handle) -> Option<usize> {
        if let Some(slot) = self.seen.get(&handle) {
            return Some(*slot);
        }
        self.seen.insert(handle, self.len);
        self.len += 1;
        None
    }

    /// A slot for an object with no identity here, which is never found again.
    fn fresh(&mut self) {
        self.len += 1;
    }
}

fn address<T>(arc: &Arc<T>) -> Handle {
    Handle::Object(Arc::as_ptr(arc) as usize)
}

struct Out {
    bytes: Vec<u8>,
    handles: HandleTable,
}

impl Out {
    fn byte(&mut self, b: u8) {
        self.bytes.push(b);
    }

    fn kind(&mut self, kind: ValueKind) {
        self.byte(kind as u8);
    }

    fn short(&mut self, x: i16) {
        self.bytes.extend_from_slice(&x.to_be_bytes());
    }

    fn int(&mut self, x: i32) {
        self.bytes.extend_from_slice(&x.to_be_bytes());
    }

    /// `writeNat`: a short while it fits in 15 bits, else the negated int.
    fn nat(&mut self, x: i32) {
        if x > 0x7fff {
            self.int(-x);
        } else {
            self.short(x as i16);
        }
    }

    /// `vos.writeByte(DUMMYVALUE); vos.writeNat(index)`.
    fn dummy(&mut self, slot: usize) {
        self.kind(ValueKind::Dummy);
        self.nat(slot as i32);
    }

    /// `UniqueString.write`: the intern token, `getVarLoc()`, the UTF-16 length and then the
    /// low byte of each UTF-16 unit, which is all `String.getBytes` copies.
    fn unique_string(&mut self, ev: &Ev<'_>, a: Atom) {
        let text = ev.tool().graph().interner().text(a);
        self.int(a.tok());
        self.int(ev.tool().vars().slot_of(a).map_or(-1, |slot| slot.index() as i32));
        self.int(text.encode_utf16().count() as i32);
        for unit in text.encode_utf16() {
            self.byte(unit as u8);
        }
    }

    /// `STRINGVALUE` with the string, or the back-reference to a field name already written.
    fn name(&mut self, ev: &Ev<'_>, a: Atom) {
        match self.handles.put(Handle::Name(a)) {
            Some(slot) => self.dummy(slot),
            None => {
                self.kind(ValueKind::String);
                self.unique_string(ev, a);
            }
        }
    }

    /// `isNorm ? len : -len`, which `SetEnumValue` and `RecordValue` write as a plain int.
    fn signed_len(&mut self, len: usize, is_norm: bool) {
        self.int(if is_norm { len as i32 } else { -(len as i32) });
    }

    fn value(&mut self, ev: &mut Ev<'_>, v: &Value) -> EvalResult<()> {
        match v {
            Value::Bool(b) => {
                self.kind(ValueKind::Bool);
                self.byte(u8::from(*b));
            }
            Value::Int(n) => {
                self.kind(ValueKind::Int);
                self.int(*n);
            }
            Value::Str(a) => {
                self.handles.fresh();
                self.kind(ValueKind::String);
                self.unique_string(ev, *a);
            }
            Value::Model(mv) => {
                self.kind(ValueKind::Model);
                self.short(mv.index as i16);
            }
            Value::Interval(iv) => {
                self.kind(ValueKind::Interval);
                self.int(iv.lo);
                self.int(iv.hi);
            }
            Value::Tuple(t) => {
                if let Some(slot) = self.handles.put(address(t)) {
                    self.dummy(slot);
                    return Ok(());
                }
                self.kind(ValueKind::Tuple);
                self.nat(t.elems.len() as i32);
                for e in &t.elems {
                    self.value(ev, e)?;
                }
            }
            Value::Record(r) => {
                if let Some(slot) = self.handles.put(address(r)) {
                    self.dummy(slot);
                    return Ok(());
                }
                self.kind(ValueKind::Record);
                // `snapshot()`: the sorted pair once normalised, the constructed one before.
                let (names, values) = r.snapshot();
                self.signed_len(names.len(), r.is_normalized());
                for (name, value) in names.iter().zip(values) {
                    self.name(ev, *name);
                    self.value(ev, value)?;
                }
            }
            Value::SetEnum(s) => {
                if let Some(slot) = self.handles.put(address(s)) {
                    self.dummy(slot);
                    return Ok(());
                }
                self.kind(ValueKind::SetEnum);
                let elems = s.snapshot();
                self.signed_len(elems.len(), s.norm.get().is_some());
                for e in elems {
                    self.value(ev, e)?;
                }
            }
            Value::FcnRcd(f) => {
                if let Some(slot) = self.handles.put(address(f)) {
                    self.dummy(slot);
                    return Ok(());
                }
                self.kind(ValueKind::FcnRcd);
                let (domain, values) = f.snapshot();
                self.nat(values.len() as i32);
                // `if (intv != null)`, asked of the value itself, not of the snapshot.
                match (&f.domain, domain) {
                    (FcnDomain::Interval(iv), _) => {
                        self.byte(0);
                        self.int(iv.lo);
                        self.int(iv.hi);
                        for value in values {
                            self.value(ev, value)?;
                        }
                    }
                    (FcnDomain::Explicit(_), Some(domain)) => {
                        self.byte(if f.is_normalized() { 1 } else { 2 });
                        for (d, value) in domain.iter().zip(values) {
                            self.value(ev, d)?;
                            self.value(ev, value)?;
                        }
                    }
                    (FcnDomain::Explicit(_), None) => unreachable!("an explicit domain is in the snapshot"),
                }
            }
            // `convertAndCache().write(vos)`: the cached SetEnum carries the handle, so a
            // lazy set written twice is a back-reference to it.
            Value::Subset(_)
            | Value::KSubset(_)
            | Value::SetOfFcns(_)
            | Value::SetOfRcds(_)
            | Value::SetOfTuples(_)
            | Value::Union(_)
            | Value::SetCup(_)
            | Value::SetCap(_)
            | Value::SetDiff(_)
            | Value::SetPred(_) => {
                let set = v.convert_and_cache(ev)?;
                self.value(ev, &Value::SetEnum(set))?;
            }
            // `toFcnRcd().write(vos)`, which does not cache, so two occurrences of one lambda
            // are two function records.
            Value::FcnLambda(l) => {
                let fcn = l.to_fcn_rcd(ev)?;
                self.value(ev, &Value::FcnRcd(fcn))?;
            }
            Value::User(_) | Value::Undef => return Err(cannot_pickle(ev, v)),
        }
        Ok(())
    }
}

struct In<'a> {
    bytes: &'a [u8],
    pos: usize,
    handles: Vec<Slot>,
}

/// `ValueInputStream.HandleTable`'s `Object[]`: a value or a field name, by the slot the
/// writer wrote it at.
enum Slot {
    /// `getIndex()` has handed out the slot and `assign` has not filled it yet, which no
    /// stream this port wrote refers to.
    Pending,
    Value(Value),
    Name(Atom),
}

impl In<'_> {
    fn byte(&mut self) -> EvalResult<u8> {
        let b = *self.bytes.get(self.pos).ok_or_else(malformed)?;
        self.pos += 1;
        Ok(b)
    }

    fn short(&mut self) -> EvalResult<i16> {
        Ok(i16::from_be_bytes([self.byte()?, self.byte()?]))
    }

    fn int(&mut self) -> EvalResult<i32> {
        Ok(i32::from_be_bytes([self.byte()?, self.byte()?, self.byte()?, self.byte()?]))
    }

    /// `readNat`: one short when it is not negative, else that short as the high half of an
    /// int whose negation is the value.
    fn nat(&mut self) -> EvalResult<i32> {
        let first = self.short()?;
        if first >= 0 {
            return Ok(i32::from(first));
        }
        let low = self.short()?;
        Ok(-((i32::from(first) << 16) | i32::from(low as u16)))
    }

    /// `getIndex()`: the next slot, to be filled once the value is built.
    fn index(&mut self) -> usize {
        self.handles.push(Slot::Pending);
        self.handles.len() - 1
    }

    fn assign(&mut self, slot: usize, entry: Slot) {
        self.handles[slot] = entry;
    }

    /// `(IValue) handles.getValue(idx)` for a `DUMMYVALUE`.
    fn value_at(&self, slot: i32) -> EvalResult<Value> {
        match usize::try_from(slot).ok().and_then(|s| self.handles.get(s)) {
            Some(Slot::Value(v)) => Ok(v.clone()),
            _ => Err(malformed()),
        }
    }

    /// `(UniqueString) handles.getValue(idx)` for a field name.
    fn name_at(&self, slot: i32) -> EvalResult<Atom> {
        match usize::try_from(slot).ok().and_then(|s| self.handles.get(s)) {
            Some(Slot::Name(a)) => Ok(*a),
            _ => Err(malformed()),
        }
    }

    /// `UniqueString.readExternal`: token and variable slot skipped, the text interned here.
    fn unique_string(&mut self, ev: &mut Ev<'_>) -> EvalResult<Atom> {
        let (_tok, _loc) = (self.int()?, self.int()?);
        let len = self.int()?;
        let len = usize::try_from(len).map_err(|_| malformed())?;
        // `readString(n)` reads `(char) buff[i]` of a `byte[]`, which sign-extends and then
        // narrows, so byte 0x80 becomes U+FF80 and not U+0080.
        let mut text = String::new();
        for _ in 0..len {
            let b = self.byte()?;
            let unit = if b < 0x80 { u32::from(b) } else { 0xff00 | u32::from(b) };
            text.push(char::from_u32(unit).expect("neither value is a surrogate"));
        }
        Ok(ev.tool().graph().interner().intern(&text))
    }

    /// A field name: `DUMMYVALUE` and a slot, or the string itself at a slot of its own.
    fn name(&mut self, ev: &mut Ev<'_>) -> EvalResult<Atom> {
        if self.byte()? == ValueKind::Dummy as u8 {
            let slot = self.nat()?;
            return self.name_at(slot);
        }
        let slot = self.index();
        let a = self.unique_string(ev)?;
        self.assign(slot, Slot::Name(a));
        Ok(a)
    }

    fn value(&mut self, ev: &mut Ev<'_>) -> EvalResult<Value> {
        let kind = self.byte()?;
        if kind == ValueKind::Bool as u8 {
            return Ok(Value::Bool(self.byte()? != 0));
        }
        if kind == ValueKind::Int as u8 {
            return Ok(Value::Int(self.int()?));
        }
        if kind == ValueKind::String as u8 {
            let a = self.unique_string(ev)?;
            let slot = self.index();
            self.assign(slot, Slot::Value(Value::Str(a)));
            return Ok(Value::Str(a));
        }
        if kind == ValueKind::Model as u8 {
            // `ModelValue.mvs[readShort()]`, an array read that reports its own bounds.
            let index = self.short()?;
            let count = ev.tool().model_value_count();
            return match ev.tool().model_value_at(index) {
                Some(mv) => Ok(Value::Model(mv)),
                None => Err(EvalError::detailed(tlars_java::index_out_of_bounds(i64::from(index), count), None)),
            };
        }
        if kind == ValueKind::Interval as u8 {
            let (lo, hi) = (self.int()?, self.int()?);
            return Ok(Value::Interval(super::IntervalValue { lo, hi }));
        }
        if kind == ValueKind::Record as u8 {
            // `RecordValue.createFromExternal` is a static of the class, so the class
            // initialises - and its field-name statics take their intern tokens - before the
            // names in the file are read.
            ev.tool().graph().interner().ensure(tlars_sany::JavaClass::RecordValue);
            let slot = self.index();
            let len = length(self.int()?)?;
            let (mut names, mut values) = (Vec::new(), Vec::new());
            for _ in 0..len {
                names.push(self.name(ev)?);
                values.push(self.value(ev)?);
            }
            // "If a RecordValue is de-serialized with a potentially different UniqueString
            // table, the RecordValue is not guaranteed to be normalized", so the length's
            // sign is dropped and the record keeps the file's field order.
            let record = Value::Record(Value::record_arc(names, values, false));
            self.assign(slot, Slot::Value(record.clone()));
            return Ok(record);
        }
        if kind == ValueKind::FcnRcd as u8 {
            let slot = self.index();
            let len = usize::try_from(self.nat()?).map_err(|_| malformed())?;
            let info = self.byte()?;
            let fcn = if info == 0 {
                let (lo, hi) = (self.int()?, self.int()?);
                let mut values = Vec::new();
                for _ in 0..len {
                    values.push(self.value(ev)?);
                }
                Value::FcnRcd(Value::fcn_rcd_interval_arc(super::IntervalValue { lo, hi }, values))
            } else {
                let (mut domain, mut values) = (Vec::new(), Vec::new());
                for _ in 0..len {
                    domain.push(self.value(ev)?);
                    values.push(self.value(ev)?);
                }
                Value::FcnRcd(Value::fcn_rcd_arc(domain, values, info == 1))
            };
            self.assign(slot, Slot::Value(fcn.clone()));
            return Ok(fcn);
        }
        if kind == ValueKind::SetEnum as u8 {
            let slot = self.index();
            let signed = self.int()?;
            let len = length(signed)?;
            let mut elems = Vec::new();
            for _ in 0..len {
                elems.push(self.value(ev)?);
            }
            let set = Value::set_enum_of(elems, signed >= 0);
            self.assign(slot, Slot::Value(set.clone()));
            return Ok(set);
        }
        if kind == ValueKind::Tuple as u8 {
            let slot = self.index();
            let len = usize::try_from(self.nat()?).map_err(|_| malformed())?;
            let mut elems = Vec::new();
            for _ in 0..len {
                elems.push(self.value(ev)?);
            }
            let tuple = Value::tuple(elems);
            self.assign(slot, Slot::Value(tuple.clone()));
            return Ok(tuple);
        }
        if kind == ValueKind::Dummy as u8 {
            let slot = self.nat()?;
            return self.value_at(slot);
        }
        Err(cannot_unpickle(kind))
    }
}

/// `len < 0 ? -len : len`, the length a `±length` field carries.
fn length(signed: i32) -> EvalResult<usize> {
    usize::try_from(signed.unsigned_abs()).map_err(|_| malformed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::IntervalValue;
    use crate::value::golden::Fixture;

    /// Replays `tests/golden/value_stream.txt`, produced by TLC's own `ValueOutputStream`
    /// through `tools/golden/ValueStreamGolden.java`. Each value carries its printed form, so
    /// a list that drifted out of step with the Java one fails on the label instead of
    /// comparing bytes of two different values. Every case is also read back, which is the
    /// `_TLCTraceDeserialize` path, and must come out as the bytes it was read from.
    #[test]
    fn matches_tlc_value_stream() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/value_stream.txt");
        let text = std::fs::read_to_string(path).expect("golden file");
        let mut model_values: Vec<(i32, String)> = Vec::new();
        let mut strings: Vec<(i32, String)> = Vec::new();
        let mut cases: Vec<Case> = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f[0] {
                "mv" => model_values.push((f[1].parse().expect("tok"), f[2].to_string())),
                "tok" => strings.push((f[1].parse().expect("tok"), unescape(f[2]))),
                "nat" => cases.push(Case {
                    kind: "nat",
                    bytes: f[1].to_string(),
                    shown: f[2].to_string(),
                    bytes_again: String::new(),
                    shown_again: String::new(),
                }),
                kind @ ("val" | "raw") => cases.push(Case {
                    kind: if kind == "val" { "val" } else { "raw" },
                    bytes: f[1].to_string(),
                    shown: unescape(f[2]),
                    bytes_again: f[3].to_string(),
                    shown_again: unescape(f[4]),
                }),
                other => panic!("unknown line kind {other}"),
            }
        }
        let fixture = Fixture::new(0, &model_values, &strings);
        let mut checked = 0;
        for kind in ["nat", "val", "raw"] {
            for (index, case) in cases.iter().filter(|c| c.kind == kind).enumerate() {
                if kind == "nat" {
                    let mut out = Out { bytes: Vec::new(), handles: HandleTable::default() };
                    out.nat(case.bytes.parse().expect("the number writeNat was given"));
                    assert_eq!(hex(&out.bytes), case.shown, "writeNat of {}", case.bytes);
                    checked += 1;
                    continue;
                }
                fixture.with_ev(|ev| {
                    let values = golden_values(ev, &fixture.model_values, kind);
                    let v = values[index].clone();
                    if kind == "val" {
                        // `ioSerialize` fingerprints before writing, which normalizes.
                        v.fingerprint(ev, crate::fp64::Fp::zero()).expect("fingerprintable");
                    }
                    let written = write(ev, &v).expect("writable");
                    let shown = v.to_tlc_string(ev).expect("printable");
                    assert_eq!(shown, case.shown, "{kind} case {index}");
                    assert_eq!(hex(&written), case.bytes, "{kind} case {index} ({shown})");
                    // The `_TLCTraceDeserialize` side: the value TLC reads back from those
                    // bytes, written out again.
                    let back = read_external(ev, &written).expect("readable");
                    let again = write(ev, &back).expect("writable");
                    assert_eq!(hex(&again), case.bytes_again, "{kind} case {index} ({shown}) read back");
                    assert_eq!(back.to_tlc_string(ev).expect("printable"), case.shown_again, "{kind} case {index}");
                });
                checked += 1;
            }
        }
        assert_eq!(checked, cases.len());
        assert!(checked > 0, "the golden file has cases");
    }

    /// One `nat`, `val` or `raw` line. A `nat` line holds the number in `bytes` and its
    /// encoding in `shown`; the other two hold what the field names say.
    struct Case {
        kind: &'static str,
        bytes: String,
        shown: String,
        bytes_again: String,
        shown_again: String,
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The `\\`, `\t`, `\n` and `\uXXXX` of the golden's labels.
    fn unescape(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('u') => {
                    let digits: String = chars.by_ref().take(4).collect();
                    let cp = u32::from_str_radix(&digits, 16).expect("four hex digits");
                    out.push(char::from_u32(cp).expect("a scalar value"));
                }
                other => panic!("unknown escape {other:?}"),
            }
        }
        out
    }

    /// `ValueStreamGolden.fingerprinted` and `.raw`, in the same order.
    fn golden_values(ev: &mut Ev<'_>, mv: &[super::super::ModelValue], kind: &str) -> Vec<Value> {
        let int = Value::Int;
        let set = Value::set_enum;
        let tuple = Value::tuple;
        let s = |t: &str| Value::Str(ev.tool().graph().interner().intern(t));
        let atom = |t: &str| ev.tool().graph().interner().intern(t);
        let record = |names: &[&str], values: Vec<Value>| {
            Value::Record(Value::record_arc(names.iter().map(|n| atom(n)).collect(), values, false))
        };
        let interval = |lo, hi| Value::Interval(IntervalValue { lo, hi });
        let fcn = |domain, values, is_norm| Value::FcnRcd(Value::fcn_rcd_arc(domain, values, is_norm));
        let fcn_interval = |lo, hi, values| Value::FcnRcd(Value::fcn_rcd_interval_arc(IntervalValue { lo, hi }, values));
        if kind == "raw" {
            return vec![
                record(&["zz", "aa"], vec![int(1), int(2)]),
                Value::Record(Value::record_arc(vec![atom("aa"), atom("zz")], vec![int(1), int(2)], true)),
                set(vec![int(3), int(1), int(3)]),
                Value::set_enum_normalized(vec![int(1), int(2)]),
                fcn(vec![int(2), int(1)], vec![int(3), int(4)], false),
                fcn(vec![int(1), int(2)], vec![int(3), int(4)], true),
                tuple(vec![set(vec![int(2), int(1)]), Value::Model(mv[2])]),
            ];
        }
        let mut values = vec![Value::Bool(true), Value::Bool(false)];
        values.extend([0, 1, -1, 127, 128, 32767, 32768, i32::MAX, i32::MIN].map(int));
        values.extend(GOLDEN_STRINGS.iter().map(|t| s(t)));
        values.extend([Value::Model(mv[0]), Value::Model(mv[1]), Value::Model(mv[3])]);
        values.extend([interval(1, 3), interval(5, 4), interval(-2, 2), interval(0, 0)]);
        values.push(tuple(vec![]));
        values.push(tuple(vec![int(1)]));
        values.push(tuple(vec![s("x"), s("x")]));
        values.push(tuple(vec![tuple(vec![int(1)]), set(vec![int(2)])]));
        let shared = tuple(vec![int(7), s("q")]);
        values.push(tuple(vec![shared.clone(), int(0), shared]));
        values.push(record(&["x"], vec![int(1)]));
        values.push(record(&["bb", "x"], vec![int(1), int(2)]));
        values.push(record(&["x"], vec![s("x")]));
        let first = record(&["field", "other"], vec![int(1), int(2)]);
        let second = record(&["field", "other"], vec![int(3), int(4)]);
        values.push(tuple(vec![first, second]));
        let shared_record = record(&["name"], vec![s("abc")]);
        values.push(tuple(vec![shared_record.clone(), shared_record]));
        values.push(set(vec![]));
        values.push(set(vec![int(1)]));
        values.push(set(vec![int(3), int(1), int(2)]));
        values.push(set(vec![s("zz"), s("aa")]));
        values.push(set(vec![set(vec![int(2), int(1)]), set(vec![])]));
        values.push(set(vec![Value::Model(mv[1]), Value::Model(mv[0])]));
        let shared_set = set(vec![int(5), int(6)]);
        values.push(tuple(vec![shared_set.clone(), shared_set]));
        values.push(fcn(vec![s("bb"), s("aa")], vec![int(1), int(2)], false));
        values.push(fcn(vec![int(2), int(1)], vec![s("q"), s("x")], false));
        values.push(fcn_interval(1, 3, vec![int(7), int(8), int(9)]));
        values.push(fcn_interval(-1, 1, vec![int(7), int(8), int(9)]));
        let subset = crate::value::subset(set(vec![int(1), int(2)]));
        values.push(subset.clone());
        values.push(tuple(vec![subset.clone(), subset]));
        let state = record(&["x"], vec![int(0)]);
        let pair = tuple(vec![int(1), state]);
        values.push(record(&["name", "other"], vec![set(vec![pair]), s("abc")]));
        values
    }

    /// `ValueStreamGolden.STRINGS`, in the same order.
    const GOLDEN_STRINGS: [&str; 14] = [
        "", "x", "abc", "aa", "\u{100}A", "\u{0}A", "\u{80}\u{ff}", "\u{ff80}\u{ffff}", "bb", "zz", "q", "field",
        "name", "other",
    ];
}
