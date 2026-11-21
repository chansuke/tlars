---- MODULE Spec ----
EXTENDS TLC
VARIABLE x
Foo == TLCSet("s:k", 42)
ASSUME PrintT(TLCGet("s:k"))
Init == x = 0
Next == x' = x
====
