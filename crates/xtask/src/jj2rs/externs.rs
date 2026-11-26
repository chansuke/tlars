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
    contains(E) -> boolean
    appendNoRepeats(Vector<E>)

class JavaVector<E>
    <init>()
    <init>(int)
    <init>(int, int)
    <init>(JavaVector<E>)
    add(E) -> boolean
    add(int, E)
    addAll(JavaVector<E>) -> boolean
    addElement(E)
    insertElementAt(E, int)
    elementAt(int) -> E
    get(int) -> E
    set(int, E) -> E
    setElementAt(E, int)
    firstElement() -> E
    lastElement() -> E
    remove(int) -> E
    removeElementAt(int)
    removeAllElements()
    contains(E) -> boolean
    indexOf(E) -> int
    size() -> int
    isEmpty() -> boolean
    clone() -> Object
    elements() -> Enumeration<E>
    iterator() -> Iterator<E>
    toString() -> String

class JavaStrings
    static codePoints(String) -> int[]
    static fromCodePoints(int[], int, int) -> String
    static arrayContains(String[], String) -> boolean
    static isLabelRoot(String) -> boolean

class File
    <init>(String)
    getName() -> String
    getPath() -> String

class TexToken
    const BUILTIN: int
    const NUMBER: int
    const STRING: int
    const IDENT: int
    const COMMENT: int
    const DASHES: int
    const END_MODULE: int
    const PROLOG: int
    const EPILOG: int
    const PF_STEP: int
    const PCAL_LABEL: int

class TLAConstants
    static MODULE_CLOSING_REGEX: String
    static RECORD_ARROW: String

class Files
    static CONFIG_EXTENSION: String
    static TLA_EXTENSION: String

class Enumeration<E>
    hasMoreElements() -> boolean
    nextElement() -> E

class Hashtable<K, V>
    <init>()
    <init>(int)
    get(K) -> V
    put(K, V) -> V
    remove(K) -> V
    containsKey(K) -> boolean
    size() -> int
    isEmpty() -> boolean
    keys() -> Enumeration<K>
    elements() -> Enumeration<V>
    values() -> ArrayList<V>

class HashMap<K, V>
    <init>()
    get(K) -> V
    put(K, V) -> V
    containsKey(K) -> boolean
    size() -> int

class HashSet<E>
    <init>()
    <init>(int)
    <init>(HashSet<E>)
    add(E) -> boolean
    addAll(HashSet<E>) -> boolean
    remove(E) -> boolean
    contains(E) -> boolean
    size() -> int
    isEmpty() -> boolean
    clear()
    retainAll(HashSet<E>) -> boolean
    iterator() -> Iterator<E>

class ArrayList<E>
    <init>()
    <init>(ArrayList<E>)
    add(E) -> boolean
    get(int) -> E
    set(int, E) -> E
    contains(E) -> boolean
    size() -> int
    isEmpty() -> boolean
    iterator() -> Iterator<E>

class Iterator<E>
    hasNext() -> boolean
    next() -> E

class Deque<E>
    <init>()
    <init>(int)
    <init>(Deque<E>)
    addFirst(E)
    addLast(E)
    push(E)
    pop() -> E
    removeFirst() -> E
    peekFirst() -> E
    peekLast() -> E
    contains(E) -> boolean
    size() -> int
    isEmpty() -> boolean
    iterator() -> Iterator<E>

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
    static parseInt(String, int) -> int throws
    static valueOf(int) -> Integer
    intValue() -> int
    static toString(int) -> String
    static toString(int, int) -> String

class Boolean
    static valueOf(boolean) -> boolean

class Arrays
    static equals(Object[], Object[]) -> boolean
    static copyOf(int[], int) -> int[]

class AtomicInteger
    <init>()
    getAndIncrement() -> int

class BigInteger
    <init>(String, int)
    toString() -> String

class BigDecimal
    <init>(String)
    toString() -> String

class Class
    isInstance(Object) -> boolean
    getName() -> String

class MP
    static getMessage(int) -> String

class BuiltInOPs
    static getOpCode(UniqueString) -> int

class Long
    static parseLong(String) -> long throws
    static toString(long) -> String

class Math
    static max(int, int) -> int
    static min(int, int) -> int
    static abs(int) -> int

class Character
    static isWhitespace(char) -> boolean
    static isLetter(char) -> boolean
    static isDigit(char) -> boolean
    static isLetterOrDigit(char) -> boolean
    static toString(int) -> String

class System
    static out: PrintStream
    static arraycopy(Any, int, Any, int, int)
    static getProperty(String) -> String
    static getProperty(String, String) -> String
    static setProperty(String, String) -> String
    static currentTimeMillis() -> long

class PrintStream
    print(Any)
    println(Any)
    println()

class Reader
    read(char[], int, int) -> int throws
    close() throws

class UniqueString
    static uniqueStringOf(String) -> UniqueString
    static of(String) -> UniqueString
    static join(String, int, UniqueString...) -> UniqueString
    toString() -> String
    equals(UniqueString) -> boolean
    concat(String) -> UniqueString
    compareTo(UniqueString) -> int
    hashCode() -> int
    length() -> int

class Location
    static nullLoc: Location
    <init>(UniqueString, int, int, int, int)
    <init>(int, int, int, int)
    static moduleLocation(String) -> Location
    toString() -> String
    beginLine() -> int
    beginColumn() -> int
    endLine() -> int
    endColumn() -> int
    source() -> String
    sourceAsUniqueString() -> UniqueString
    equals(Location) -> boolean

class SanyOutput
    <init>()
    log(LogLevel, String)
    log(LogLevel, String, Any)
    log(LogLevel, String, Any, Any)

class NamedInputStream
    getName() -> String
    getFileName() -> String
    getModuleName() -> String
    exists() -> boolean
    isLibraryModule() -> boolean
    absoluteResolvedPath() -> String
    originalFilePathSuffix() -> String

class FilenameToStream
    isStandardModule(String) -> boolean

class FileUtil
    static createNamedInputStream(String, FilenameToStream) -> NamedInputStream
    static createNamedInputStream(String, FilenameToStream, NamedInputStream) -> NamedInputStream
    static newParser(SanyOutput, NamedInputStream) -> TLAplusParser throws

class ToolIO
    static getDefaultResolver() -> FilenameToStream
    static out: PrintStream
    static err: PrintStream
    const SYSTEM: int
    static getMode() -> int

class LogLevel
    static ERROR: LogLevel
    static WARNING: LogLevel
    static INFO: LogLevel
    static DEBUG: LogLevel
    static TRACE: LogLevel

class Assert
    static check(boolean, int)

class EC
    const TLC_PARAMETER_MUST_BE_POSTFIX: int
    const SANY_PARSER_CHECK_1: int
    const SANY_PARSER_CHECK_2: int
    const SANY_PARSER_CHECK_3: int

class KeyWords
    static CONSTANT: String
    static CONSTANTS: String
    static EXTENDS: String
    static MODULE: String
    static SPECIFICATION: String
    static VARIABLE: String

enum ErrorCode

enum ErrorLevel
    static WARNING: ErrorLevel
    static ERROR: ErrorLevel

class ErrorCode
    getSeverityLevel() -> ErrorLevel
    getStandardValue() -> int
    static fromStandardValue(int) -> ErrorCode

class SetOfLevelConstraints
    <init>()
    <init>(SetOfLevelConstraints)
    put(SymbolNode, int) -> Integer
    get(SymbolNode) -> Integer
    containsKey(SymbolNode) -> boolean
    remove(SymbolNode) -> Integer
    putAll(SetOfLevelConstraints)
    keySet() -> ArrayList<SymbolNode>
    toString() -> String

class SetOfArgLevelConstraints
    <init>()
    <init>(SetOfArgLevelConstraints)
    put(ParamAndPosition, int) -> Integer
    put(SymbolNode, int, int) -> Integer
    get(ParamAndPosition) -> Integer
    containsKey(ParamAndPosition) -> boolean
    remove(ParamAndPosition) -> Integer
    putAll(SetOfArgLevelConstraints)
    keySet() -> ArrayList<ParamAndPosition>
    toString() -> String

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
        if let Some(name) = line.strip_prefix("enum ") {
            classes.entry(name.trim().into()).or_insert_with(|| ClassInfo { is_extern: true, ..ClassInfo::default() }).is_enum = true;
            current = Some(name.trim().into());
            continue;
        }
        if let Some(decl) = line.strip_prefix("class ") {
            let (name, params) = match decl.split_once('<') {
                Some((n, p)) => (n.trim(), p.trim_end_matches('>').split(',').map(|s| s.trim().to_string()).collect()),
                None => (decl.trim(), Vec::new()),
            };
            let info = classes.entry(name.into()).or_insert_with(|| ClassInfo { is_extern: true, ..ClassInfo::default() });
            info.type_params = params;
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
        let raw: Vec<&str> = params.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
        let varargs = raw.last().is_some_and(|p| p.ends_with("..."));
        let params: Vec<T> = raw
            .iter()
            .map(|p| match p.strip_suffix("...") {
                Some(elem) => T::Arr(Box::new(ty(elem))),
                None => ty(p),
            })
            .collect();
        let rest = rest.trim();
        let fallible = rest.ends_with("throws");
        let ret = match rest.trim_end_matches("throws").trim().strip_prefix("->") {
            Some(r) => ty(r.trim()),
            None => T::Void,
        };
        let is_ctor = name == "<init>";
        let returns_param = ret.mentions_param();
        let sig = MethodSig {
            params,
            ret,
            is_static: is_static || is_ctor,
            fallible,
            rust_name: String::new(),
            member: None,
            varargs,
            has_body: true,
            returns_param,
        };
        info.methods.entry(name.into()).or_default().push(sig);
    }
    // Library exceptions: values of the generated `JThrow`.
    for (c, _) in super::hier::EXTERN_THROWABLES {
        let sig = |ret: T, fallible| MethodSig {
            params: Vec::new(),
            ret,
            is_static: false,
            fallible,
            rust_name: String::new(),
            member: None,
            varargs: false,
            has_body: true,
            returns_param: false,
        };
        let mut info = ClassInfo { is_extern: true, ..ClassInfo::default() };
        info.methods.insert("getMessage".into(), vec![sig(T::Str, true)]);
        info.methods.insert("printStackTrace".into(), vec![sig(T::Void, false)]);
        classes.insert(c.to_string(), info);
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
