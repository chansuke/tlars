//! `com.google.gson.JsonElement` and `JsonElement.toString()`, at the gson 2.14.0 that
//! `tlatools/lib/gson/gson-2.14.0.jar` holds.
//!
//! `tlc2.module.Json` builds a tree of these and prints it, so the bytes of `ToJson` are
//! gson's writer, not TLC's. `toString()` writes through a `JsonWriter` that is
//! `Strictness.LENIENT` and not HTML-safe, in the compact formatting style
//! (`JsonElement.java:421-433`), which is why `<a>` stays `<a>` and there is no space
//! after a comma or a colon.

/// A `JsonElement`. Only the shapes `Json.java` builds or `JsonParser` returns.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Node {
    /// `JsonNull.INSTANCE`, which only the parser produces.
    Null,
    /// `JsonPrimitive(boolean)`.
    Bool(bool),
    /// `JsonPrimitive(Number)`, as its literal: that is what `toString()` writes
    /// (`JsonWriter.value(Number)` appends `value.toString()`) and what `getAsString()`
    /// returns. `new JsonPrimitive(int)` holds an `Integer`, a parsed number a
    /// `LazilyParsedNumber` whose `toString()` is the text as written in the document.
    Number(String),
    /// `JsonPrimitive(String)`.
    Str(String),
    /// `JsonArray`.
    Array(Vec<Node>),
    /// `JsonObject`, whose `LinkedTreeMap` iterates in insertion order.
    Object(Vec<(String, Node)>),
}

impl Node {
    /// `new JsonPrimitive(int)`.
    pub(super) fn int(value: i32) -> Node {
        Node::Number(value.to_string())
    }

    /// `JsonObject.add(property, value)`: `LinkedTreeMap.put` overwrites a key that is
    /// already there and leaves it at the position of its first insertion.
    pub(super) fn add(&mut self, property: String, value: Node) {
        let Node::Object(members) = self else { unreachable!("add is JsonObject's") };
        match members.iter_mut().find(|(name, _)| *name == property) {
            Some(slot) => slot.1 = value,
            None => members.push((property, value)),
        }
    }

    /// `JsonElement.toString()`.
    pub(super) fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    /// `JsonElementTypeAdapter.write`.
    fn write(&self, out: &mut String) {
        match self {
            Node::Null => out.push_str("null"),
            Node::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Node::Number(literal) => out.push_str(literal),
            Node::Str(s) => string(s, out),
            Node::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Node::Object(members) => {
                out.push('{');
                for (i, (name, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    string(name, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// `JsonWriter.string(value)` with `REPLACEMENT_CHARS` (`JsonWriter.java:181-199`,
/// `743-772`): below `0x20` the named escapes and otherwise `\u00xx` in lower-case hex,
/// `"` and `\`, and the two line separators. Everything else is written as it is, so a
/// non-ASCII character keeps its own bytes and `<`, `>`, `&`, `=`, `'` are not escaped
/// (those are the HTML-safe table's, which `toString()` does not use).
///
/// gson walks UTF-16 code units; walking `char`s is the same outcome, because a surrogate
/// of an astral character is neither below 128 nor a line separator and so is written
/// unchanged either way.
fn string(value: &str, out: &mut String) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::Node;

    /// The escaping and rendering gson 2.14.0 produces, from
    /// `JsonElement.toString()` run on the pinned jar.
    #[test]
    fn to_string_matches_gson() {
        let cases: &[(Node, &str)] = &[
            (Node::Str("a\"b".into()), r#""a\"b""#),
            (Node::Str("a\\b".into()), r#""a\\b""#),
            (Node::Str("a\nb".into()), r#""a\nb""#),
            (Node::Str("a\tb".into()), r#""a\tb""#),
            (Node::Str("a\rb".into()), r#""a\rb""#),
            (Node::Str("a\u{8}b".into()), r#""a\bb""#),
            (Node::Str("a\u{c}b".into()), r#""a\fb""#),
            (Node::Str("a\0b".into()), r#""a\u0000b""#),
            (Node::Str("a\u{1f}b".into()), r#""a\u001fb""#),
            (Node::Str("a\u{7f}b".into()), "\"a\u{7f}b\""),
            (Node::Str("<a>&='b'".into()), r#""<a>&='b'""#),
            (Node::Str("a\u{2028}b".into()), r#""a\u2028b""#),
            (Node::Str("a\u{2029}b".into()), r#""a\u2029b""#),
            (Node::Str("éあ".into()), "\"éあ\""),
            (Node::Str("a/b".into()), r#""a/b""#),
            (Node::Str(String::new()), r#""""#),
            (Node::int(5), "5"),
            (Node::int(-5), "-5"),
            (Node::int(i32::MIN), "-2147483648"),
            (Node::Bool(true), "true"),
            (Node::Bool(false), "false"),
            (Node::Null, "null"),
            (Node::Array(Vec::new()), "[]"),
            (Node::Object(Vec::new()), "{}"),
            (Node::Array(vec![Node::int(1), Node::Object(Vec::new())]), "[1,{}]"),
            (Node::Object(vec![("a".into(), Node::int(1)), ("b".into(), Node::int(2))]), r#"{"a":1,"b":2}"#),
        ];
        for (node, want) in cases {
            assert_eq!(&node.to_json(), want, "{node:?}");
        }
    }

    /// `JsonObject.add` of a key that is already there: the value is replaced and the key
    /// keeps its first position, as gson prints `{"b":3,"a":2}` for b, a, b.
    #[test]
    fn repeated_key_keeps_its_position() {
        let mut o = Node::Object(Vec::new());
        o.add("b".into(), Node::int(1));
        o.add("a".into(), Node::int(2));
        o.add("b".into(), Node::int(3));
        assert_eq!(o.to_json(), r#"{"b":3,"a":2}"#);
    }
}
