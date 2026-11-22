---- MODULE trace_in_next_during_replay ----
\* `TLCExt!Trace` inside the next-state relation, reached again while the counterexample is
\* printed. Every state the replay regenerates carries the trace record it was written at
\* (`ConcurrentTLCTrace.getTrace` sets `uid` on each one), so `Trace` answers there too, and
\* each answer reconstructs the behaviour again.
EXTENDS TLCExt, TLC, Naturals, Sequences
VARIABLE x
Init == x = 0
Next == /\ PrintT(Len(Trace))
        /\ x' = (x + 1) % 3
Inv == x < 2
====
