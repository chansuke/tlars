//! Hand-written classes the translated parser calls (signatures in `xtask`'s
//! `jj2rs/externs.rs`): the Java library subset, `UniqueString`, `Location`, `SanyOutput`,
//! `JunctionListContext`, and the translated members listed as hand-written there.

#![allow(non_snake_case)]

use super::generated::{self, SimpleCharStream, TLAplusParser, TLAplusParserConstants};
use super::java::*;
use crate::{Atom, Interner};
use std::cell::RefCell;
use std::sync::Arc;

// ---- session ----

thread_local! {
    static INTERNER: RefCell<Option<Arc<Interner>>> = const { RefCell::new(None) };
}

/// Makes `interner` the one `UniqueString` uses on this thread. Java statics belong to the
/// JVM, which is the session here: switching to another session's interner forgets them.
pub(crate) fn enter(interner: &Arc<Interner>) {
    let same = INTERNER.with(|i| i.borrow().as_ref().is_some_and(|cur| Arc::ptr_eq(cur, interner)));
    if !same {
        generated::reset_all_statics();
        LOCATION_UNKNOWN.with(|u| *u.borrow_mut() = None);
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
    pub(crate) fn parseInt(s: JStr) -> i32 {
        tlars_java::parse_int(s.as_str()).unwrap_or_else(|e| panic!("java.lang.NumberFormatException: {e:?}"))
    }

    /// `Integer.toString(i, radix)`: lowercase digits, `-` for negatives.
    pub(crate) fn toString(i: i32, radix: i32) -> JStr {
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

pub(crate) struct Character;

impl Character {
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

impl System {
    pub(crate) fn out() -> J<PrintStream> {
        J::new(PrintStream)
    }

    /// Copies as if through a temporary array, so overlapping ranges work.
    #[track_caller]
    pub(crate) fn arraycopy<T: Clone>(src: &JArr<T>, src_pos: i32, dest: &JArr<T>, dest_pos: i32, length: i32) {
        let (s, d, n) = (index(src_pos), index(dest_pos), index(length));
        let tmp: Vec<T> = src.jget()[s..s + n].to_vec();
        dest.jget_mut()[d..d + n].clone_from_slice(&tmp);
    }

    /// Only the property the parser reads; TLC runs with Linux's line separator.
    pub(crate) fn getProperty(key: JStr, default: JStr) -> JStr {
        match key.as_str() {
            "line.separator" => JStr::of("\n"),
            _ => default,
        }
    }
}

#[track_caller]
fn index(i: i32) -> usize {
    usize::try_from(i).expect("ArrayIndexOutOfBoundsException")
}

/// `System.out`: only debugging dumps in the translated code write to it.
pub(crate) struct PrintStream;

impl J<PrintStream> {
    pub(crate) fn print(&self, x: &dyn JToString) {
        print!("{}", x.java_string());
    }
    pub(crate) fn println_1(&self, x: &dyn JToString) {
        println!("{}", x.java_string());
    }
    pub(crate) fn println_0(&self) {
        println!();
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

/// Java's `==` on references and interned values, used where collections compare elements.
pub(crate) trait JSame {
    fn jsame(&self, other: &Self) -> bool;
}

impl<T> JSame for J<T> {
    fn jsame(&self, other: &Self) -> bool {
        self.same(other)
    }
}

impl JSame for UniqueString {
    fn jsame(&self, other: &Self) -> bool {
        self == other
    }
}

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
    /// A snapshot; the parser never mutates a vector while enumerating it.
    pub(crate) fn elements(&self) -> J<Enumeration<E>> {
        J::new(Enumeration { items: self.jget().0.clone(), next: 0 })
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

/// Keys of a `Hashtable`: Java's `hashCode` decides the enumeration order.
pub(crate) trait JKey: Clone + Eq {
    fn java_hash(&self) -> i32;
}

impl JKey for UniqueString {
    fn java_hash(&self) -> i32 {
        interner().java_hash(self.atom())
    }
}

pub(crate) struct Hashtable<K, V>(tlars_java::Hashtable<K, V>);

impl<K: JKey, V> Hashtable<K, V> {
    pub(crate) fn new_0() -> J<Hashtable<K, V>> {
        J::new(Hashtable(tlars_java::Hashtable::new()))
    }
}

impl<K: JKey, V: Clone + JNull> J<Hashtable<K, V>> {
    pub(crate) fn get(&self, k: K) -> V {
        let h = k.java_hash();
        self.jget().0.get(&k, h).cloned().unwrap_or_else(V::null)
    }
    pub(crate) fn put(&self, k: K, v: V) -> V {
        let h = k.java_hash();
        self.jget_mut().0.put(k, h, v).unwrap_or_else(V::null)
    }
    pub(crate) fn containsKey(&self, k: K) -> bool {
        let h = k.java_hash();
        self.jget().0.get(&k, h).is_some()
    }
    pub(crate) fn keys(&self) -> J<Enumeration<K>> {
        J::new(Enumeration { items: self.jget().0.elements().map(|(k, _)| k.clone()).collect(), next: 0 })
    }
}

/// `java.util.Deque` / `ArrayDeque`, front at index 0.
pub(crate) struct Deque<E>(std::collections::VecDeque<E>);

impl<E> Deque<E> {
    pub(crate) fn new_0() -> J<Deque<E>> {
        J::new(Deque(std::collections::VecDeque::new()))
    }
}

impl<E: Clone + JSame> J<Deque<E>> {
    pub(crate) fn addFirst(&self, e: E) {
        self.jget_mut().0.push_front(e);
    }
    #[track_caller]
    pub(crate) fn removeFirst(&self) -> E {
        self.jget_mut().0.pop_front().expect("java.util.NoSuchElementException")
    }
    pub(crate) fn contains(&self, e: E) -> bool {
        self.jget().0.iter().any(|x| x.jsame(&e))
    }
    pub(crate) fn size(&self) -> i32 {
        self.jget().0.len() as i32
    }
}

// ---- java.io ----

#[derive(Clone, Copy, Debug)]
pub(crate) struct IOException;

impl IOException {
    pub(crate) fn new_0() -> IOException {
        IOException
    }
}

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
}

impl JNull for UniqueString {
    fn null() -> Self {
        UniqueString(None)
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
}

impl J<Location> {
    /// The translated code never builds `nullLoc`, so its arm is not reachable here.
    pub(crate) fn toString(&self) -> JStr {
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
    pub(crate) fn log_2(&self, level: LogLevel, format: JStr) {
        self.jget_mut().messages.push((level, java_format(format.as_str(), &[])));
    }
    pub(crate) fn log_3(&self, level: LogLevel, format: JStr, arg: &dyn JToString) {
        self.jget_mut().messages.push((level, java_format(format.as_str(), &[arg])));
    }
}

/// `String.format` for the `%s` and `%%` conversions the parser's messages use.
#[track_caller]
fn java_format(format: &str, args: &[&dyn JToString]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push_str(&args.next().expect("java.util.MissingFormatArgumentException").java_string()),
            Some('%') => out.push('%'),
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
