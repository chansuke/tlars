//! The Java semantics that code translated by `xtask jj2rs` relies on: nullable shared
//! mutable references, strings, exceptions, and `int`/`long` arithmetic. Generated code
//! uses only this module and the support classes; it never touches Rust references directly.

use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
use std::rc::Rc;

/// A Java reference to an object of class `T`: `null` or shared, mutable, compared by
/// identity.
pub(crate) struct J<T: ?Sized>(Option<Rc<RefCell<T>>>);

impl<T: ?Sized> Clone for J<T> {
    fn clone(&self) -> Self {
        J(self.0.clone())
    }
}

impl<T> J<T> {
    pub(crate) fn new(v: T) -> Self {
        J(Some(Rc::new(RefCell::new(v))))
    }
}

impl<T: ?Sized> J<T> {
    pub(crate) fn is_null(&self) -> bool {
        self.0.is_none()
    }
    /// Borrow for reading. A `null` receiver is Java's `NullPointerException`. (The `j`
    /// prefix keeps these clear of translated method names such as `get` and `read`.)
    #[track_caller]
    pub(crate) fn jget(&self) -> Ref<'_, T> {
        self.0.as_ref().expect("NullPointerException").borrow()
    }
    /// Field read whose borrow ends with the read, so the value can be passed to code that
    /// mutates the same object.
    #[track_caller]
    pub(crate) fn jread<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.jget())
    }
    /// Field write.
    #[track_caller]
    pub(crate) fn jget_mut(&self) -> RefMut<'_, T> {
        self.0.as_ref().expect("NullPointerException").borrow_mut()
    }
    /// `==` on references.
    pub(crate) fn same(&self, other: &J<T>) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// A Java array `T[]` (a reference, so it can be `null` and is shared).
pub(crate) type JArr<T> = J<Vec<T>>;

impl<T: Clone> J<Vec<T>> {
    /// `new T[n]`, filled with `T`'s Java default.
    pub(crate) fn new_array(len: i32, default: T) -> Self {
        J::new(vec![default; usize::try_from(len).expect("NegativeArraySizeException")])
    }
    #[track_caller]
    pub(crate) fn at(&self, i: i32) -> T {
        self.jget()[idx(i)].clone()
    }
    #[track_caller]
    pub(crate) fn set_at(&self, i: i32, v: T) {
        self.jget_mut()[idx(i)] = v;
    }
    pub(crate) fn length(&self) -> i32 {
        self.jget().len() as i32
    }
}

#[track_caller]
fn idx(i: i32) -> usize {
    usize::try_from(i).expect("ArrayIndexOutOfBoundsException")
}

/// A Java `String`: immutable, nullable. Java strings are UTF-16; TLA+ sources and the
/// parser's strings are compared and sliced in UTF-16 units through these methods.
#[derive(Clone, Default)]
pub(crate) struct JStr(Option<Rc<str>>);

impl JStr {
    pub(crate) fn of(s: &str) -> Self {
        JStr(Some(Rc::from(s)))
    }
    pub(crate) fn is_null(&self) -> bool {
        self.0.is_none()
    }
    #[track_caller]
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_deref().expect("NullPointerException")
    }
    fn units(&self) -> Vec<u16> {
        self.as_str().encode_utf16().collect()
    }
    pub(crate) fn equals(&self, other: &JStr) -> bool {
        !other.is_null() && self.as_str() == other.as_str()
    }
    pub(crate) fn length(&self) -> i32 {
        self.units().len() as i32
    }
    #[track_caller]
    pub(crate) fn char_at(&self, i: i32) -> u16 {
        self.units()[idx(i)]
    }
    #[track_caller]
    pub(crate) fn substring(&self, begin: i32, end: i32) -> JStr {
        JStr::of(&String::from_utf16_lossy(&self.units()[idx(begin)..idx(end)]))
    }
    #[track_caller]
    pub(crate) fn substring_from(&self, begin: i32) -> JStr {
        JStr::of(&String::from_utf16_lossy(&self.units()[idx(begin)..]))
    }
    pub(crate) fn index_of_char(&self, c: u16) -> i32 {
        self.units().iter().position(|u| *u == c).map_or(-1, |p| p as i32)
    }
    pub(crate) fn index_of(&self, s: &JStr) -> i32 {
        let hay = self.units();
        let needle = s.units();
        (0..=hay.len().saturating_sub(needle.len()))
            .find(|&i| hay[i..].starts_with(&needle))
            .map_or(-1, |p| p as i32)
    }
    /// `String` `==`: identity. Equal literals are one object in Java (interned); here two
    /// `JStr::of` calls are not, so translated code must not depend on literal identity.
    pub(crate) fn same(&self, other: &JStr) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }
    pub(crate) fn last_index_of(&self, s: &JStr) -> i32 {
        let hay = self.units();
        let needle = s.units();
        if needle.len() > hay.len() {
            return -1;
        }
        (0..=hay.len() - needle.len()).rev().find(|&i| hay[i..].starts_with(&needle)).map_or(-1, |p| p as i32)
    }
    pub(crate) fn contains(&self, s: &JStr) -> bool {
        self.as_str().contains(s.as_str())
    }
    /// `String.trim()`: strips units `<= ' '` from both ends.
    pub(crate) fn trim(&self) -> JStr {
        let u = self.units();
        let start = u.iter().position(|c| *c > 0x20).unwrap_or(u.len());
        let end = u.iter().rposition(|c| *c > 0x20).map_or(start, |e| e + 1);
        JStr::of(&String::from_utf16_lossy(&u[start..end]))
    }
    /// `split("\\r?\\n")`: Java drops trailing empty strings, keeps leading ones.
    pub(crate) fn split_lines(&self) -> JArr<JStr> {
        let s = self.as_str();
        let mut parts: Vec<&str> = s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
        // The last piece never ended in `\n`, so a `\r` there was not part of a delimiter.
        if let Some(last) = s.rsplit('\n').next() {
            *parts.last_mut().expect("split yields one piece") = last;
        }
        if s.contains('\n') {
            while parts.last().is_some_and(|p| p.is_empty()) {
                parts.pop();
            }
        }
        J::new(parts.into_iter().map(JStr::of).collect())
    }
    /// `new String(char[] value, int offset, int count)`.
    #[track_caller]
    pub(crate) fn from_chars(chars: &JArr<u16>, offset: i32, count: i32) -> JStr {
        let v = chars.jget();
        JStr::of(&String::from_utf16_lossy(&v[idx(offset)..idx(offset) + idx(count)]))
    }
    pub(crate) fn starts_with(&self, s: &JStr) -> bool {
        self.as_str().starts_with(s.as_str())
    }
    pub(crate) fn ends_with(&self, s: &JStr) -> bool {
        self.as_str().ends_with(s.as_str())
    }
}

/// `String.valueOf(x)` and string concatenation (`"a" + x`).
pub(crate) trait JToString {
    fn java_string(&self) -> String;
}

impl JToString for JStr {
    fn java_string(&self) -> String {
        self.0.as_deref().unwrap_or("null").to_string()
    }
}
impl JToString for i32 {
    fn java_string(&self) -> String {
        self.to_string()
    }
}
impl JToString for i64 {
    fn java_string(&self) -> String {
        self.to_string()
    }
}
impl JToString for bool {
    fn java_string(&self) -> String {
        self.to_string()
    }
}
impl JToString for f64 {
    fn java_string(&self) -> String {
        tlars_java::double_to_string(*self)
    }
}
/// `char` (a UTF-16 unit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct JChar(pub(crate) u16);
impl JToString for JChar {
    fn java_string(&self) -> String {
        String::from_utf16_lossy(&[self.0])
    }
}

/// `a + b` where either side is a `String`.
pub(crate) fn concat(a: &dyn JToString, b: &dyn JToString) -> JStr {
    JStr::of(&(a.java_string() + &b.java_string()))
}

/// `java.lang.Object` holding a translated value: assigning to `Object` boxes, a cast back
/// downcasts (a wrong type is `ClassCastException`, a panic).
pub(crate) struct Object {
    value: Box<dyn Any>,
    to_string: fn(&dyn Any) -> String,
}

/// Values that can be stored as an `Object`.
pub(crate) trait Boxable: Clone + JNull + 'static {
    fn is_java_null(&self) -> bool;
    /// `toString()` as `String.format("%s", o)` would call it.
    fn object_string(&self) -> String;
}

impl Boxable for JStr {
    fn is_java_null(&self) -> bool {
        self.is_null()
    }
    fn object_string(&self) -> String {
        self.java_string()
    }
}

impl<U: 'static> Boxable for J<U> {
    fn is_java_null(&self) -> bool {
        self.is_null()
    }
    /// Translated `toString()` methods are fallible and not reachable from here; no output
    /// TLC shows depends on this form.
    fn object_string(&self) -> String {
        format!("{}@object", std::any::type_name::<U>())
    }
}

impl Object {
    pub(crate) fn of<T: Boxable>(v: T) -> J<Object> {
        if v.is_java_null() {
            return J::null();
        }
        fn string_of<T: Boxable>(a: &dyn Any) -> String {
            a.downcast_ref::<T>().expect("boxed type").object_string()
        }
        J::new(Object { value: Box::new(v), to_string: string_of::<T> })
    }
}

impl J<Object> {
    #[track_caller]
    pub(crate) fn cast<T: Boxable>(&self) -> T {
        if self.is_null() {
            return T::null();
        }
        self.jget().value.downcast_ref::<T>().cloned().expect("ClassCastException")
    }
}

impl JToString for J<Object> {
    fn java_string(&self) -> String {
        if self.is_null() {
            return "null".into();
        }
        let o = self.jget();
        (o.to_string)(&*o.value)
    }
}

/// Exceptions the translated code throws or catches. `Error`s and runtime exceptions are
/// Rust panics instead, as they abort the parse in TLC too.
pub(crate) enum JThrow {
    ParseException(J<super::generated::ParseException>),
    /// JavaCC's lookahead short-circuit; never escapes `jj_2_*`.
    LookaheadSuccess,
    TokenMgrError(J<super::generated::TokenMgrError>),
    /// `java.io.IOException` from `SimpleCharStream.readChar` at end of input.
    IoException,
}

impl fmt::Debug for JThrow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            JThrow::ParseException(_) => "ParseException",
            JThrow::LookaheadSuccess => "LookaheadSuccess",
            JThrow::TokenMgrError(_) => "TokenMgrError",
            JThrow::IoException => "IOException",
        })
    }
}

pub(crate) type JResult<T> = Result<T, JThrow>;

/// `null` for any reference type.
pub(crate) trait JNull {
    fn null() -> Self;
}
impl<T: ?Sized> JNull for J<T> {
    fn null() -> Self {
        J(None)
    }
}
impl JNull for JStr {
    fn null() -> Self {
        JStr(None)
    }
}

/// Java's `>>>` on `int` and `long`.
pub(crate) fn ushr_i32(x: i32, n: i32) -> i32 {
    ((x as u32) >> (n & 31)) as i32
}
pub(crate) fn ushr_i64(x: i64, n: i32) -> i64 {
    ((x as u64) >> (n & 63)) as i64
}
