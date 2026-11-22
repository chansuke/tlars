---- MODULE json_deserialize_malformed ----
EXTENDS Json, TLC, Naturals, Sequences
\* A document that stops inside an object: `Streams.parse` has read something, so the
\* EOFException is wrapped rather than turned into JsonNull, and the message carries gson's
\* line, column and JSON path.
ASSUME JsonDeserialize("bad.json") = 0
====
