---- MODULE tlc_trace_round_trip ----
EXTENDS _TLCTrace, TLC, Naturals, Sequences
CONSTANT MV
VARIABLES x, s, f, r
Init == x = 0 /\ s = "a" /\ f = [i \in 1..2 |-> i * 2] /\ r = [tag |-> MV, items |-> {"p", "q"}]
Next == x' = x + 1 /\ s' = s /\ f' = f /\ r' = r
Inv == x < 2
====
