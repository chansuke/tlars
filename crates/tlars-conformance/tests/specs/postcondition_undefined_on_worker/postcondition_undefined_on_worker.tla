---- MODULE postcondition_undefined_on_worker ----
VARIABLE x
\* The same unresolvable POSTCONDITION, this time with the violation found while the
\* successors are explored: the exception reaches `Worker.run`, which drops it because the
\* violation already recorded an error state, so nothing follows the trace.
Init == x = 0
Next == x' = 1
Inv == x = 0
====
