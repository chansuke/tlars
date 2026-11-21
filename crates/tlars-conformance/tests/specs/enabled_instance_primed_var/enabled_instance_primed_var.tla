---- MODULE enabled_instance_primed_var ----
VARIABLE x
I == INSTANCE Sub WITH y <- x
Init == x = 0
Next == x' = x
Inv == I!Op
====
