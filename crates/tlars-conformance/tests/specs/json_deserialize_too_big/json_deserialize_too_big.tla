---- MODULE json_deserialize_too_big ----
EXTENDS Json, TLC, Naturals, Sequences
\* The same refusal for a number that is an integer but not a 32-bit one.
ASSUME JsonDeserialize("big.json") = 0
====
