---- MODULE alias_trace_on_two_state ----
EXTENDS TLCExt
VARIABLE x
\* A counterexample of two states is printed from `s1` and `s2` directly, with an empty
\* prefix, so `TLCExt!Trace` is the empty tuple for both of them.
Init == x = 0
Next == x' = 1
Inv == x = 0
Alias == [x |-> x, t |-> Trace]
====
