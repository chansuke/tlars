---- MODULE trace_in_next_during_replay ----
\* `TLCExt!Trace` inside the next-state relation, reached again while the counterexample is
\* printed. The states the replay regenerates carry the trace record each one was written
\* at in TLC (`ConcurrentTLCTrace.getTrace` sets `uid` on every reconstructed state), so
\* `Trace` answers there too; tlars' reconstruction leaves the uid unset.
EXTENDS TLCExt, TLC, Naturals, Sequences
VARIABLE x
Init == x = 0
Next == /\ PrintT(Len(Trace))
        /\ x' = (x + 1) % 3
Inv == x < 2
====
