---- MODULE json_deserialize_empty_file ----
EXTENDS Json, TLC, Naturals, Sequences
\* `Streams.parse` answers JsonNull for a document that ends before anything is read,
\* for compatibility with gson 1.5, so an empty file fails the same way an explicit null
\* does rather than as a syntax error.
ASSUME JsonDeserialize("empty.json") = 0
====
