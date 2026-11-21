---- MODULE enabled_let_thunk_cache ----
EXTENDS Integers
VARIABLE x
Action == LET y == x' IN /\ x' \in {0, 1}
                         /\ y + 0 = 1
Init == x = 0
Next == x' = x
Inv == ENABLED Action
====
