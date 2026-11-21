---- MODULE Spec ----
VARIABLE x
Init == (ENABLED x) /\ x = 0
Next == x' = x
====
