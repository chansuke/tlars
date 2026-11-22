---- MODULE json_deserialize_shapes ----
EXTENDS Json, TLC, Naturals, Sequences
\* An object is a record and an array a tuple, with the fields in the order the document
\* wrote them: `getRecordValue` builds the record unnormalised and `ToJson` reads the
\* snapshot, so a round trip through both shows the document's own order. The corpus has
\* cases for all of this and reaches none of them, because its own `fixtures/` files are
\* not copied next to the spec. `data.json` also carries what the reader has to decode:
\* raw UTF-8 (FileReader's charset is UTF-8 since JDK 18), a \u escape, a surrogate pair
\* that is one astral character, and the escapes gson accepts, \/ among them.
ASSUME PrintT(ToJson(JsonDeserialize("data.json")))
ASSUME PrintT(JsonDeserialize("data.json"))
\* A repeated key: gson's `JsonObject.add` replaces the value and keeps the first
\* position, so the record has one field and not two.
ASSUME PrintT(ToJson(JsonDeserialize("dup.json")))
\* Keys need not be TLA+ identifiers. `UniqueString.uniqueStringOf` interns them as they
\* are and the record prints them as written.
ASSUME PrintT(JsonDeserialize("keys.json"))
\* What `Strictness.LENIENT` accepts: comments, unquoted names, single quotes, and `;`
\* where a `,` belongs.
ASSUME PrintT(ToJson(JsonDeserialize("lenient.json")))
====
