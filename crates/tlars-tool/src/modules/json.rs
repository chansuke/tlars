//! `tlc2.module.Json`: `ToJson`, `ToJsonArray`, `ToJsonObject`, `JsonSerialize`,
//! `JsonDeserialize`, `ndJsonSerialize`, `ndJsonDeserialize`.
//!
//! The module's TLA+ bodies are decoys (`Json.tla` defines `JsonSerialize == TRUE`), so
//! every observable byte comes from the Java class and from gson, which [`node`] and
//! [`parse`] reproduce. The conversion is neither total nor a bijection: a set becomes an
//! array, a tuple an object or an array depending on which operator asked, and a value
//! class the chain of `instanceof` tests does not name raises an `IOException` that
//! `MethodValue.eval` reports as TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE.

mod node;
mod parse;

use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use crate::modules::eval_exception;
use crate::value::{FcnDomain, FcnRcdValue, IntervalValue, RecordValue, Value};
use node::Node;
use std::path::PathBuf;
use std::sync::Arc;
use tlars_msg::Ec;

/// An `IOException` the module raises on purpose. It is a checked exception, not an
/// `EvalException`, so `MethodValue.eval` wraps it in the message naming the Java method
/// (`MethodValue.java:147-169`); any kind that `modules::wrap` wraps will do.
fn io_exception(message: String) -> EvalError {
    EvalError::detailed(message, None)
}

/// `"Cannot convert value: unsupported value type " + value.getClass().getName()`.
fn unsupported(v: &Value) -> EvalError {
    io_exception(format!("Cannot convert value: unsupported value type tlc2.value.impl.{}", v.java_class()))
}

/// `new StringValue(node.toString())`.
fn json_string(ev: &mut Ev<'_>, node: &Node) -> Value {
    let text = node.to_json();
    Value::Str(ev.tool().graph().interner().intern(&text))
}

/// `ToJson(value)`.
pub(crate) fn to_json(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let node = get_node(ev, &a[0])?;
    Ok(json_string(ev, &node))
}

/// `ToJsonArray(value)`.
pub(crate) fn to_json_array(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let node = get_array_node(ev, &a[0])?;
    Ok(json_string(ev, &node))
}

/// `ToJsonObject(value)`.
pub(crate) fn to_json_object(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let node = get_object_node(ev, &a[0])?;
    Ok(json_string(ev, &node))
}

/// `getNode(IValue)` (`Json.java:263-287`). The order of the tests is observable: a tuple
/// is an array here, and an object only where `getObjectNode` asked for one.
fn get_node(ev: &mut Ev<'_>, v: &Value) -> EvalResult<Node> {
    match v {
        Value::Record(r) => object_node_of_record(ev, r),
        Value::Tuple(t) => array_node_of_tuple(ev, &t.elems),
        Value::Str(a) => Ok(Node::Str(ev.tool().graph().interner().text(*a).to_string())),
        Value::Model(mv) => Ok(Node::Str(ev.tool().graph().interner().text(mv.name).to_string())),
        Value::Int(i) => Ok(Node::int(*i)),
        Value::Bool(b) => Ok(Node::Bool(*b)),
        Value::FcnRcd(f) => object_node_of_fcn(ev, f),
        Value::FcnLambda(f) => {
            let fcn = f.to_fcn_rcd(ev)?;
            object_node_of_fcn(ev, &fcn)
        }
        // `SetEnumValue` and then every other `Enumerable`, which `toSetEnum()` turns into
        // one; both arrive at the same `getArrayNode(SetEnumValue)`.
        _ if v.is_enumerable() => array_node_of_set(ev, v),
        _ => Err(unsupported(v)),
    }
}

/// `getObjectNode(IValue)` (`Json.java:317-329`): a record, a tuple or a function only.
fn get_object_node(ev: &mut Ev<'_>, v: &Value) -> EvalResult<Node> {
    match v {
        Value::Record(r) => object_node_of_record(ev, r),
        Value::Tuple(t) => object_node_of_tuple(ev, &t.elems),
        Value::FcnRcd(f) => object_node_of_fcn(ev, f),
        Value::FcnLambda(f) => {
            let fcn = f.to_fcn_rcd(ev)?;
            object_node_of_fcn(ev, &fcn)
        }
        _ => Err(unsupported(v)),
    }
}

/// `getArrayNode(IValue)` (`Json.java:391-405`): a tuple, a function or an enumerable set.
fn get_array_node(ev: &mut Ev<'_>, v: &Value) -> EvalResult<Node> {
    match v {
        Value::Tuple(t) => array_node_of_tuple(ev, &t.elems),
        Value::FcnRcd(f) => array_node_of_fcn(ev, f),
        Value::FcnLambda(f) => {
            let fcn = f.to_fcn_rcd(ev)?;
            array_node_of_fcn(ev, &fcn)
        }
        _ if v.is_enumerable() => array_node_of_set(ev, v),
        _ => Err(unsupported(v)),
    }
}

/// `getObjectNode(RecordValue)` (`Json.java:362-369`): the fields of the *snapshot*, so an
/// unnormalised record keeps the order the spec wrote them in.
fn object_node_of_record(ev: &mut Ev<'_>, r: &Arc<RecordValue>) -> EvalResult<Node> {
    let (names, values) = r.snapshot();
    let (names, values) = (names.to_vec(), values.to_vec());
    let mut object = Node::Object(Vec::new());
    for (name, value) in names.iter().zip(&values) {
        let name = ev.tool().graph().interner().text(*name).to_string();
        let child = get_node(ev, value)?;
        object.add(name, child);
    }
    Ok(object)
}

/// `getObjectNode(TupleValue)` (`Json.java:377-383`): the indices as decimal keys, from 0.
fn object_node_of_tuple(ev: &mut Ev<'_>, elems: &[Value]) -> EvalResult<Node> {
    let mut object = Node::Object(Vec::new());
    for (i, e) in elems.iter().enumerate() {
        let child = get_node(ev, e)?;
        object.add(i.to_string(), child);
    }
    Ok(object)
}

/// `getArrayNode(TupleValue)` (`Json.java:413-419`).
fn array_node_of_tuple(ev: &mut Ev<'_>, elems: &[Value]) -> EvalResult<Node> {
    let mut items = Vec::with_capacity(elems.len());
    for e in elems {
        items.push(get_node(ev, e)?);
    }
    Ok(Node::Array(items))
}

/// `getArrayNode(SetEnumValue)` (`Json.java:446-454`): the elements in normalised order, so
/// the set comes out sorted and deduplicated, and a set TLC cannot order fails here.
fn array_node_of_set(ev: &mut Ev<'_>, v: &Value) -> EvalResult<Node> {
    let set = v.to_set_enum(ev)?.expect("an enumerable value converts");
    let elems = crate::value::order::normalized(ev, &set)?.to_vec();
    let mut items = Vec::with_capacity(elems.len());
    for e in &elems {
        items.push(get_node(ev, e)?);
    }
    Ok(Node::Array(items))
}

/// `getObjectNode(FcnRcdValue)` (`Json.java:337-354`): a function whose domain is `1..n` is
/// an array after all, and otherwise the keys are the domain elements as `Value.toString()`
/// prints them (not `Values.ppr`), which is why a set key reads `{1}`.
fn object_node_of_fcn(ev: &mut Ev<'_>, f: &Arc<FcnRcdValue>) -> EvalResult<Node> {
    let fcn = snapshot(f);
    if is_valid_sequence(ev, &fcn)? {
        return array_node_of_fcn(ev, &fcn);
    }
    let domain = domain_as_values(&fcn);
    let values = fcn.snapshot().1.to_vec();
    let mut object = Node::Object(Vec::new());
    for (d, value) in domain.iter().zip(&values) {
        let key = match d {
            Value::Str(a) => ev.tool().graph().interner().text(*a).to_string(),
            other => other.to_tlc_string(ev)?,
        };
        let child = get_node(ev, value)?;
        object.add(key, child);
    }
    Ok(object)
}

/// `getArrayNode(FcnRcdValue)` (`Json.java:427-438`): the values in domain order, or the
/// object form when the domain is not `1..n`.
fn array_node_of_fcn(ev: &mut Ev<'_>, f: &Arc<FcnRcdValue>) -> EvalResult<Node> {
    let fcn = snapshot(f);
    if !is_valid_sequence(ev, &fcn)? {
        return object_node_of_fcn(ev, &fcn);
    }
    let values = fcn.snapshot().1.to_vec();
    let mut items = Vec::with_capacity(values.len());
    for value in &values {
        items.push(get_node(ev, value)?);
    }
    Ok(Node::Array(items))
}

/// `FcnRcdValue.snapshot()` (`FcnRcdValue.java:680-693`): the value itself when it is
/// sorted, else a private copy. The copy matters, because `isValidSequence` normalises what
/// it is handed and TLC leaves the caller's own function unsorted.
fn snapshot(f: &Arc<FcnRcdValue>) -> Arc<FcnRcdValue> {
    if f.is_normalized() {
        return f.clone();
    }
    match &f.domain {
        FcnDomain::Explicit(domain) => Value::fcn_rcd_arc(domain.to_vec(), f.values.to_vec(), false),
        // An interval domain is sorted by construction, so `is_normalized` answered true.
        FcnDomain::Interval(_) => unreachable!("an unsorted function has an explicit domain"),
    }
}

/// `getDomainAsValues()` (`FcnRcdValue.java:590-596`): the `domain` array, which is the very
/// array `normalize()` sorts in place, or a fresh `intv.asValues()`.
fn domain_as_values(f: &FcnRcdValue) -> Vec<Value> {
    match (f.snapshot().0, f.interval()) {
        (Some(domain), _) => domain.to_vec(),
        (None, Some(iv)) => interval_as_values(iv),
        (None, None) => unreachable!("a function has an explicit domain or an interval"),
    }
}

/// `IntervalValue.asValues()`.
fn interval_as_values(iv: IntervalValue) -> Vec<Value> {
    (i64::from(iv.lo)..=i64::from(iv.hi)).map(|n| Value::Int(n as i32)).collect()
}

/// `isValidSequence(FcnRcdValue)` (`Json.java:295-309`): is the domain `1..n`?
///
/// Two accidents of the Java are observable. A domain holding anything but an `IntValue`
/// returns *before* `normalize()`, so such a function keeps its unsorted order
/// (`(1 :> "x") @@ (mv :> "y")` prints `1` first). And the array the second loop reads is
/// the one `normalize()` has just sorted, so a function written out of order still counts
/// as a sequence.
fn is_valid_sequence(ev: &mut Ev<'_>, f: &Arc<FcnRcdValue>) -> EvalResult<bool> {
    let domain = domain_as_values(f);
    if !domain.iter().all(|d| matches!(d, Value::Int(_))) {
        return Ok(false);
    }
    f.normalize(ev)?;
    let sorted = domain_as_values(f);
    Ok(sorted.iter().enumerate().all(|(i, d)| matches!(d, Value::Int(n) if i64::from(*n) == i as i64 + 1)))
}

/// `ndJsonSerialize(path, value)` (`Json.java:159-174`): one JSON value per line, each
/// followed by a newline, so the file ends with one.
pub(crate) fn nd_serialize(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let Some(tuple) = a[1].to_tuple(ev)? else {
        let shown = a[1].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "ndJsonSerialize", "sequence", &shown]));
    };
    let elems = tuple.elems.to_vec();
    let mut writer = open_for_writing(ev, &a[0])?;
    for e in &elems {
        let line = format!("{}\n", get_node(ev, e)?.to_json());
        writer.write(&line)?;
    }
    Ok(Value::Bool(true))
}

/// `JsonSerialize(path, value)` (`Json.java:238-255`). What is written is `getNode(v)` of
/// the argument itself, not of the tuple or record the check accepted, so a set nested in a
/// record is serialized all the same.
pub(crate) fn serialize(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let accepted = a[1].to_tuple(ev)?.is_some() || a[1].to_rcd(ev)?.is_some();
    if !accepted {
        let shown = a[1].ppr(ev)?;
        return Err(eval_exception(ev, Ec::TLC_MODULE_ARGUMENT_ERROR, &["second", "JsonSerialize", "sequence or record", &shown]));
    }
    let mut writer = open_for_writing(ev, &a[0])?;
    let text = get_node(ev, &a[1])?.to_json();
    writer.write(&text)
}

/// A file both serializers have opened. TLC opens it in the `try` header and converts the
/// value inside the body, so a value the conversion refuses still leaves an empty file
/// behind, and `JsonSerialize` answers TRUE only after the write.
struct Writer {
    file: std::fs::File,
    path: String,
}

impl Writer {
    fn write(&mut self, text: &str) -> EvalResult<Value> {
        use std::io::Write as _;
        match self.file.write_all(text.as_bytes()) {
            Ok(()) => Ok(Value::Bool(true)),
            Err(e) => Err(io_exception(io_message(&self.path, &e))),
        }
    }
}

/// `file.getParentFile().mkdirs()` and then `new FileWriter(new File(path))`, which opens
/// for writing and truncates.
fn open_for_writing(ev: &mut Ev<'_>, path: &Value) -> EvalResult<Writer> {
    let path = string_argument(ev, path);
    if let Some(parent) = file_parent(&path) {
        let _ = std::fs::create_dir_all(resolve(ev, &parent));
    }
    let target = resolve(ev, &path);
    // `FileOutputStream`'s native open refuses a directory, which Unix `open(2)` with
    // `O_WRONLY` does too, with the same `EISDIR`.
    if target.is_dir() {
        return Err(io_exception(io_message(&path, &std::io::Error::from_raw_os_error(EISDIR))));
    }
    match std::fs::File::create(&target) {
        Ok(file) => Ok(Writer { file, path }),
        Err(e) => Err(io_exception(io_message(&path, &e))),
    }
}

/// `EISDIR` on Linux.
const EISDIR: i32 = 21;

/// `JsonDeserialize(path)` (`Json.java:146-150`).
pub(crate) fn deserialize(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let text = read_file(ev, &a[0])?;
    let node = parse::parse_reader(&text).map_err(io_exception)?;
    get_value(ev, &node)
}

/// `ndJsonDeserialize(path)` (`Json.java:121-138`): one value per line, blank lines ignored
/// as the ndjson spec asks.
pub(crate) fn nd_deserialize(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let text = read_file(ev, &a[0])?;
    let mut values = Vec::new();
    for line in read_lines(&text) {
        // `String.trim()` cuts everything up to and including U+0020, not Unicode
        // whitespace.
        let line = line.trim_matches(|c| c <= ' ');
        if line.is_empty() {
            continue;
        }
        let node = parse::parse_reader(line).map_err(io_exception)?;
        values.push(get_value(ev, &node)?);
    }
    Ok(Value::tuple(values))
}

/// `BufferedReader.readLine()` until it answers null: a line ends at `\n`, `\r` or `\r\n`,
/// the last one needs no terminator, and an empty input has no line at all.
fn read_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find(['\n', '\r']) {
            Some(i) => {
                lines.push(&rest[..i]);
                let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[i + skip..];
            }
            None => {
                lines.push(rest);
                break;
            }
        }
    }
    lines
}

/// `new FileReader(new File(path))` read to the end. `FileReader` decodes with the default
/// charset, UTF-8 since JDK 18 (JEP 400), replacing what it cannot decode.
fn read_file(ev: &mut Ev<'_>, path: &Value) -> EvalResult<String> {
    let path = string_argument(ev, path);
    let source = resolve(ev, &path);
    // `new FileInputStream(File)` refuses a directory up front, where Unix `open(2)` for
    // reading would accept it and fail at the first `read`.
    if source.is_dir() {
        return Err(io_exception(io_message(&path, &std::io::Error::from_raw_os_error(EISDIR))));
    }
    match std::fs::read(&source) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) => Err(io_exception(io_message(&path, &e))),
    }
}

/// How the JDK builds a `FileNotFoundException`'s message: the path as the spec wrote it,
/// then the C library's text for `errno` in parentheses (`io_util_md.c`,
/// `throwFileNotFoundException`). The path is not made absolute, so a relative one is
/// reported relative.
fn io_message(path: &str, e: &std::io::Error) -> String {
    format!("{path} ({})", tlars_java::last_error_string(e.raw_os_error().unwrap_or(0)))
}

/// The path argument, which `MethodValue.eval` has already cast to `StringValue`
/// (`JavaParam::StringValue`), so anything else failed before the method ran.
fn string_argument(ev: &Ev<'_>, v: &Value) -> String {
    match v {
        Value::Str(a) => ev.tool().graph().interner().text(*a).to_string(),
        other => unreachable!("the path is cast to StringValue first, not {}", other.java_class()),
    }
}

/// `new File(path)` opened by the process: a relative path resolves against the directory
/// the run was started in, not against `user.dir`, because the JDK hands the path to
/// `open(2)` unchanged.
fn resolve(ev: &Ev<'_>, path: &str) -> PathBuf {
    ev.tool().globals().cwd.join(path)
}

/// `new File(path).getParent()` (`UnixFileSystem.normalize` then `File.getParent`):
/// repeated separators collapse and a trailing one goes, and a path with no separator left
/// has no parent at all, which is why `JsonSerialize("out.json", ...)` creates no directory.
fn file_parent(path: &str) -> Option<String> {
    let absolute = path.starts_with('/');
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        // `/a` has the root as its parent; `a` and `/` have none.
        return (absolute && parts.len() == 1).then(|| "/".to_string());
    }
    let parent = parts[..parts.len() - 1].join("/");
    Some(if absolute { format!("/{parent}") } else { parent })
}

/// `getValue(JsonElement)` (`Json.java:462-487`): an array is a tuple, an object a record,
/// and a number has to fit in an `int`.
fn get_value(ev: &mut Ev<'_>, node: &Node) -> EvalResult<Value> {
    match node {
        Node::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(get_value(ev, item)?);
            }
            Ok(Value::tuple(values))
        }
        Node::Object(members) => {
            let mut names = Vec::with_capacity(members.len());
            let mut values = Vec::with_capacity(members.len());
            for (name, value) in members {
                names.push(ev.tool().graph().interner().intern(name));
                values.push(get_value(ev, value)?);
            }
            Ok(Value::Record(Value::record_arc(names, values, false)))
        }
        // `Integer.parseInt(primitive.getAsString())` on the literal as the document wrote
        // it, so `2.0` and `1e2` are refused along with anything too large.
        Node::Number(literal) => match tlars_java::parse_int(literal) {
            Ok(n) => Ok(Value::Int(n)),
            Err(_) => {
                Err(io_exception(format!("Cannot convert value: expected a 32-bit integer JSON number, but found {literal}")))
            }
        },
        Node::Bool(b) => Ok(Value::Bool(*b)),
        Node::Str(s) => Ok(Value::Str(ev.tool().graph().interner().intern(s))),
        // `node.toString()` of a `JsonNull` is `null`.
        Node::Null => Err(io_exception(format!("Cannot convert value: unsupported JSON value {}", node.to_json()))),
    }
}

#[cfg(test)]
mod tests {
    use super::file_parent;

    /// `new File(path).getParent()`, which decides whether `mkdirs()` runs.
    #[test]
    fn file_parent_matches_java() {
        assert_eq!(file_parent("out.json"), None);
        assert_eq!(file_parent("."), None);
        assert_eq!(file_parent("sub/dir/out.json").as_deref(), Some("sub/dir"));
        assert_eq!(file_parent("sub//dir//out.json").as_deref(), Some("sub/dir"));
        assert_eq!(file_parent("/a").as_deref(), Some("/"));
        assert_eq!(file_parent("/a/b").as_deref(), Some("/a"));
        assert_eq!(file_parent("/"), None);
        assert_eq!(file_parent("a/").as_deref(), None);
    }
}
