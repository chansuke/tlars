---- MODULE Spec ----
VARIABLE x
I == INSTANCE Sub WITH y <- x
Init == x = 0
Next == x' = x
Inv == I!Op
====
