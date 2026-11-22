---- MODULE json_deserialize_null ----
EXTENDS Json, TLC, Naturals, Sequences
\* JSON's null has no TLA+ value, and the message quotes the element's own toString().
ASSUME JsonDeserialize("null.json") = 0
====
