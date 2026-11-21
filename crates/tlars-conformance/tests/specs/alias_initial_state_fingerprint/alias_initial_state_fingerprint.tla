---- MODULE alias_initial_state_fingerprint ----
EXTENDS Integers
VARIABLE x
\* `getTrace` recovers the trace's initial state by its fingerprint and keeps it on the
\* state info, so that state's `fp:` is the original one even though the ALIAS changed x.
\* Every later state was recovered from its predecessor, which caches nothing, so its `fp:`
\* is the alias's own.
Init == x = 0
Next == x < 2 /\ x' = x + 1
Inv == x < 2
Alias == [x |-> x + 100]
====
