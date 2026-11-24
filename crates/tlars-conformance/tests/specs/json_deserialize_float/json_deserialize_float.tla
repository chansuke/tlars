---- MODULE json_deserialize_float ----
EXTENDS Json, TLC, Naturals, Sequences
\* A number keeps the literal the document wrote (gson's LazilyParsedNumber), and
\* `Integer.parseInt` of it decides: `1.5` is a JSON number and not a TLA+ integer.
ASSUME JsonDeserialize("float.json") = 0
====
