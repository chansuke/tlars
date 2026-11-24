---- MODULE json_nd_deserialize_two_on_a_line ----
EXTENDS Json, TLC, Naturals, Sequences
\* ndJson parses each line on its own, so two values on one line are trailing data.
ASSUME ndJsonDeserialize("two.ndjson") = 0
====
