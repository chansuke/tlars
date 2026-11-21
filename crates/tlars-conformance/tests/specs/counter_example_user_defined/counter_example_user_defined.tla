---- MODULE counter_example_user_defined ----
VARIABLE x
\* `getCounterExampleDef()` runs before the POSTCONDITIONs are looked up, so a
\* `CounterExample` the spec defines itself fails the check even though the cfg names no
\* POSTCONDITION at all.
CounterExample == 1
Init == x = 0
Next == x' = x
Inv == x = 1
====
