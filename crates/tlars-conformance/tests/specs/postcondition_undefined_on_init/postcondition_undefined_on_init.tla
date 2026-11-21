---- MODULE postcondition_undefined_on_init ----
VARIABLE x
\* The POSTCONDITION cannot be looked up, and the invariant is violated by the initial
\* state. The lookup's `Assert.fail` is raised inside `modelCheckImpl`'s own try, so 2102
\* carries its message, the initial states are generated again, and its code ends the run.
Init == x = 0
Next == x' = x
Inv == x = 1
====
