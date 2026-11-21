---- MODULE Spec ----
EXTENDS TLC, Integers
VARIABLE x
ASSUME /\ TLCSet(3, "three")
       /\ TLCSet(0, 0)
       /\ TLCSet(17, <<1, 2>>)
       /\ TLCSet(4, {1})
       /\ PrintT(TLCGet("all"))
       /\ PrintT(DOMAIN TLCGet("all"))
Init == x = 0
Next == x' = x
====
