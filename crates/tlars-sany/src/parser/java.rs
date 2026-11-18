//! The Java semantics that code translated by `xtask jj2rs` relies on: nullable shared
//! mutable references, strings, exceptions, and `int`/`long` arithmetic. Generated code
//! uses only this module and the support classes; it never touches Rust references directly.

use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
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
    /// `test(object)`, or `false` for `null` (`instanceof`).
    pub(crate) fn jtest(&self, test: impl FnOnce(&T) -> bool) -> bool {
        self.0.as_ref().is_some_and(|o| test(&o.borrow()))
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
    /// An array whose elements are built one by one (`new T[a][b]`: `a` distinct rows).
    pub(crate) fn new_array_with(len: i32, mut element: impl FnMut() -> T) -> Self {
        J::new((0..usize::try_from(len).expect("NegativeArraySizeException")).map(|_| element()).collect())
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
pub(crate) struct JStr(Option<Rc<StrRep>>);

/// A string's text, and whether it is ASCII (then UTF-16 units are its bytes, so
/// `length` and `charAt` need not re-encode it; SANY's explorer dumps call them per
/// character).
pub(crate) struct StrRep {
    text: Box<str>,
    ascii: bool,
}

impl JStr {
    pub(crate) fn of(s: &str) -> Self {
        JStr(Some(Rc::new(StrRep { text: Box::from(s), ascii: s.is_ascii() })))
    }
    pub(crate) fn is_null(&self) -> bool {
        self.0.is_none()
    }
    #[track_caller]
    fn rep(&self) -> &StrRep {
        self.0.as_deref().expect("NullPointerException")
    }
    #[track_caller]
    pub(crate) fn as_str(&self) -> &str {
        &self.rep().text
    }
    fn units(&self) -> Vec<u16> {
        self.as_str().encode_utf16().collect()
    }
    pub(crate) fn equals(&self, other: &JStr) -> bool {
        !other.is_null() && self.as_str() == other.as_str()
    }
    /// `String.equals(Object)`.
    pub(crate) fn equals_object(&self, other: &J<Object>) -> bool {
        other.instance_of::<JStr>(|s| self.equals(s))
    }
    pub(crate) fn length(&self) -> i32 {
        let r = self.rep();
        if r.ascii { r.text.len() as i32 } else { r.text.encode_utf16().count() as i32 }
    }
    #[track_caller]
    pub(crate) fn char_at(&self, i: i32) -> u16 {
        let r = self.rep();
        if r.ascii {
            return u16::from(r.text.as_bytes()[idx(i)]);
        }
        r.text.encode_utf16().nth(idx(i)).expect("StringIndexOutOfBoundsException")
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
    /// `indexOf(str, fromIndex)`: a negative start searches from 0, one past the end finds
    /// only the empty string.
    pub(crate) fn index_of_from(&self, s: &JStr, from: i32) -> i32 {
        let hay = self.units();
        let needle = s.units();
        let start = usize::try_from(from.max(0)).expect("non-negative");
        if start > hay.len() || needle.len() > hay.len() {
            return if needle.is_empty() && start <= hay.len() { start as i32 } else { -1 };
        }
        (start..=hay.len() - needle.len()).find(|&i| hay[i..].starts_with(&needle)).map_or(-1, |p| p as i32)
    }
    /// `startsWith(prefix, toffset)`: false for an offset outside the string.
    pub(crate) fn starts_with_at(&self, s: &JStr, offset: i32) -> bool {
        let hay = self.units();
        let needle = s.units();
        usize::try_from(offset).ok().is_some_and(|o| o <= hay.len() && hay[o..].starts_with(&needle))
    }
    /// `compareTo`: lexicographic by UTF-16 unit, then by length.
    pub(crate) fn compare_to(&self, s: &JStr) -> i32 {
        let (a, b) = (self.units(), s.units());
        for (x, y) in a.iter().zip(&b) {
            if x != y {
                return i32::from(*x) - i32::from(*y);
            }
        }
        a.len() as i32 - b.len() as i32
    }
    /// `split(regex)` for a regex that is an alternation of literal strings: at each position
    /// the first alternative that matches ends a piece; trailing empty pieces are dropped, and
    /// a string with no match is the only piece.
    pub(crate) fn split_literals(&self, alternatives: &[&str]) -> JArr<JStr> {
        let s = self.as_str();
        let mut pieces = Vec::new();
        let mut start = 0;
        let mut i = 0;
        let mut matched = false;
        while i < s.len() {
            match alternatives.iter().find(|a| s[i..].starts_with(**a)) {
                Some(a) => {
                    pieces.push(&s[start..i]);
                    i += a.len();
                    start = i;
                    matched = true;
                }
                None => i += s[i..].chars().next().map_or(1, char::len_utf8),
            }
        }
        if !matched {
            return J::new(vec![self.clone()]);
        }
        pieces.push(&s[start..]);
        while pieces.last().is_some_and(|p| p.is_empty()) {
            pieces.pop();
        }
        J::new(pieces.into_iter().map(JStr::of).collect())
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
    /// `toLowerCase()`; SANY lowercases ASCII numerals only, where every locale agrees.
    pub(crate) fn to_lower_case(&self) -> JStr {
        JStr::of(&self.as_str().to_lowercase())
    }
    /// `replace(CharSequence, CharSequence)`: literal, all occurrences.
    pub(crate) fn replace(&self, from: &JStr, to: &JStr) -> JStr {
        JStr::of(&self.as_str().replace(from.as_str(), to.as_str()))
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
        self.0.as_deref().map_or("null", |r| &r.text).to_string()
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

/// `java.lang.Integer`: a nullable boxed `int`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct JInteger(pub(crate) Option<i32>);

impl JInteger {
    pub(crate) fn of(v: i32) -> JInteger {
        JInteger(Some(v))
    }
    #[track_caller]
    pub(crate) fn intValue(&self) -> i32 {
        self.0.expect("NullPointerException: unboxing null")
    }
    pub(crate) fn is_null(&self) -> bool {
        self.0.is_none()
    }
    /// `==` on boxes: `Integer.valueOf` shares boxes for -128..=127; other values the
    /// sources never compare by identity.
    pub(crate) fn same(&self, other: &JInteger) -> bool {
        match (self.0, other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b && (-128..=127).contains(&a),
            _ => false,
        }
    }
}

impl JNull for JInteger {
    fn null() -> Self {
        JInteger(None)
    }
}

impl JValue for JInteger {
    fn java_class(&self) -> &'static str {
        "Integer"
    }
    fn is_java_null(&self) -> bool {
        self.is_null()
    }
    fn object_string(&self) -> String {
        self.java_string()
    }
    fn java_hash(&self) -> i32 {
        self.0.unwrap_or(0)
    }
    fn java_equals(&self, other: &Self) -> bool {
        self.0.is_some() && self.0 == other.0
    }
}

impl JToString for JInteger {
    fn java_string(&self) -> String {
        self.0.map_or_else(|| "null".into(), |v| v.to_string())
    }
}

/// `java.lang.Object` holding a translated value: assigning to `Object` boxes, a cast back
/// downcasts (a wrong type is `ClassCastException`, a panic). The box keeps the value's
/// `hashCode`, `equals` and `toString`, so `Object`-keyed tables work.
pub(crate) struct Object {
    value: Box<dyn Any>,
    to_string: fn(&dyn Any) -> String,
    hash: fn(&dyn Any) -> i32,
    equals: fn(&dyn Any, &J<Object>) -> bool,
    instance_of: fn(&dyn Any, &str) -> bool,
    class: fn(&dyn Any) -> &'static str,
}

/// What Java's `Object` methods do for a value: `hashCode`, `equals`, `toString` (as
/// `String.valueOf`). Translated classes get generated implementations that call their own
/// overrides; classes without overrides use identity.
pub(crate) trait JValue: Clone + JNull + 'static {
    fn is_java_null(&self) -> bool;
    fn object_string(&self) -> String;
    fn java_hash(&self) -> i32;
    fn java_equals(&self, other: &Self) -> bool;
    /// `Class.forName(class).isInstance(this)`, by simple class name.
    fn java_instance_of(&self, class: &str) -> bool {
        !self.is_java_null() && class == "Object"
    }
    /// The simple name of `getClass()`.
    fn java_class(&self) -> &'static str {
        "Object"
    }
}

impl JValue for JStr {
    fn java_class(&self) -> &'static str {
        "String"
    }
    fn is_java_null(&self) -> bool {
        self.is_null()
    }
    fn object_string(&self) -> String {
        self.java_string()
    }
    fn java_hash(&self) -> i32 {
        if self.is_null() { 0 } else { tlars_java::string_hash_code(self.as_str()) }
    }
    fn java_equals(&self, other: &Self) -> bool {
        self.equals(other)
    }
}

impl JValue for i32 {
    fn is_java_null(&self) -> bool {
        false
    }
    fn object_string(&self) -> String {
        self.to_string()
    }
    fn java_hash(&self) -> i32 {
        *self
    }
    fn java_equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl JValue for bool {
    fn is_java_null(&self) -> bool {
        false
    }
    fn object_string(&self) -> String {
        self.to_string()
    }
    /// `Boolean.hashCode`.
    fn java_hash(&self) -> i32 {
        if *self { 1231 } else { 1237 }
    }
    fn java_equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T: ?Sized> J<T> {
    /// `Object.hashCode` for classes that do not override it. Java's identity hash is
    /// arbitrary; tables keyed this way iterate in no reproducible order in TLC either.
    /// The object's address, unique while it lives: identity as a map key.
    pub(crate) fn ptr_key(&self) -> usize {
        self.0.as_ref().map_or(0, |rc| Rc::as_ptr(rc) as *const () as usize)
    }
    pub(crate) fn identity_hash(&self) -> i32 {
        self.0.as_ref().map_or(0, |rc| Rc::as_ptr(rc) as *const () as usize as i32)
    }
}

impl Object {
    pub(crate) fn of<T: JValue>(v: T) -> J<Object> {
        if v.is_java_null() {
            return J::null();
        }
        // An `Object` stays itself (no box in a box).
        if let Some(o) = (&v as &dyn Any).downcast_ref::<J<Object>>() {
            return o.clone();
        }
        fn string_of<T: JValue>(a: &dyn Any) -> String {
            a.downcast_ref::<T>().expect("boxed type").object_string()
        }
        fn hash_of<T: JValue>(a: &dyn Any) -> i32 {
            a.downcast_ref::<T>().expect("boxed type").java_hash()
        }
        fn equals_of<T: JValue>(a: &dyn Any, other: &J<Object>) -> bool {
            let a = a.downcast_ref::<T>().expect("boxed type");
            !other.is_null() && other.jget().value.downcast_ref::<T>().is_some_and(|b| a.java_equals(b))
        }
        fn instance_of<T: JValue>(a: &dyn Any, class: &str) -> bool {
            a.downcast_ref::<T>().expect("boxed type").java_instance_of(class)
        }
        fn class_of<T: JValue>(a: &dyn Any) -> &'static str {
            a.downcast_ref::<T>().expect("boxed type").java_class()
        }
        J::new(Object {
            value: Box::new(v),
            to_string: string_of::<T>,
            hash: hash_of::<T>,
            equals: equals_of::<T>,
            instance_of: instance_of::<T>,
            class: class_of::<T>,
        })
    }
}

impl J<Object> {
    #[track_caller]
    pub(crate) fn cast<T: JValue>(&self) -> T {
        if self.is_null() {
            return T::null();
        }
        if let Some(o) = (self as &dyn Any).downcast_ref::<T>() {
            return o.clone();
        }
        self.jget().value.downcast_ref::<T>().cloned().expect("ClassCastException")
    }

    /// `o instanceof C` on an `Object`: boxed as a `T` that passes `test`.
    pub(crate) fn instance_of<T: JValue>(&self, test: impl FnOnce(&T) -> bool) -> bool {
        !self.is_null() && self.jget().value.downcast_ref::<T>().is_some_and(test)
    }
}

impl JValue for J<Object> {
    fn is_java_null(&self) -> bool {
        self.is_null()
    }
    fn object_string(&self) -> String {
        self.java_string()
    }
    fn java_hash(&self) -> i32 {
        if self.is_null() {
            return 0;
        }
        let o = self.jget();
        (o.hash)(&*o.value)
    }
    fn java_equals(&self, other: &Self) -> bool {
        if self.is_null() {
            return other.is_null();
        }
        let o = self.jget();
        (o.equals)(&*o.value, other)
    }
    fn java_instance_of(&self, class: &str) -> bool {
        if self.is_null() {
            return false;
        }
        let o = self.jget();
        (o.instance_of)(&*o.value, class)
    }
    fn java_class(&self) -> &'static str {
        let o = self.jget();
        (o.class)(&*o.value)
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

/// A library exception (`IOException`, `NumberFormatException`, ...) in flight. Translated
/// exceptions travel in their own `JThrow` variants (generated).
#[derive(Clone)]
pub(crate) struct ExternThrowable {
    pub(crate) class: &'static str,
    pub(crate) message: JStr,
}

impl JThrow {
    pub(crate) fn new_extern(class: &'static str, message: JStr) -> JThrow {
        JThrow::Extern(ExternThrowable { class, message })
    }
    /// Debugging aid only; TLC's own stack traces are not reproduced.
    #[allow(non_snake_case)]
    pub(crate) fn printStackTrace(&self) {
        eprintln!("{self:?}");
    }
}

pub(crate) use super::generated::JThrow;

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
/// The primitive stand-ins for boxed `Integer`/`Boolean`/`Long` (see the emitter): what a
/// map lookup that misses returns.
impl JNull for i32 {
    fn null() -> Self {
        0
    }
}
impl JNull for i64 {
    fn null() -> Self {
        0
    }
}
impl JNull for bool {
    fn null() -> Self {
        false
    }
}

/// Java's `>>>` on `int` and `long`.
pub(crate) fn ushr_i32(x: i32, n: i32) -> i32 {
    ((x as u32) >> (n & 31)) as i32
}
pub(crate) fn ushr_i64(x: i64, n: i32) -> i64 {
    ((x as u64) >> (n & 63)) as i64
}
