//! Signatures of the classes the translated code uses but that are not translated: the Java
//! library subset and SANY/TLC classes, implemented by hand in
//! `tlars-sany/src/parser/support.rs`. The emitter types calls into them from this table; a
//! call missing here is an emission error, never a guess.
//!
//! One class per `class` line, then its members:
//! `[static] name(T, ...) -> R [throws]`, `<init>(T, ...)`, `[static] field: T`,
//! `const NAME: T` (a Rust `const`). `Any` passes the argument by reference unchanged
//! (`&dyn JToString` or a generic array on the Rust side); `E`, `K`, `V` are the class's
//! type parameters.

use super::ast::Type;
use super::emit::{ClassInfo, Classes, MethodSig, T};

const TABLE: &str = r"
class Object

class Vector<E>
    <init>()
    <init>(int)
    addElement(E)
    elementAt(int) -> E
    setElementAt(E, int)
    removeElementAt(int)
    removeAllElements()
    size() -> int
    isEmpty() -> boolean
    elements() -> Enumeration<E>

class Enumeration<E>
    hasMoreElements() -> boolean
    nextElement() -> E

class Hashtable<K, V>
    <init>()
    get(K) -> V
    put(K, V) -> V
    containsKey(K) -> boolean
    keys() -> Enumeration<K>

class Deque<E>
    <init>()
    addFirst(E)
    removeFirst() -> E
    contains(E) -> boolean
    size() -> int

class StringBuffer
    <init>()
    <init>(String)
    <init>(int)
    append(Any) -> StringBuffer
    append(char[]) -> StringBuffer
    setLength(int)
    length() -> int
    toString() -> String

class Integer
    const MAX_VALUE: int
    const MIN_VALUE: int
    static parseInt(String) -> int
    static toString(int, int) -> String

class Character
    static isWhitespace(char) -> boolean

class System
    static out: PrintStream
    static arraycopy(Any, int, Any, int, int)
    static getProperty(String, String) -> String

class PrintStream
    print(Any)
    println(Any)
    println()

class IOException
    <init>()

class Reader
    read(char[], int, int) -> int throws
    close() throws

class UniqueString
    static uniqueStringOf(String) -> UniqueString
    toString() -> String
    equals(UniqueString) -> boolean
    concat(String) -> UniqueString

class Location
    <init>(UniqueString, int, int, int, int)
    toString() -> String
    beginLine() -> int

class SanyOutput
    <init>()
    log(LogLevel, String)
    log(LogLevel, String, Any)

class LogLevel
    static ERROR: LogLevel
    static WARNING: LogLevel
    static INFO: LogLevel
    static DEBUG: LogLevel
    static TRACE: LogLevel

class Assert
    static check(boolean, int)

class EC
    const SANY_PARSER_CHECK_1: int
    const SANY_PARSER_CHECK_2: int
    const SANY_PARSER_CHECK_3: int

class KeyWords
    static CONSTANT: String

class JunctionListContext
    <init>()
    startNewJunctionList(int, int)
    terminateCurrentJunctionList()
    isNewBullet(int, int) -> boolean
    isAboveCurrent(int) -> boolean
";

/// The extern classes and the `(class, const)` pairs that are Rust `const`s.
pub(crate) fn classes() -> (Classes, Vec<(String, String)>) {
    let mut classes = Classes::new();
    let mut consts = Vec::new();
    let mut current: Option<String> = None;
    for line in TABLE.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(decl) = line.strip_prefix("class ") {
            let (name, params) = match decl.split_once('<') {
                Some((n, p)) => (n.trim(), p.trim_end_matches('>').split(',').map(|s| s.trim().to_string()).collect()),
                None => (decl.trim(), Vec::new()),
            };
            classes.insert(name.into(), ClassInfo { type_params: params, is_extern: true, ..ClassInfo::default() });
            current = Some(name.into());
            continue;
        }
        let class = current.clone().expect("member before class");
        let info = classes.get_mut(&class).expect("class");
        let ty = |s: &str| T::of_with_params(&parse_type(s), &info.type_params);
        if let Some(c) = line.strip_prefix("const ") {
            let (name, t) = c.split_once(':').expect("const NAME: T");
            info.fields.insert(name.trim().into(), (ty(t.trim()), true));
            consts.push((class.clone(), name.trim().to_string()));
            continue;
        }
        let (is_static, rest) = match line.strip_prefix("static ") {
            Some(r) => (true, r),
            None => (false, line),
        };
        let Some((name, rest)) = rest.split_once('(') else {
            let (name, t) = rest.split_once(':').expect("field: T");
            info.fields.insert(name.trim().into(), (ty(t.trim()), is_static));
            continue;
        };
        let (params, rest) = rest.split_once(')').expect("params");
        let params: Vec<T> = params.split(',').map(str::trim).filter(|p| !p.is_empty()).map(ty).collect();
        let rest = rest.trim();
        let fallible = rest.ends_with("throws");
        let ret = match rest.trim_end_matches("throws").trim().strip_prefix("->") {
            Some(r) => ty(r.trim()),
            None => T::Void,
        };
        let is_ctor = name == "<init>";
        let sig = MethodSig { params, ret, is_static: is_static || is_ctor, fallible, rust_name: String::new(), member: None };
        info.methods.entry(name.into()).or_default().push(sig);
    }
    for info in classes.values_mut() {
        super::emit::name_overloads(info);
    }
    (classes, consts)
}

/// Parses a type written in the table (`Vector<E>`, `char[]`).
pub(crate) fn parse_type(s: &str) -> Type {
    let toks = super::lex::lex(s).expect("type");
    super::parse::Parser::new(&toks).type_only().expect("type")
}
