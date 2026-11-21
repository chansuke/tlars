---- MODULE enabled_unassigned_variable ----
VARIABLE x
Init == (ENABLED x) /\ x = 0
Next == x' = x
====
