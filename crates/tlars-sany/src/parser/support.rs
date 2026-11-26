//! Hand-written classes the translated parser calls (signatures in `xtask`'s
//! `jj2rs/externs.rs`): the Java library subset, `UniqueString`, `Location`, `SanyOutput`,
//! `JunctionListContext`, and the translated members listed as hand-written there.

#![expect(non_snake_case, dead_code, reason = "the Java library subset generated.rs calls by its Java names; it calls only part of it")]

pub(crate) use super::files::{FileUtil, FilenameToStream, NamedInputStream, ToolIO};
use super::generated::{
    self, Debug, Errors, LevelConstants, ParamAndPosition, ParseUnit, SemanticNodeObj, SimpleCharStream, SpecObj, TLAplusParser,
    TLAplusParserConstants, TLAtoPCalMapping, trans,
};
use super::java::*;
use crate::{Atom, Interner};
use std::cell::RefCell;
use std::sync::Arc;

// ---- session ----

thread_local! {
    static INTERNER: RefCell<Option<Arc<Interner>>> = const { RefCell::new(None) };
    /// The load's [`crate::Host`], and how often a fixed clock has been read.
    static HOST: RefCell<Option<(crate::Host, i64)>> = const { RefCell::new(None) };
}

/// Sets what this thread's parser reads as the time and the temporary directory.
pub(crate) fn set_host(host: crate::Host) {
    HOST.with(|h| *h.borrow_mut() = Some((host, 0)));
}

/// The host a test parses under: a fixed clock and this process's temporary directory.
#[cfg(test)]
pub(crate) fn test_host() -> crate::Host {
    crate::Host::process(tlars_java::time::Clock::Fixed { epoch_millis: 0, utc_offset_minutes: 0 })
}

pub(crate) fn host() -> crate::Host {
    HOST.with(|h| h.borrow().as_ref().map(|(host, _)| host.clone()))
        .expect("SANY read the host outside a load: pass LoadRequest::host, or call support::set_host in a test")
}

/// Makes `interner` the one `UniqueString` uses on this thread. Java statics belong to the
/// JVM, which is the session here: switching to another session's interner forgets them.
pub(crate) fn enter(interner: &Arc<Interner>) {
    let same = INTERNER.with(|i| i.borrow().as_ref().is_some_and(|cur| Arc::ptr_eq(cur, interner)));
    if !same {
        generated::reset_all_statics();
        LOCATION_UNKNOWN.with(|u| *u.borrow_mut() = None);
        LOCATION_NULL.with(|l| *l.borrow_mut() = None);
        INTERNER.with(|i| *i.borrow_mut() = Some(interner.clone()));
    }
}

fn interner() -> Arc<Interner> {
    INTERNER.with(|i| i.borrow().clone()).expect("parser used outside `support::enter`")
}

// ---- java.lang ----

pub(crate) struct Integer;

impl Integer {
    pub(crate) const MAX_VALUE: i32 = i32::MAX;
    pub(crate) const MIN_VALUE: i32 = i32::MIN;

    /// A malformed number is `NumberFormatException`, which TLC does not catch here.
    #[track_caller]
    pub(crate) fn parseInt_1(s: JStr) -> i32 {
        tlars_java::parse_int(s.as_str()).unwrap_or_else(|e| panic!("java.lang.NumberFormatException: {e:?}"))
    }

    /// `Integer.parseInt(s, radix)`; `NumberFormatException` is caught by the callers.
    pub(crate) fn parseInt_2(s: JStr, radix: i32) -> JResult<i32> {
        let t = s.as_str();
        // `from_str_radix` accepts what Java does: an optional sign, then at least one digit.
        let parsed = if (2..=36).contains(&radix) { i32::from_str_radix(t, radix as u32).ok() } else { None };
        parsed.ok_or_else(|| JThrow::new_extern("NumberFormatException", JStr::of(&format!("For input string: \"{t}\""))))
    }

    pub(crate) fn valueOf(i: i32) -> JInteger {
        JInteger::of(i)
    }

    pub(crate) fn toString_1(i: i32) -> JStr {
        JStr::of(&i.to_string())
    }

    /// `Integer.toString(i, radix)`: lowercase digits, `-` for negatives.
    pub(crate) fn toString_2(i: i32, radix: i32) -> JStr {
        let radix = if (2..=36).contains(&radix) { radix as u32 } else { 10 };
        let mut n = i64::from(i).unsigned_abs();
        let mut digits = Vec::new();
        loop {
            digits.push(std::char::from_digit((n % u64::from(radix)) as u32, radix).expect("digit"));
            n /= u64::from(radix);
            if n == 0 {
                break;
            }
        }
        if i < 0 {
            digits.push('-');
        }
        JStr::of(&digits.iter().rev().collect::<String>())
    }
}

pub(crate) struct Long;

impl Long {
    /// `Long.parseLong`; overflow is `NumberFormatException`, which `DecimalNode` catches.
    pub(crate) fn parseLong(s: JStr) -> JResult<i64> {
        tlars_java::parse_long(s.as_str())
            .map_err(|_| JThrow::new_extern("NumberFormatException", JStr::of(&format!("For input string: \"{}\"", s.as_str()))))
    }
    pub(crate) fn toString(v: i64) -> JStr {
        JStr::of(&v.to_string())
    }
}

pub(crate) struct Math;

impl Math {
    pub(crate) fn max(a: i32, b: i32) -> i32 {
        a.max(b)
    }
    /// `Math.abs(int)`: `Integer.MIN_VALUE` stays negative.
    pub(crate) fn abs(a: i32) -> i32 {
        a.wrapping_abs()
    }
    pub(crate) fn min(a: i32, b: i32) -> i32 {
        a.min(b)
    }
}

pub(crate) struct Boolean;

impl Boolean {
    pub(crate) fn valueOf(b: bool) -> bool {
        b
    }
}

pub(crate) struct Arrays;

impl Arrays {
    /// `Arrays.equals(Object[], Object[])`: element-wise `equals`, `null` arrays equal only
    /// each other.
    pub(crate) fn equals<T: JValue>(a: JArr<T>, b: JArr<T>) -> bool {
        if a.is_null() || b.is_null() {
            return a.is_null() && b.is_null();
        }
        let (a, b) = (a.jget(), b.jget());
        a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| if x.is_java_null() { y.is_java_null() } else { x.java_equals(y) })
    }
    pub(crate) fn copyOf(a: JArr<i32>, n: i32) -> JArr<i32> {
        let mut v = a.jget().clone();
        v.resize(index(n), 0);
        J::new(v)
    }
}

/// `java.util.concurrent.atomic.AtomicInteger`, shared by clones (the static `uid` counter).
#[derive(Clone, Default)]
pub(crate) struct AtomicInteger(std::rc::Rc<std::cell::Cell<i32>>);

impl AtomicInteger {
    pub(crate) fn new_0() -> AtomicInteger {
        AtomicInteger::default()
    }
    pub(crate) fn getAndIncrement(&self) -> i32 {
        let v = self.0.get();
        self.0.set(v.wrapping_add(1));
        v
    }
}

impl JNull for AtomicInteger {
    fn null() -> Self {
        AtomicInteger::default()
    }
}

/// `java.math.BigInteger`, only as `NumeralNode` uses it: built from digits, printed.
#[derive(Clone, Default)]
pub(crate) struct BigInteger(JStr);

impl BigInteger {
    pub(crate) fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

impl BigInteger {
    /// `new BigInteger(digits, radix)` for an unsigned numeral; printed in decimal.
    pub(crate) fn new_2(digits: JStr, radix: i32) -> BigInteger {
        // Little-endian base-10^9 limbs.
        let mut limbs: Vec<u64> = vec![0];
        for c in digits.as_str().chars() {
            let d = u64::from(c.to_digit(radix as u32).unwrap_or_else(|| panic!("java.lang.NumberFormatException: {}", digits.as_str())));
            let mut carry = d;
            for l in limbs.iter_mut() {
                let v = *l * radix as u64 + carry;
                *l = v % 1_000_000_000;
                carry = v / 1_000_000_000;
            }
            while carry > 0 {
                limbs.push(carry % 1_000_000_000);
                carry /= 1_000_000_000;
            }
        }
        let mut out = limbs.last().expect("limb").to_string();
        for l in limbs.iter().rev().skip(1) {
            out.push_str(&format!("{l:09}"));
        }
        BigInteger(JStr::of(&out))
    }
    pub(crate) fn toString(&self) -> JStr {
        self.0.clone()
    }
}

impl JNull for BigInteger {
    fn null() -> Self {
        BigInteger::default()
    }
}

/// `java.math.BigDecimal`, only as `DecimalNode` uses it: built from `"digits.digits"`,
/// printed with `BigDecimal.toString` (scientific notation when the adjusted exponent is
/// below -6).
#[derive(Clone, Default)]
pub(crate) struct BigDecimal(JStr);

impl BigDecimal {
    pub(crate) fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

impl BigDecimal {
    pub(crate) fn new_1(s: JStr) -> BigDecimal {
        let text = s.as_str();
        let (int, frac) = text.split_once('.').unwrap_or((text, ""));
        let digits = format!("{int}{frac}");
        let unscaled = digits.trim_start_matches('0');
        let unscaled = if unscaled.is_empty() { "0" } else { unscaled };
        let scale = frac.len() as i64;
        let adjusted = -scale + (unscaled.len() as i64 - 1);
        let out = if adjusted >= -6 {
            if scale == 0 {
                unscaled.to_string()
            } else if (unscaled.len() as i64) > scale {
                let (a, b) = unscaled.split_at(unscaled.len() - scale as usize);
                format!("{a}.{b}")
            } else {
                format!("0.{}{unscaled}", "0".repeat((scale as usize) - unscaled.len()))
            }
        } else {
            let (first, rest) = unscaled.split_at(1);
            let mantissa = if rest.is_empty() { first.to_string() } else { format!("{first}.{rest}") };
            format!("{mantissa}E{adjusted}")
        };
        BigDecimal(JStr::of(&out))
    }
    pub(crate) fn toString(&self) -> JStr {
        self.0.clone()
    }
}

impl JNull for BigDecimal {
    fn null() -> Self {
        BigDecimal::default()
    }
}

/// `java.lang.Class`, by simple name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct JClass(pub(crate) &'static str);

impl JClass {
    pub(crate) fn same(&self, other: &JClass) -> bool {
        self == other
    }
    pub(crate) fn is_null(&self) -> bool {
        false
    }
    pub(crate) fn isInstance(&self, o: J<Object>) -> bool {
        o.java_instance_of(self.0)
    }
    pub(crate) fn getName(&self) -> JStr {
        JStr::of(generated::java_class_name(self.0))
    }
}

impl JNull for JClass {
    fn null() -> Self {
        JClass("null")
    }
}

/// `Class.toString()`: `class <name>`.
impl JToString for JClass {
    fn java_string(&self) -> String {
        self.object_string()
    }
}

impl JValue for JClass {
    fn is_java_null(&self) -> bool {
        false
    }
    fn object_string(&self) -> String {
        format!("class {}", generated::java_class_name(self.0))
    }
    fn java_hash(&self) -> i32 {
        tlars_java::string_hash_code(self.0)
    }
    fn java_equals(&self, other: &Self) -> bool {
        self == other
    }
}

/// `Object.toString()`: `getClass().getName() + "@" + Integer.toHexString(hashCode())`.
pub(crate) fn object_to_string(class: &str, hash: i32) -> String {
    format!("{}@{:x}", generated::java_class_name(class), hash as u32)
}

/// `tlc2.output.MP`.
pub(crate) struct MP;

impl MP {
    /// `MP.getMessage(code)`: the bare message (class NONE, no parameters).
    pub(crate) fn getMessage(code: i32) -> JStr {
        let ec = tlars_msg::Ec::from_code(code).unwrap_or(tlars_msg::Ec::UNKNOWN);
        let cx = tlars_msg::RenderCx { class: tlars_msg::MsgClass::None, tool_mode: false, now: String::new(), debug: false };
        JStr::of(&tlars_msg::render(ec, &[], &cx))
    }
}

/// `tlc2.tool.BuiltInOPs`: TLC's operator codes, used by TLC on the semantic graph.
pub(crate) struct BuiltInOPs;

impl BuiltInOPs {
    pub(crate) fn getOpCode(_name: UniqueString) -> i32 {
        todo!("M2: TLC's BuiltInOPs table")
    }
}

pub(crate) struct Character;

impl Character {
    /// `Character.isLetter(char)`: general categories Lu, Ll, Lt, Lm and Lo. ASCII is exact;
    /// beyond it, Rust's `Alphabetic` stands in (it also admits letter numbers and some
    /// marks, which Java does not).
    pub(crate) fn isLetter(c: u16) -> bool {
        match char::from_u32(u32::from(c)) {
            Some(ch) if ch.is_ascii() => ch.is_ascii_alphabetic(),
            Some(ch) => ch.is_alphabetic(),
            None => false,
        }
    }
    /// `Character.isDigit(char)`: category Nd. ASCII is exact; beyond it, Rust's `Numeric`
    /// stands in.
    pub(crate) fn isDigit(c: u16) -> bool {
        match char::from_u32(u32::from(c)) {
            Some(ch) if ch.is_ascii() => ch.is_ascii_digit(),
            Some(ch) => ch.is_numeric(),
            None => false,
        }
    }
    pub(crate) fn isLetterOrDigit(c: u16) -> bool {
        Self::isLetter(c) || Self::isDigit(c)
    }
    /// `Character.toString(int codePoint)`.
    pub(crate) fn toString(code_point: i32) -> JStr {
        let ch = u32::try_from(code_point).ok().and_then(char::from_u32).expect("IllegalArgumentException");
        JStr::of(&ch.to_string())
    }
    /// `Character.isWhitespace`: space separators except no-break spaces, plus the ASCII
    /// controls Java lists.
    pub(crate) fn isWhitespace(c: u16) -> bool {
        match c {
            0x09..=0x0D | 0x1C..=0x1F => true,
            0x00A0 | 0x2007 | 0x202F => false,
            _ => char::from_u32(u32::from(c)).is_some_and(|ch| {
                matches!(ch, ' ' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{205F}' | '\u{3000}')
            }),
        }
    }
}

pub(crate) struct System;

thread_local! {
    /// Properties set with `System.setProperty` (PlusCal's `-unixEOL`).
    static SYSTEM_PROPERTIES: RefCell<std::collections::BTreeMap<String, JStr>> = RefCell::default();
}

impl System {
    pub(crate) fn out() -> J<PrintStream> {
        J::new(PrintStream::Stdout)
    }

    /// Copies as if through a temporary array, so overlapping ranges work.
    #[track_caller]
    pub(crate) fn arraycopy<T: Clone>(src: &JArr<T>, src_pos: i32, dest: &JArr<T>, dest_pos: i32, length: i32) {
        let (s, d, n) = (index(src_pos), index(dest_pos), index(length));
        let tmp: Vec<T> = src.jget()[s..s + n].to_vec();
        dest.jget_mut()[d..d + n].clone_from_slice(&tmp);
    }

    /// The platform properties SANY reads (Linux); TLC runs without the `-D` switches SANY
    /// reads.
    pub(crate) fn getProperty_2(key: JStr, default: JStr) -> JStr {
        if let Some(v) = SYSTEM_PROPERTIES.with(|p| p.borrow().get(key.as_str()).cloned()) {
            return v;
        }
        match key.as_str() {
            "line.separator" => JStr::of("\n"),
            "file.separator" => JStr::of("/"),
            _ => default,
        }
    }
    pub(crate) fn getProperty_1(key: JStr) -> JStr {
        Self::getProperty_2(key, JStr::null())
    }
    /// `System.setProperty`: returns the previous value.
    pub(crate) fn setProperty(key: JStr, value: JStr) -> JStr {
        let previous = Self::getProperty_1(key.clone());
        SYSTEM_PROPERTIES.with(|p| p.borrow_mut().insert(key.as_str().to_string(), value));
        previous
    }
    /// A fixed clock advances one millisecond per reading, so a loop that waits for the
    /// next millisecond (`ModuleNode.generateUnusedName`) ends, and every run reads the
    /// same times.
    pub(crate) fn currentTimeMillis() -> i64 {
        HOST.with(|h| {
            let mut h = h.borrow_mut();
            let (host, reads) = h.as_mut().expect("SANY read the clock outside a load: pass LoadRequest::host");
            match host.clock {
                tlars_java::time::Clock::System => host.clock.now_millis(),
                tlars_java::time::Clock::Fixed { epoch_millis, .. } => {
                    *reads += 1;
                    epoch_millis + *reads
                }
            }
        })
    }
}

#[track_caller]
fn index(i: i32) -> usize {
    usize::try_from(i).expect("ArrayIndexOutOfBoundsException")
}

/// `System.out`: only debugging dumps in the translated code write to it.
/// A `PrintStream`: `System.out`, or `ToolIO.out`/`ToolIO.err`, whose lines are kept for
/// the caller (`files::take_tool_io`).
pub(crate) enum PrintStream {
    Stdout,
    Tool { is_err: bool, pending: String },
}

impl PrintStream {
    #[expect(clippy::disallowed_macros, reason = "Java's System.out, which only the translated debugging dumps write to")]
    fn write(&mut self, text: &str) {
        match self {
            PrintStream::Stdout => print!("{text}"),
            PrintStream::Tool { is_err, pending } => {
                pending.push_str(text);
                while let Some(n) = pending.find('\n') {
                    let line: String = pending.drain(..=n).collect();
                    super::files::tool_println(*is_err, line.trim_end_matches('\n').to_string());
                }
            }
        }
    }
}

impl J<PrintStream> {
    pub(crate) fn print(&self, x: &dyn JToString) {
        self.jget_mut().write(&x.java_string());
    }
    pub(crate) fn println_1(&self, x: &dyn JToString) {
        self.jget_mut().write(&format!("{}\n", x.java_string()));
    }
    pub(crate) fn println_0(&self) {
        self.jget_mut().write("\n");
    }
}

pub(crate) struct StringBuffer(String);

impl StringBuffer {
    pub(crate) fn new_0() -> J<StringBuffer> {
        J::new(StringBuffer(String::new()))
    }
    pub(crate) fn new_1_0(s: JStr) -> J<StringBuffer> {
        J::new(StringBuffer(s.as_str().to_string()))
    }
    /// The capacity carries no semantics.
    pub(crate) fn new_1_1(_capacity: i32) -> J<StringBuffer> {
        Self::new_0()
    }
}

impl J<StringBuffer> {
    pub(crate) fn append_1_0(&self, x: &dyn JToString) -> J<StringBuffer> {
        self.jget_mut().0.push_str(&x.java_string());
        self.clone()
    }
    /// `append(char[])` appends the characters (not the array's `toString`).
    pub(crate) fn append_1_1(&self, chars: JArr<u16>) -> J<StringBuffer> {
        let s = String::from_utf16_lossy(&chars.jget());
        self.jget_mut().0.push_str(&s);
        self.clone()
    }
    /// Truncates (or pads with `\0`) to `len` UTF-16 units.
    pub(crate) fn setLength(&self, len: i32) {
        let mut units: Vec<u16> = self.jget().0.encode_utf16().collect();
        units.resize(index(len), 0);
        self.jget_mut().0 = String::from_utf16_lossy(&units);
    }
    pub(crate) fn length(&self) -> i32 {
        self.jget().0.encode_utf16().count() as i32
    }
    pub(crate) fn toString(&self) -> JStr {
        JStr::of(&self.jget().0)
    }
}

impl JToString for J<StringBuffer> {
    fn java_string(&self) -> String {
        self.jget().0.clone()
    }
}

// ---- java.util ----

/// A key as Java tables see it: `hashCode` and `equals` from [`JValue`].
#[derive(Clone)]
struct Key<K>(K);

impl<K: JValue> PartialEq for Key<K> {
    fn eq(&self, other: &Self) -> bool {
        self.0.java_equals(&other.0)
    }
}
impl<K: JValue> Eq for Key<K> {}

/// `java.util.Vector` and `tla2sany.utilities.Vector` (same behaviour for what the parser
/// uses).
pub(crate) struct Vector<E>(Vec<E>);

impl<E> Vector<E> {
    pub(crate) fn new_0() -> J<Vector<E>> {
        J::new(Vector(Vec::new()))
    }
    pub(crate) fn new_1(_capacity: i32) -> J<Vector<E>> {
        Self::new_0()
    }
}

impl<E: Clone> J<Vector<E>> {
    pub(crate) fn addElement(&self, e: E) {
        self.jget_mut().0.push(e);
    }
    #[track_caller]
    pub(crate) fn elementAt(&self, i: i32) -> E {
        self.jget().0[index(i)].clone()
    }
    #[track_caller]
    pub(crate) fn setElementAt(&self, e: E, i: i32) {
        self.jget_mut().0[index(i)] = e;
    }
    #[track_caller]
    pub(crate) fn removeElementAt(&self, i: i32) {
        self.jget_mut().0.remove(index(i));
    }
    pub(crate) fn removeAllElements(&self) {
        self.jget_mut().0.clear();
    }
    pub(crate) fn size(&self) -> i32 {
        self.jget().0.len() as i32
    }
    pub(crate) fn isEmpty(&self) -> bool {
        self.jget().0.is_empty()
    }
    /// A snapshot; the translated code never mutates a collection while enumerating it.
    pub(crate) fn elements(&self) -> J<Enumeration<E>> {
        J::new(Enumeration { items: self.jget().0.clone(), next: 0 })
    }
}

/// `java.util.Vector` (PlusCal's), erased as in Java: one type for every element type,
/// holding `Object`s. Accessors take the element type the caller expects (a turbofish from
/// the translation) and cast to it. Unlike `tla2sany.utilities.Vector`, `contains` and
/// `indexOf` compare with `equals`.
pub(crate) struct JavaVector(pub(crate) Vec<J<Object>>);

impl JavaVector {
    pub(crate) fn new_0() -> J<JavaVector> {
        J::new(JavaVector(Vec::new()))
    }
    pub(crate) fn new_1_0(_capacity: i32) -> J<JavaVector> {
        Self::new_0()
    }
    pub(crate) fn new_2(_capacity: i32, _increment: i32) -> J<JavaVector> {
        Self::new_0()
    }
    /// `new Vector<>(collection)`: a copy.
    pub(crate) fn new_1_1(c: J<JavaVector>) -> J<JavaVector> {
        J::new(JavaVector(c.jget().0.clone()))
    }
}

impl J<JavaVector> {
    pub(crate) fn add_1<E: JValue>(&self, e: E) -> bool {
        self.jget_mut().0.push(Object::of(e));
        true
    }
    #[track_caller]
    pub(crate) fn add_2<E: JValue>(&self, i: i32, e: E) {
        let i = index(i);
        let mut v = self.jget_mut();
        assert!(i <= v.0.len(), "ArrayIndexOutOfBoundsException");
        v.0.insert(i, Object::of(e));
    }
    pub(crate) fn addAll(&self, other: J<JavaVector>) -> bool {
        let items = other.jget().0.clone();
        let changed = !items.is_empty();
        self.jget_mut().0.extend(items);
        changed
    }
    pub(crate) fn addElement<E: JValue>(&self, e: E) {
        self.jget_mut().0.push(Object::of(e));
    }
    #[track_caller]
    pub(crate) fn insertElementAt<E: JValue>(&self, e: E, i: i32) {
        self.add_2(i, e);
    }
    #[track_caller]
    pub(crate) fn elementAt<E: JValue>(&self, i: i32) -> E {
        self.jget().0.get(index(i)).expect("ArrayIndexOutOfBoundsException").cast::<E>()
    }
    #[track_caller]
    pub(crate) fn get<E: JValue>(&self, i: i32) -> E {
        self.elementAt(i)
    }
    #[track_caller]
    pub(crate) fn set<E: JValue>(&self, i: i32, e: E) -> E {
        let mut v = self.jget_mut();
        let slot = v.0.get_mut(index(i)).expect("ArrayIndexOutOfBoundsException");
        std::mem::replace(slot, Object::of(e)).cast::<E>()
    }
    #[track_caller]
    pub(crate) fn setElementAt<E: JValue>(&self, e: E, i: i32) {
        let _: E = self.set(i, e);
    }
    #[track_caller]
    pub(crate) fn firstElement<E: JValue>(&self) -> E {
        self.jget().0.first().expect("java.util.NoSuchElementException").cast::<E>()
    }
    #[track_caller]
    pub(crate) fn lastElement<E: JValue>(&self) -> E {
        self.jget().0.last().expect("java.util.NoSuchElementException").cast::<E>()
    }
    #[track_caller]
    pub(crate) fn remove<E: JValue>(&self, i: i32) -> E {
        let i = index(i);
        let mut v = self.jget_mut();
        assert!(i < v.0.len(), "ArrayIndexOutOfBoundsException");
        v.0.remove(i).cast::<E>()
    }
    #[track_caller]
    pub(crate) fn removeElementAt(&self, i: i32) {
        let _: J<Object> = self.remove(i);
    }
    pub(crate) fn removeAllElements(&self) {
        self.jget_mut().0.clear();
    }
    pub(crate) fn size(&self) -> i32 {
        self.jget().0.len() as i32
    }
    pub(crate) fn isEmpty(&self) -> bool {
        self.jget().0.is_empty()
    }
    /// A snapshot; the translated code never mutates a vector while enumerating it.
    pub(crate) fn elements<E: JValue>(&self) -> J<Enumeration<E>> {
        J::new(Enumeration { items: self.jget().0.iter().map(|o| o.cast::<E>()).collect(), next: 0 })
    }
    pub(crate) fn iterator<E: JValue>(&self) -> J<Iterator<E>> {
        Iterator::over(self.jget().0.iter().map(|o| o.cast::<E>()).collect())
    }
    pub(crate) fn contains<E: JValue>(&self, e: E) -> bool {
        self.indexOf(e) >= 0
    }
    /// `indexOf(o)`: the first element `o.equals`, or null for a null `o`.
    pub(crate) fn indexOf<E: JValue>(&self, e: E) -> i32 {
        let o = Object::of(e);
        let v = self.jget();
        let found = if o.is_null() { v.0.iter().position(J::is_null) } else { v.0.iter().position(|x| o.java_equals(x)) };
        found.map_or(-1, |i| i as i32)
    }
    pub(crate) fn toString(&self) -> JStr {
        JStr::of(&self.java_string())
    }
}

/// `AbstractCollection.toString`: `[a, b]`.
impl JToString for J<JavaVector> {
    fn java_string(&self) -> String {
        if self.is_null() {
            return "null".into();
        }
        let items: Vec<String> = self.jget().0.iter().map(JValue::object_string).collect();
        format!("[{}]", items.join(", "))
    }
}

/// `Vector.equals` and `hashCode` are `AbstractList`'s: element by element.
impl JValue for J<JavaVector> {
    fn is_java_null(&self) -> bool {
        self.is_null()
    }
    fn object_string(&self) -> String {
        self.java_string()
    }
    fn java_hash(&self) -> i32 {
        self.jget().0.iter().fold(1i32, |h, e| h.wrapping_mul(31).wrapping_add(if e.is_null() { 0 } else { e.java_hash() }))
    }
    fn java_equals(&self, other: &Self) -> bool {
        if other.is_null() {
            return false;
        }
        if self.same(other) {
            return true;
        }
        let (a, b) = (self.jget(), other.jget());
        a.0.len() == b.0.len() && a.0.iter().zip(&b.0).all(|(x, y)| if x.is_null() { y.is_null() } else { x.java_equals(y) })
    }
    fn java_instance_of(&self, class: &str) -> bool {
        matches!(class, "Vector" | "JavaVector" | "AbstractList" | "List" | "Collection" | "Object")
    }
    fn java_class(&self) -> &'static str {
        "Vector"
    }
}

/// Element identity (`==`), which `tla2sany.utilities.Vector.contains` compares by.
pub(crate) trait JSame {
    fn same_object(&self, other: &Self) -> bool;
}
impl<T: ?Sized> JSame for J<T> {
    fn same_object(&self, other: &Self) -> bool {
        self.same(other)
    }
}
impl JSame for JStr {
    fn same_object(&self, other: &Self) -> bool {
        self.same(other)
    }
}

impl<E: Clone + JSame> J<Vector<E>> {
    pub(crate) fn contains(&self, e: E) -> bool {
        self.jget().0.iter().any(|x| x.same_object(&e))
    }
    pub(crate) fn appendNoRepeats(&self, v: J<Vector<E>>) {
        for i in 0..v.size() {
            let e = v.elementAt(i);
            if !self.contains(e.clone()) {
                self.addElement(e);
            }
        }
    }
}

/// `tla2sany.utilities.Vector.toString`: `[ a, b ]`.
impl<E: JValue> JToString for J<Vector<E>> {
    fn java_string(&self) -> String {
        if self.is_null() {
            return "null".into();
        }
        let items: Vec<String> = self.jget().0.iter().map(JValue::object_string).collect();
        if items.is_empty() { "[  ]".into() } else { format!("[ {} ]", items.join(", ")) }
    }
}

pub(crate) struct Enumeration<E> {
    items: Vec<E>,
    next: usize,
}

impl<E: Clone> J<Enumeration<E>> {
    pub(crate) fn hasMoreElements(&self) -> bool {
        let e = self.jget();
        e.next < e.items.len()
    }
    #[track_caller]
    pub(crate) fn nextElement(&self) -> E {
        let mut e = self.jget_mut();
        let item = e.items.get(e.next).cloned().expect("java.util.NoSuchElementException");
        e.next += 1;
        item
    }
}

/// `java.util.Iterator` over a snapshot (no `remove`; the sources never use it).
pub(crate) struct Iterator<E> {
    items: Vec<E>,
    next: usize,
}

impl<E> Iterator<E> {
    fn over(items: Vec<E>) -> J<Iterator<E>> {
        J::new(Iterator { items, next: 0 })
    }
}

impl<E: Clone> J<Iterator<E>> {
    pub(crate) fn hasNext(&self) -> bool {
        let it = self.jget();
        it.next < it.items.len()
    }
    #[track_caller]
    pub(crate) fn next(&self) -> E {
        let mut it = self.jget_mut();
        let item = it.items.get(it.next).cloned().expect("java.util.NoSuchElementException");
        it.next += 1;
        item
    }
}

/// `java.util.Hashtable`: enumeration order from `tlars_java::Hashtable`.
pub(crate) struct Hashtable<K, V>(tlars_java::Hashtable<Key<K>, V>);

impl<K: JValue, V> Hashtable<K, V> {
    pub(crate) fn new_0() -> J<Hashtable<K, V>> {
        J::new(Hashtable(tlars_java::Hashtable::new()))
    }
    /// `new Hashtable<>(initialCapacity)`, which changes the enumeration order.
    pub(crate) fn new_1(initial_capacity: i32) -> J<Hashtable<K, V>> {
        let capacity = usize::try_from(initial_capacity).expect("IllegalArgumentException");
        J::new(Hashtable(tlars_java::Hashtable::with_capacity(capacity)))
    }
}

impl<K: JValue, V: Clone + JNull> J<Hashtable<K, V>> {
    pub(crate) fn get(&self, k: K) -> V {
        let h = k.java_hash();
        self.jget().0.get(&Key(k), h).cloned().unwrap_or_else(V::null)
    }
    pub(crate) fn put(&self, k: K, v: V) -> V {
        let h = k.java_hash();
        self.jget_mut().0.put(Key(k), h, v).unwrap_or_else(V::null)
    }
    pub(crate) fn remove(&self, k: K) -> V {
        let h = k.java_hash();
        self.jget_mut().0.remove(&Key(k), h).unwrap_or_else(V::null)
    }
    pub(crate) fn containsKey(&self, k: K) -> bool {
        let h = k.java_hash();
        self.jget().0.get(&Key(k), h).is_some()
    }
    pub(crate) fn size(&self) -> i32 {
        self.jget().0.len() as i32
    }
    pub(crate) fn isEmpty(&self) -> bool {
        self.jget().0.is_empty()
    }
    pub(crate) fn keys(&self) -> J<Enumeration<K>> {
        J::new(Enumeration { items: self.jget().0.elements().map(|(k, _)| k.0.clone()).collect(), next: 0 })
    }
    pub(crate) fn elements(&self) -> J<Enumeration<V>> {
        J::new(Enumeration { items: self.jget().0.elements().map(|(_, v)| v.clone()).collect(), next: 0 })
    }
    /// `values()`, as a snapshot in enumeration order.
    pub(crate) fn values(&self) -> J<ArrayList<V>> {
        J::new(ArrayList(self.jget().0.elements().map(|(_, v)| v.clone()).collect()))
    }
}

/// `java.util.HashMap` (JDK 21 iteration order).
pub(crate) struct HashMap<K, V>(tlars_java::JavaHashMap<K, V>);

impl<K: JValue, V> HashMap<K, V> {
    pub(crate) fn new_0() -> J<HashMap<K, V>> {
        J::new(HashMap(tlars_java::JavaHashMap::new()))
    }
}

impl<K: JValue, V: Clone + JNull> J<HashMap<K, V>> {
    pub(crate) fn get(&self, k: K) -> V {
        let h = k.java_hash();
        self.jget().0.get(&k, h, |a, b| a.java_equals(b)).cloned().unwrap_or_else(V::null)
    }
    pub(crate) fn put(&self, k: K, v: V) -> V {
        let h = k.java_hash();
        self.jget_mut().0.put(k, h, v, |a, b| a.java_equals(b)).unwrap_or_else(V::null)
    }
    pub(crate) fn containsKey(&self, k: K) -> bool {
        let h = k.java_hash();
        self.jget().0.get(&k, h, |a, b| a.java_equals(b)).is_some()
    }
    pub(crate) fn size(&self) -> i32 {
        self.jget().0.len() as i32
    }
}

/// `java.util.HashSet` (a `HashMap` with dummy values, JDK 21 iteration order).
pub(crate) struct HashSet<E>(tlars_java::JavaHashMap<E, ()>);

impl<E: JValue> HashSet<E> {
    pub(crate) fn new_0() -> J<HashSet<E>> {
        J::new(HashSet(tlars_java::JavaHashMap::new()))
    }
    pub(crate) fn new_1_0(capacity: i32) -> J<HashSet<E>> {
        J::new(HashSet(tlars_java::JavaHashMap::with_capacity(index(capacity))))
    }
    /// `new HashSet<>(c)`: `HashMap.newHashMap(max(c.size(), 12))`, then `addAll(c)`.
    pub(crate) fn new_1_1(c: J<HashSet<E>>) -> J<HashSet<E>> {
        let items = c.items();
        let set = J::new(HashSet(tlars_java::JavaHashMap::for_mappings(items.len().max(12))));
        for e in items {
            set.add(e);
        }
        set
    }
}

impl<E: JValue> J<HashSet<E>> {
    fn items(&self) -> Vec<E> {
        self.jget().0.iter().map(|(e, _)| e.clone()).collect()
    }
    pub(crate) fn add(&self, e: E) -> bool {
        let h = e.java_hash();
        self.jget_mut().0.put(e, h, (), |a, b| a.java_equals(b)).is_none()
    }
    /// `AbstractCollection.addAll`: `add` each element in `c`'s iteration order.
    pub(crate) fn addAll(&self, c: J<HashSet<E>>) -> bool {
        let mut changed = false;
        for e in c.items() {
            changed |= self.add(e);
        }
        changed
    }
    pub(crate) fn remove(&self, e: E) -> bool {
        let h = e.java_hash();
        self.jget_mut().0.remove(&e, h, |a, b| a.java_equals(b)).is_some()
    }
    pub(crate) fn contains(&self, e: E) -> bool {
        let h = e.java_hash();
        self.jget().0.get(&e, h, |a, b| a.java_equals(b)).is_some()
    }
    pub(crate) fn size(&self) -> i32 {
        self.jget().0.len() as i32
    }
    pub(crate) fn isEmpty(&self) -> bool {
        self.jget().0.is_empty()
    }
    pub(crate) fn clear(&self) {
        self.jget_mut().0.clear();
    }
    /// `retainAll(c)`: removes, in iteration order, what `c` does not contain.
    pub(crate) fn retainAll(&self, c: J<HashSet<E>>) -> bool {
        let mut changed = false;
        for e in self.items() {
            if !c.contains(e.clone()) {
                self.remove(e);
                changed = true;
            }
        }
        changed
    }
    pub(crate) fn iterator(&self) -> J<Iterator<E>> {
        Iterator::over(self.items())
    }
}

/// `AbstractCollection.toString`: `[a, b]`.
impl<E: JValue> JToString for J<HashSet<E>> {
    fn java_string(&self) -> String {
        if self.is_null() {
            return "null".into();
        }
        let items: Vec<String> = self.items().iter().map(JValue::object_string).collect();
        format!("[{}]", items.join(", "))
    }
}

/// `java.util.ArrayList` (and `List`/`Collection` views, as snapshots).
pub(crate) struct ArrayList<E>(pub(crate) Vec<E>);

impl<E: Clone> ArrayList<E> {
    /// A list holding `items` (`List.subList` copies and the like).
    pub(crate) fn from_vec(items: Vec<E>) -> J<ArrayList<E>> {
        J::new(ArrayList(items))
    }
    pub(crate) fn new_0() -> J<ArrayList<E>> {
        J::new(ArrayList(Vec::new()))
    }
    pub(crate) fn new_1(c: J<ArrayList<E>>) -> J<ArrayList<E>> {
        J::new(ArrayList(c.jget().0.clone()))
    }
}

impl<E: Clone> J<ArrayList<E>> {
    pub(crate) fn add(&self, e: E) -> bool {
        self.jget_mut().0.push(e);
        true
    }
    #[track_caller]
    pub(crate) fn get(&self, i: i32) -> E {
        self.jget().0[index(i)].clone()
    }
    #[track_caller]
    pub(crate) fn set(&self, i: i32, e: E) -> E {
        std::mem::replace(&mut self.jget_mut().0[index(i)], e)
    }
    pub(crate) fn size(&self) -> i32 {
        self.jget().0.len() as i32
    }
    pub(crate) fn isEmpty(&self) -> bool {
        self.jget().0.is_empty()
    }
    pub(crate) fn iterator(&self) -> J<Iterator<E>> {
        Iterator::over(self.jget().0.clone())
    }
}

impl<E: JValue> J<ArrayList<E>> {
    /// `contains` compares with `equals`.
    pub(crate) fn contains(&self, e: E) -> bool {
        self.jget().0.iter().any(|x| e.java_equals(x))
    }
}

/// `java.util.Deque` / `ArrayDeque`, head at the front.
pub(crate) struct Deque<E>(std::collections::VecDeque<E>);

impl<E> Deque<E> {
    pub(crate) fn new_0() -> J<Deque<E>> {
        J::new(Deque(std::collections::VecDeque::new()))
    }
    /// The capacity carries no semantics.
    pub(crate) fn new_1_0(_capacity: i32) -> J<Deque<E>> {
        Self::new_0()
    }
}

impl<E: Clone> Deque<E> {
    /// `new ArrayDeque<>(c)`: a copy in `c`'s order.
    pub(crate) fn new_1_1(c: J<Deque<E>>) -> J<Deque<E>> {
        J::new(Deque(c.jget().0.clone()))
    }
}

impl<E: JValue> J<Deque<E>> {
    pub(crate) fn addFirst(&self, e: E) {
        self.jget_mut().0.push_front(e);
    }
    pub(crate) fn addLast(&self, e: E) {
        self.jget_mut().0.push_back(e);
    }
    pub(crate) fn push(&self, e: E) {
        self.addFirst(e);
    }
    #[track_caller]
    pub(crate) fn pop(&self) -> E {
        self.removeFirst()
    }
    #[track_caller]
    pub(crate) fn removeFirst(&self) -> E {
        self.jget_mut().0.pop_front().expect("java.util.NoSuchElementException")
    }
    pub(crate) fn peekFirst(&self) -> E {
        self.jget().0.front().cloned().unwrap_or_else(E::null)
    }
    pub(crate) fn peekLast(&self) -> E {
        self.jget().0.back().cloned().unwrap_or_else(E::null)
    }
    /// `contains` compares with `equals`.
    pub(crate) fn contains(&self, e: E) -> bool {
        self.jget().0.iter().any(|x| x.java_equals(&e))
    }
    pub(crate) fn size(&self) -> i32 {
        self.jget().0.len() as i32
    }
    pub(crate) fn isEmpty(&self) -> bool {
        self.jget().0.is_empty()
    }
    /// Head to tail.
    pub(crate) fn iterator(&self) -> J<Iterator<E>> {
        Iterator::over(self.jget().0.iter().cloned().collect())
    }
}

// ---- java.io ----

/// `java.io.Reader` over the decoded source, as `InputStreamReader` would deliver it.
pub(crate) struct Reader {
    units: Vec<u16>,
    pos: usize,
}

impl Reader {
    /// `new InputStreamReader(new ByteArrayInputStream(bytes), UTF_8)`: malformed input
    /// becomes U+FFFD.
    pub(crate) fn utf8(bytes: &[u8]) -> J<Reader> {
        J::new(Reader { units: String::from_utf8_lossy(bytes).encode_utf16().collect(), pos: 0 })
    }
}

impl J<Reader> {
    /// `read(cbuf, off, len)`: up to `len` units, `-1` at end of input.
    pub(crate) fn read(&self, buf: JArr<u16>, off: i32, len: i32) -> JResult<i32> {
        let mut r = self.jget_mut();
        if len == 0 {
            return Ok(0);
        }
        if r.pos >= r.units.len() {
            return Ok(-1);
        }
        let n = index(len).min(r.units.len() - r.pos);
        let off = index(off);
        buf.jget_mut()[off..off + n].copy_from_slice(&r.units[r.pos..r.pos + n]);
        r.pos += n;
        Ok(n as i32)
    }
    pub(crate) fn close(&self) -> JResult<()> {
        Ok(())
    }
}

// ---- util / tla2sany ----

/// `util.UniqueString`: an interned string, `Copy`, compared by token.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct UniqueString(Option<Atom>);

impl UniqueString {
    pub(crate) fn uniqueStringOf(s: JStr) -> UniqueString {
        UniqueString(Some(interner().intern(s.as_str())))
    }
    #[track_caller]
    fn atom(self) -> Atom {
        self.0.expect("NullPointerException")
    }
    /// The interned string, or `None` for a null `UniqueString`.
    pub(crate) fn as_atom(self) -> Option<Atom> {
        self.0
    }
    pub(crate) fn is_null(&self) -> bool {
        self.0.is_none()
    }
    pub(crate) fn same(&self, other: &UniqueString) -> bool {
        self == other
    }
    pub(crate) fn toString(&self) -> JStr {
        JStr::of(&interner().text(self.atom()))
    }
    /// `equals(UniqueString)` compares tokens.
    pub(crate) fn equals(&self, other: UniqueString) -> bool {
        other.0.is_some() && *self == other
    }
    pub(crate) fn concat(&self, s: JStr) -> UniqueString {
        UniqueString::uniqueStringOf(concat(&self.toString(), &s))
    }
    pub(crate) fn of(s: JStr) -> UniqueString {
        UniqueString::uniqueStringOf(s)
    }
    /// `compareTo`: token order.
    pub(crate) fn compareTo(&self, other: UniqueString) -> i32 {
        self.atom().tok() - other.atom().tok()
    }
    pub(crate) fn hashCode(&self) -> i32 {
        self.java_hash()
    }
    pub(crate) fn length(&self) -> i32 {
        self.toString().length()
    }
    /// `join(delim, n, us...)`: the first `n` joined with `"!"` (the delimiter is ignored),
    /// interning each intermediate string as TLC does.
    pub(crate) fn join(_delim: JStr, n: i32, us: JArr<UniqueString>) -> UniqueString {
        let mut out = UniqueString::null();
        for i in 0..n {
            let u = us.at(i);
            out = if out.is_null() { u } else { out.concat(JStr::of("!")).concat(u.toString()) };
        }
        out
    }
}

impl JNull for UniqueString {
    fn null() -> Self {
        UniqueString(None)
    }
}

/// `UniqueString.hashCode` is its string's; `equals(Object)` is identity, which interning
/// makes token equality.
impl JValue for UniqueString {
    fn is_java_null(&self) -> bool {
        self.is_null()
    }
    fn object_string(&self) -> String {
        self.java_string()
    }
    fn java_hash(&self) -> i32 {
        self.0.map_or(0, |a| interner().java_hash(a))
    }
    fn java_equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl JToString for UniqueString {
    fn java_string(&self) -> String {
        match self.0 {
            Some(a) => interner().text(a).to_string(),
            None => "null".into(),
        }
    }
}

thread_local! {
    /// `Location.unknown`, interned by `Location`'s static initialiser.
    static LOCATION_UNKNOWN: RefCell<Option<UniqueString>> = const { RefCell::new(None) };
}

/// `tla2sany.st.Location`, the part the parser uses.
pub(crate) struct Location {
    name: UniqueString,
    b_line: i32,
    b_column: i32,
    e_line: i32,
    e_column: i32,
}

impl Location {
    /// Runs the class initialiser on first use: `unknown = uniqueStringOf("--unknown--")`.
    fn unknown() -> UniqueString {
        if let Some(u) = LOCATION_UNKNOWN.with(|u| *u.borrow()) {
            return u;
        }
        let u = UniqueString::uniqueStringOf(JStr::of("--unknown--"));
        LOCATION_UNKNOWN.with(|cell| *cell.borrow_mut() = Some(u));
        u
    }

    pub(crate) fn new_5(name: UniqueString, bl: i32, bc: i32, el: i32, ec: i32) -> J<Location> {
        Location::unknown();
        J::new(Location { name, b_line: bl, b_column: bc, e_line: el, e_column: ec })
    }

    /// `new Location(bl, bc, el, ec)`: no module name.
    pub(crate) fn new_4(bl: i32, bc: i32, el: i32, ec: i32) -> J<Location> {
        Location::new_5(UniqueString::null(), bl, bc, el, ec)
    }

    /// `Location.nullLoc`, one object per session (`toString` compares with it).
    pub(crate) fn nullLoc() -> J<Location> {
        if let Some(l) = LOCATION_NULL.with(|l| l.borrow().clone()) {
            return l;
        }
        let l = Location::new_5(Location::unknown(), 0, 0, 0, 0);
        LOCATION_NULL.with(|cell| *cell.borrow_mut() = Some(l.clone()));
        l
    }

    /// `Location.moduleLocation(name)`: the whole module.
    pub(crate) fn moduleLocation(name: JStr) -> J<Location> {
        Location::new_5(UniqueString::uniqueStringOf(name), 0, 0, 0, 0)
    }
}

thread_local! {
    static LOCATION_NULL: RefCell<Option<J<Location>>> = const { RefCell::new(None) };
}

/// `Location.equals`/`hashCode`: by position and source name.
impl JValue for J<Location> {
    fn is_java_null(&self) -> bool {
        self.is_null()
    }
    fn object_string(&self) -> String {
        self.toString().java_string()
    }
    fn java_hash(&self) -> i32 {
        if self.is_null() {
            return 0;
        }
        let l = self.jget();
        let mut r: i32 = 1;
        for v in [l.b_column, l.b_line, l.e_column, l.e_line, l.name.java_hash()] {
            r = r.wrapping_mul(31).wrapping_add(v);
        }
        r
    }
    fn java_equals(&self, other: &Self) -> bool {
        self.equals(other.clone())
    }
}

impl J<Location> {
    pub(crate) fn toString(&self) -> JStr {
        if self.same(&Location::nullLoc()) {
            return JStr::of("Unknown location");
        }
        let l = self.jget();
        let span = format!("line {}, col {} to line {}, col {} of module ", l.b_line, l.b_column, l.e_line, l.e_column);
        if l.name.is_null() {
            return JStr::of(&(span + "null"));
        }
        if !l.name.equals(Location::unknown()) && l.b_column == 0 && l.e_column == 0 && l.b_line == 0 && l.e_line == 0 {
            return JStr::of(&format!("In module {}", l.name.java_string()));
        }
        JStr::of(&(span + &l.name.java_string()))
    }
    pub(crate) fn beginLine(&self) -> i32 {
        self.jget().b_line
    }
    pub(crate) fn beginColumn(&self) -> i32 {
        self.jget().b_column
    }
    pub(crate) fn endLine(&self) -> i32 {
        self.jget().e_line
    }
    pub(crate) fn endColumn(&self) -> i32 {
        self.jget().e_column
    }
    pub(crate) fn source(&self) -> JStr {
        let n = self.jget().name;
        if n.is_null() { JStr::null() } else { n.toString() }
    }
    pub(crate) fn sourceAsUniqueString(&self) -> UniqueString {
        self.jget().name
    }
    /// `equals(Object)`: same positions and `source().equals(other.source())`.
    pub(crate) fn equals(&self, other: J<Location>) -> bool {
        if other.is_null() {
            return false;
        }
        let (a, b) = (self.jget(), other.jget());
        a.b_line == b.b_line && a.b_column == b.b_column && a.e_line == b.e_line && a.e_column == b.e_column && {
            drop((a, b));
            self.source().equals(&other.source())
        }
    }
}

impl JToString for J<Location> {
    fn java_string(&self) -> String {
        if self.is_null() { "null".into() } else { self.toString().java_string() }
    }
}

/// `tla2sany.output.LogLevel`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum LogLevel {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    pub(crate) fn TRACE() -> LogLevel {
        LogLevel::Trace
    }
    pub(crate) fn DEBUG() -> LogLevel {
        LogLevel::Debug
    }
    pub(crate) fn INFO() -> LogLevel {
        LogLevel::Info
    }
    pub(crate) fn WARNING() -> LogLevel {
        LogLevel::Warning
    }
    pub(crate) fn ERROR() -> LogLevel {
        LogLevel::Error
    }
}

/// `tla2sany.output.SanyOutput`: records what the parser logs; the caller prints it at the
/// levels its real output (TLC's `SimpleSanyOutput`) would. `new SilentSanyOutput()` is a
/// recorder nobody reads.
#[derive(Default)]
pub(crate) struct SanyOutput {
    pub(crate) messages: Vec<(LogLevel, String)>,
}

impl SanyOutput {
    pub(crate) fn new_0() -> J<SanyOutput> {
        J::new(SanyOutput::default())
    }
}

impl J<SanyOutput> {
    /// Without arguments the format is the message (`SimpleSanyOutput` skips
    /// `String.format`).
    pub(crate) fn log_2(&self, level: LogLevel, format: JStr) {
        self.record(level, format.as_str().to_string());
    }
    pub(crate) fn log_3(&self, level: LogLevel, format: JStr, arg: &dyn JToString) {
        self.record(level, format_str(format.as_str(), &[arg]));
    }
    pub(crate) fn log_4(&self, level: LogLevel, format: JStr, a: &dyn JToString, b: &dyn JToString) {
        self.record(level, format_str(format.as_str(), &[a, b]));
    }
    /// Keeps the message, and prints it as TLC's `SimpleSanyOutput(ToolIO.out, INFO)` does.
    fn record(&self, level: LogLevel, message: String) {
        if !matches!(level, LogLevel::Trace | LogLevel::Debug) {
            super::load::console(super::load::ConsoleLine::Log(message.clone()));
        }
        self.jget_mut().messages.push((level, message));
    }
}

/// `String.format(format, args...)`.
#[track_caller]
pub(crate) fn java_format(format: &JStr, args: &[&dyn JToString]) -> JStr {
    JStr::of(&format_str(format.as_str(), args))
}

/// `String.format(format, objects)` with an `Object[]`.
#[track_caller]
pub(crate) fn java_format_objects(format: &JStr, objects: &JArr<J<Object>>) -> JStr {
    let items: Vec<J<Object>> = objects.jget().clone();
    let args: Vec<&dyn JToString> = items.iter().map(|o| o as &dyn JToString).collect();
    JStr::of(&format_str(format.as_str(), &args))
}

/// The conversions SANY's messages use: `%s`, `%d` (printed alike), `%N$s`, `%n`, `%%`.
#[track_caller]
fn format_str(format: &str, args: &[&dyn JToString]) -> String {
    let mut out = String::new();
    // Ordinary conversions take the arguments in turn; `%N$s` names one and does not move
    // that cursor.
    let mut next = 0;
    let mut chars = format.chars().peekable();
    let arg = |i: usize| args.get(i).expect("java.util.MissingFormatArgumentException").java_string();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut digits = String::new();
        while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
            digits.push(*d);
            chars.next();
        }
        let explicit = if digits.is_empty() {
            None
        } else {
            assert_eq!(chars.next(), Some('$'), "unsupported format width in %{digits}");
            Some(digits.parse::<usize>().expect("index") - 1)
        };
        match chars.next() {
            Some('s' | 'd') => match explicit {
                Some(i) => out.push_str(&arg(i)),
                None => {
                    out.push_str(&arg(next));
                    next += 1;
                }
            },
            Some('%') if explicit.is_none() => out.push('%'),
            Some('n') if explicit.is_none() => out.push('\n'),
            other => panic!("unsupported format conversion %{other:?}"),
        }
    }
    out
}

pub(crate) struct Assert;

impl Assert {
    /// `Assert.check(condition, errorCode)`: TLC's internal sanity checks.
    #[track_caller]
    pub(crate) fn check(condition: bool, error_code: i32) {
        assert!(condition, "tlc2.tool.TLCRuntimeException: EC {error_code}");
    }
}

/// `tlc2.output.EC`, the codes the parser checks with.
pub(crate) struct EC;

impl EC {
    pub(crate) const TLC_PARAMETER_MUST_BE_POSTFIX: i32 = 2136;
    pub(crate) const SANY_PARSER_CHECK_1: i32 = 4000;
    pub(crate) const SANY_PARSER_CHECK_2: i32 = 4001;
    pub(crate) const SANY_PARSER_CHECK_3: i32 = 4002;
}

/// `util.TLAConstants.KeyWords`.
pub(crate) struct KeyWords;

impl KeyWords {
    pub(crate) fn CONSTANT() -> JStr {
        JStr::of("CONSTANT")
    }
    pub(crate) fn CONSTANTS() -> JStr {
        JStr::of("CONSTANTS")
    }
    pub(crate) fn EXTENDS() -> JStr {
        JStr::of("EXTENDS")
    }
    pub(crate) fn MODULE() -> JStr {
        JStr::of("MODULE")
    }
    pub(crate) fn SPECIFICATION() -> JStr {
        JStr::of("SPECIFICATION")
    }
    pub(crate) fn VARIABLE() -> JStr {
        JStr::of("VARIABLE")
    }
}

// ---- tla2sany.parser.JunctionListContext (enum + inner class, ported by hand) ----

/// Bulleted `/\` and `\/` lists the parser is inside, innermost first.
pub(crate) struct JunctionListContext {
    stack: std::collections::VecDeque<(JunctionListType, i32)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JunctionListType {
    Conjunction,
    Disjunction,
}

impl JunctionListContext {
    pub(crate) fn new_0() -> J<JunctionListContext> {
        J::new(JunctionListContext { stack: std::collections::VecDeque::new() })
    }

    fn is_junction_list_bullet_token(kind: i32) -> bool {
        kind == TLAplusParserConstants::AND || kind == TLAplusParserConstants::OR
    }

    #[track_caller]
    fn as_junction_list_type(kind: i32) -> JunctionListType {
        match kind {
            TLAplusParserConstants::AND => JunctionListType::Conjunction,
            TLAplusParserConstants::OR => JunctionListType::Disjunction,
            _ => panic!("java.lang.IllegalArgumentException: {kind}"),
        }
    }
}

impl J<JunctionListContext> {
    pub(crate) fn startNewJunctionList(&self, column: i32, kind: i32) {
        let t = JunctionListContext::as_junction_list_type(kind);
        self.jget_mut().stack.push_front((t, column));
    }
    #[track_caller]
    pub(crate) fn terminateCurrentJunctionList(&self) {
        self.jget_mut().stack.pop_front().expect("java.util.NoSuchElementException");
    }
    pub(crate) fn isNewBullet(&self, column: i32, kind: i32) -> bool {
        let head = self.jget().stack.front().copied();
        // Java's `&&` evaluates `asJunctionListType` only for bullet tokens.
        head.is_some_and(|(t, c)| {
            JunctionListContext::is_junction_list_bullet_token(kind) && c == column && t == JunctionListContext::as_junction_list_type(kind)
        })
    }
    pub(crate) fn isAboveCurrent(&self, column: i32) -> bool {
        self.jget().stack.front().is_none_or(|(_, c)| *c < column)
    }
}

// ---- hand-written members of translated classes ----

impl J<SimpleCharStream> {
    /// `ExpandBuff(boolean)`: its `catch (Throwable)` only rethrows `System.arraycopy`'s
    /// runtime exceptions as an `Error`, i.e. aborts, as a panic here does.
    pub(crate) fn ExpandBuff(&self, wrap_around: bool) -> JResult<()> {
        let (bufsize, token_begin, bufpos) = self.jread(|s| (s.bufsize, s.tokenBegin, s.bufpos));
        let grown = bufsize + 2048;
        let newbuffer = JArr::new_array(grown, 0u16);
        let newbufline = JArr::new_array(grown, 0i32);
        let newbufcolumn = JArr::new_array(grown, 0i32);
        let (buffer, bufline, bufcolumn) = self.jread(|s| (s.buffer.clone(), s.bufline.clone(), s.bufcolumn.clone()));
        System::arraycopy(&buffer, token_begin, &newbuffer, 0, bufsize - token_begin);
        System::arraycopy(&bufline, token_begin, &newbufline, 0, bufsize - token_begin);
        System::arraycopy(&bufcolumn, token_begin, &newbufcolumn, 0, bufsize - token_begin);
        let next = if wrap_around {
            System::arraycopy(&buffer, 0, &newbuffer, bufsize - token_begin, bufpos);
            System::arraycopy(&bufline, 0, &newbufline, bufsize - token_begin, bufpos);
            System::arraycopy(&bufcolumn, 0, &newbufcolumn, bufsize - token_begin, bufpos);
            bufpos + (bufsize - token_begin)
        } else {
            bufpos - token_begin
        };
        let mut s = self.jget_mut();
        s.buffer = newbuffer;
        s.bufline = newbufline;
        s.bufcolumn = newbufcolumn;
        s.bufpos = next;
        s.maxNextCharInd = next;
        s.bufsize = grown;
        s.available = grown;
        s.tokenBegin = 0;
        Ok(())
    }
}

impl TLAplusParser {
    /// `new TLAplusParser(SanyOutput out, byte[] sourceInput)`: decodes the source as UTF-8
    /// and parses through the `Reader` constructor, which builds the same lexer state.
    pub(crate) fn new_from_bytes(out: J<SanyOutput>, source: &[u8]) -> JResult<J<TLAplusParser>> {
        let p = TLAplusParser::new_1_0(Reader::utf8(source))?;
        p.jget_mut().out = out;
        Ok(p)
    }
}

// ---- tla2sany.semantic: hand-written classes and members ----

/// `tla2sany.semantic.ErrorCode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ErrorCode(crate::SanyErrorCode);

impl ErrorCode {
    /// `ErrorCode.NAME`.
    #[track_caller]
    pub(crate) fn of(name: &str) -> ErrorCode {
        ErrorCode(*crate::error_codes::SANY_ERROR_CODES.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no ErrorCode {name}")))
    }
    pub(crate) fn getSeverityLevel(&self) -> ErrorLevel {
        match self.0.level {
            crate::ErrorLevel::Warning => ErrorLevel::Warning,
            crate::ErrorLevel::Error => ErrorLevel::Error,
        }
    }
    pub(crate) fn getStandardValue(&self) -> i32 {
        self.0.value
    }
    /// An unknown value is `IllegalArgumentException`, which only SANY's command line catches.
    #[track_caller]
    pub(crate) fn fromStandardValue(value: i32) -> ErrorCode {
        ErrorCode(crate::SanyErrorCode::from_standard_value(value).unwrap_or_else(|| panic!("java.lang.IllegalArgumentException: {value}")))
    }
    pub(crate) fn same(&self, other: &ErrorCode) -> bool {
        self == other
    }
    pub(crate) fn is_null(&self) -> bool {
        false
    }
    pub(crate) fn code(&self) -> crate::SanyErrorCode {
        self.0
    }
}

impl JNull for ErrorCode {
    fn null() -> Self {
        ErrorCode::of("GENERAL")
    }
}

impl JValue for ErrorCode {
    fn is_java_null(&self) -> bool {
        false
    }
    fn object_string(&self) -> String {
        self.0.name.to_string()
    }
    fn java_hash(&self) -> i32 {
        self.0.value
    }
    fn java_equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl JToString for ErrorCode {
    fn java_string(&self) -> String {
        self.0.name.to_string()
    }
}

/// `ErrorCode.ErrorLevel`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ErrorLevel {
    Warning,
    Error,
}

impl ErrorLevel {
    #[track_caller]
    pub(crate) fn of(name: &str) -> ErrorLevel {
        match name {
            "WARNING" => ErrorLevel::Warning,
            "ERROR" => ErrorLevel::Error,
            _ => panic!("no ErrorLevel {name}"),
        }
    }
    pub(crate) fn same(&self, other: &ErrorLevel) -> bool {
        self == other
    }
    pub(crate) fn is_null(&self) -> bool {
        false
    }
}

impl JNull for ErrorLevel {
    fn null() -> Self {
        ErrorLevel::Error
    }
}

impl JValue for ErrorLevel {
    fn is_java_null(&self) -> bool {
        false
    }
    fn object_string(&self) -> String {
        format!("{self:?}").to_uppercase()
    }
    fn java_hash(&self) -> i32 {
        *self as i32
    }
    fn java_equals(&self, other: &Self) -> bool {
        self == other
    }
}

/// `SetOfLevelConstraints` (a `HashMap<SymbolNode, Integer>`): `put` keeps the minimum.
pub(crate) struct SetOfLevelConstraints(tlars_java::JavaHashMap<J<SemanticNodeObj>, i32>);

/// `SetOfArgLevelConstraints` (a `HashMap<ParamAndPosition, Integer>`): `put` keeps the
/// maximum.
pub(crate) struct SetOfArgLevelConstraints(tlars_java::JavaHashMap<J<ParamAndPosition>, i32>);

macro_rules! level_constraints {
    ($set:ident, $key:ty, $put:ident, $keep:ident, $absent:expr) => {
        impl $set {
            pub(crate) fn new_0() -> J<$set> {
                J::new($set(tlars_java::JavaHashMap::new()))
            }
            /// `new HashMap<>(m)`: pre-sized by `putMapEntries`, filled in `m`'s order with the
            /// plain `put` (the override does not run).
            pub(crate) fn new_1(m: J<$set>) -> J<$set> {
                let mut map = tlars_java::JavaHashMap::new();
                let entries: Vec<($key, i32)> = m.jget().0.iter().map(|(k, v)| (k.clone(), *v)).collect();
                map.reserve_for_put_all(entries.len());
                for (k, v) in entries {
                    let h = k.java_hash();
                    map.put(k, h, v, |a, b| a.java_equals(b));
                }
                J::new($set(map))
            }
        }

        impl J<$set> {
            fn lookup(&self, k: &$key) -> Option<i32> {
                self.jget().0.get(k, k.java_hash(), |a, b| a.java_equals(b)).copied()
            }
            /// The override: keep the stricter level. Returns the old value.
            pub(crate) fn $put(&self, k: $key, level: i32) -> JInteger {
                let old = self.lookup(&k);
                let kept = level.$keep(old.unwrap_or($absent));
                let h = k.java_hash();
                self.jget_mut().0.put(k, h, kept, |a, b| a.java_equals(b));
                JInteger(old)
            }
            pub(crate) fn get(&self, k: $key) -> JInteger {
                JInteger(self.lookup(&k))
            }
            pub(crate) fn containsKey(&self, k: $key) -> bool {
                self.lookup(&k).is_some()
            }
            pub(crate) fn remove(&self, k: $key) -> JInteger {
                let h = k.java_hash();
                JInteger(self.jget_mut().0.remove(&k, h, |a, b| a.java_equals(b)))
            }
            /// The override: `put` each entry of `s` in its key order.
            pub(crate) fn putAll(&self, s: J<$set>) {
                let entries: Vec<($key, i32)> = s.jget().0.iter().map(|(k, v)| (k.clone(), *v)).collect();
                for (k, v) in entries {
                    self.$put(k, v);
                }
            }
            /// `keySet()`, as a snapshot in iteration order.
            pub(crate) fn keySet(&self) -> J<ArrayList<$key>> {
                J::new(ArrayList(self.jget().0.iter().map(|(k, _)| k.clone()).collect()))
            }
        }
    };
}

level_constraints!(SetOfLevelConstraints, J<SemanticNodeObj>, put, min, LevelConstants::MaxLevel);
level_constraints!(SetOfArgLevelConstraints, J<ParamAndPosition>, put_2, max, LevelConstants::MinLevel);

impl JToString for J<SetOfLevelConstraints> {
    fn java_string(&self) -> String {
        if self.is_null() { "null".into() } else { self.toString().expect("toString").java_string() }
    }
}

impl JToString for J<SetOfArgLevelConstraints> {
    fn java_string(&self) -> String {
        if self.is_null() { "null".into() } else { self.toString().expect("toString").java_string() }
    }
}

impl J<SetOfLevelConstraints> {
    pub(crate) fn toString(&self) -> JResult<JStr> {
        let mut parts = Vec::new();
        for (k, v) in self.jget().0.iter() {
            parts.push(format!("{} -> {v}", k.getName_0_via_SymbolNode()?.java_string()));
        }
        Ok(JStr::of(&format!("{{ {}}}", parts.join(", "))))
    }
}

impl J<SetOfArgLevelConstraints> {
    pub(crate) fn put_3(&self, param: J<SemanticNodeObj>, position: i32, level: i32) -> JResult<JInteger> {
        let pap = ParamAndPosition::new_2(param, position)?;
        Ok(self.put_2(pap, level))
    }
    pub(crate) fn toString(&self) -> JResult<JStr> {
        let mut parts = Vec::new();
        for (k, v) in self.jget().0.iter() {
            parts.push(format!("{} -> {v}", k.java_string()));
        }
        Ok(JStr::of(&format!("{{ {}}}", parts.join(", "))))
    }
}

impl J<SemanticNodeObj> {
    /// `isDefinedWith(SemanticNode)`: only TLC's `SpecProcessor` uses it.
    pub(crate) fn isDefinedWith__SemanticNode(&self, _sn: J<SemanticNodeObj>) -> JResult<bool> {
        todo!("TLC: SemanticNode.isDefinedWith")
    }
    /// `substituteFor(OpDefNode, OpDefNode)`: only TLC's `SpecProcessor` uses it.
    pub(crate) fn substituteFor__SemanticNode(&self, _s: J<SemanticNodeObj>, _o: J<SemanticNodeObj>) -> JResult<()> {
        todo!("TLC: SemanticNode.substituteFor")
    }
}

// ---- tla2sany.modanalyzer: hand-written members ----

impl J<SpecObj> {
    /// `SpecObj.validateParseUnit`: warns when a module's PlusCal algorithm or its TLA+
    /// translation changed since the translator checksummed them (`pcal.Validator`). TLC
    /// reads the file again; the stream's contents are the same bytes.
    pub(crate) fn validateParseUnit(&self, parse_unit: J<ParseUnit>, errors: J<Errors>) -> JResult<()> {
        use super::pcal_validator::ValidationResult::*;
        let bytes = parse_unit.getNis()?.bytes();
        let results = super::pcal_validator::validate(&parse_unit, &bytes)?;
        let name = parse_unit.getName()?;
        let message = |code: &str, format: &str| -> JResult<()> {
            errors.addMessage(
                ErrorCode::of(code),
                Location::moduleLocation(name.clone()),
                JStr::of(format),
                J::new(vec![Object::of(name.clone())]),
            )?;
            Ok(())
        };
        if results.contains(&TlaDivergenceExists) && results.contains(&PcalDivergenceExists) {
            message(
                "PLUSCAL_ALGORITHM_AND_TRANSLATION_CHANGED_SINCE_LAST_TRANSLATION",
                "Both the PlusCal algorithm and its TLA+ translation in module %s have changed since the last translation.",
            )?;
        } else if results.contains(&TlaDivergenceExists) {
            message(
                "PLUSCAL_TRANSLATION_CHANGED_SINCE_LAST_TRANSLATION",
                "The TLA+ translation in module %s has changed since its last translation.",
            )?;
        } else if results.contains(&PcalDivergenceExists) {
            message(
                "PLUSCAL_ALGORITHM_CHANGED_SINCE_LAST_TRANSLATION",
                "The PlusCal algorithm in module %s has changed since its last translation.",
            )?;
        } else if results.contains(&ErrorEncountered) {
            message("INTERNAL_ERROR", "An unexpected problem was encountered attempting to validate the specification for %s")?;
        }
        Ok(())
    }
}

// ---- pcal and tla2tex: hand-written classes ----

/// Helpers for the stream pipelines and `String` constructors that `PATCHES` rewrites in
/// PlusCal's sources.
pub(crate) struct JavaStrings;

impl JavaStrings {
    /// `input.codePoints().toArray()`.
    pub(crate) fn codePoints(input: JStr) -> JArr<i32> {
        J::new(input.as_str().chars().map(|c| c as i32).collect())
    }
    /// `new String(int[] codePoints, int offset, int count)`.
    #[track_caller]
    pub(crate) fn fromCodePoints(code_points: JArr<i32>, offset: i32, count: i32) -> JStr {
        let v = code_points.jget();
        let (start, n) = (index(offset), index(count));
        let s: String = v[start..start + n]
            .iter()
            .map(|&c| u32::try_from(c).ok().and_then(char::from_u32).expect("IllegalArgumentException"))
            .collect();
        JStr::of(&s)
    }
    /// `Arrays.stream(array).anyMatch(s::equals)`.
    pub(crate) fn arrayContains(array: JArr<JStr>, s: JStr) -> bool {
        array.jget().iter().any(|x| s.equals(x))
    }
    /// `Pattern.compile("[a-zA-Z][a-zA-Z0-9_]*").matcher(s).matches()`.
    pub(crate) fn isLabelRoot(s: JStr) -> bool {
        let mut chars = s.as_str().chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }
}

/// `java.io.File`, as far as path names: `getName` and `getPath` (no file system access).
pub(crate) struct File {
    path: String,
}

impl File {
    /// `new File(pathname)`: duplicate and trailing separators are dropped.
    pub(crate) fn new_1(pathname: JStr) -> J<File> {
        let mut path = String::new();
        for c in pathname.as_str().chars() {
            if !(c == '/' && path.ends_with('/')) {
                path.push(c);
            }
        }
        if path.len() > 1 && path.ends_with('/') {
            path.pop();
        }
        J::new(File { path })
    }
}

impl J<File> {
    pub(crate) fn getPath(&self) -> JStr {
        JStr::of(&self.jget().path)
    }
    pub(crate) fn getName(&self) -> JStr {
        let f = self.jget();
        JStr::of(f.path.rsplit('/').next().unwrap_or(&f.path))
    }
}

/// `tla2tex.Token`'s kinds of tokens.
pub(crate) struct TexToken;

impl TexToken {
    pub(crate) const BUILTIN: i32 = 1;
    pub(crate) const NUMBER: i32 = 2;
    pub(crate) const STRING: i32 = 3;
    pub(crate) const IDENT: i32 = 4;
    pub(crate) const COMMENT: i32 = 5;
    pub(crate) const DASHES: i32 = 6;
    pub(crate) const END_MODULE: i32 = 7;
    pub(crate) const PROLOG: i32 = 8;
    pub(crate) const EPILOG: i32 = 9;
    pub(crate) const PF_STEP: i32 = 12;
    pub(crate) const PCAL_LABEL: i32 = 13;
}

/// `util.TLAConstants`.
pub(crate) struct TLAConstants;

impl TLAConstants {
    pub(crate) fn MODULE_CLOSING_REGEX() -> JStr {
        JStr::of("^[=]+$")
    }
    pub(crate) fn RECORD_ARROW() -> JStr {
        JStr::of(" |-> ")
    }
}

/// `util.TLAConstants.Files`.
pub(crate) struct Files;

impl Files {
    pub(crate) fn CONFIG_EXTENSION() -> JStr {
        JStr::of(".cfg")
    }
    pub(crate) fn TLA_EXTENSION() -> JStr {
        JStr::of(".tla")
    }
}

impl Debug {
    /// `tla2tex.Debug.now()`: `new Date().getTime()`.
    pub(crate) fn now() -> JResult<i64> {
        Ok(System::currentTimeMillis())
    }
}

// The PlusCal translator as a program (`pcal.trans` with a file to translate): not reached by
// validation, which is all tlars runs of PlusCal so far.
impl trans {
    pub(crate) fn OutputHelpMessage() -> JResult<bool> {
        todo!("pcal.trans -help: the help text resource")
    }
    pub(crate) fn WriteStringVectorToFile<L>(_v: J<L>, _file: JStr) -> JResult<()> {
        todo!("pcal.trans: writing the translation to a file")
    }
}

impl J<TLAtoPCalMapping> {
    /// `TLAtoPCalMapping.hashCode`: `Arrays.hashCode` of the mapping, whose elements are
    /// arrays and hash by identity. Never used as a key.
    pub(crate) fn hashCode(&self) -> JResult<i32> {
        todo!("TLAtoPCalMapping.hashCode hashes arrays by identity")
    }
}
