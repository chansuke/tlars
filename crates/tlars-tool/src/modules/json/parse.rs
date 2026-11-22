//! `com.google.gson.JsonParser.parseReader(Reader)` at gson 2.14.0: the lenient
//! `JsonReader` that `tlc2.module.Json` reads documents with.

use super::node::Node;

/// `JsonParser.parseReader(Reader)`. `Err` is the `getMessage()` of the exception it throws,
/// which is what `MethodValue.eval` puts into TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE.
pub(super) fn parse_reader(_text: &str) -> Result<Node, String> {
    todo!("M3 unit 6: gson's lenient JsonReader, which JsonDeserialize and ndJsonDeserialize read with")
}
