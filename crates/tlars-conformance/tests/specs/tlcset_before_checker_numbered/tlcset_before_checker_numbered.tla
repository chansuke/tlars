---- MODULE tlcset_before_checker_numbered ----
EXTENDS TLC
VARIABLE x
Foo == TLCSet(0, 42)
ASSUME PrintT(TLCGet(0))
Init == x = 0
Next == x' = x
====
