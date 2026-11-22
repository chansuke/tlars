---- MODULE json_deserialize_trailing_data ----
EXTENDS Json, TLC, Naturals, Sequences
\* `JsonParser.parseReader` restores the reader's strictness before checking that the
\* document is spent, so trailing data is reported by the strict peek that follows, not by
\* the "Did not consume the entire document." the method would otherwise raise.
ASSUME JsonDeserialize("two.json") = 0
====
