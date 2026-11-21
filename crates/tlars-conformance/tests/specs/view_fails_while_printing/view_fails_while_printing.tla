---- MODULE view_fails_while_printing ----
EXTENDS Integers
VARIABLE x
\* The VIEW is what `-view` prints a trace state as. It holds for the initial state, which is
\* fingerprinted, and fails for the successor being printed, so the failure replaces the rest
\* of the trace and ends the run.
Init == x = 0
Next == x < 2 /\ x' = x + 1
V == 1 \div (x - 1)
Inv == x < 2
====
