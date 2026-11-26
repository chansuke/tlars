---- MODULE assume_name_through_instance ----
\* A named assumption of an instantiated module is an operator of the instantiating one, and
\* TLC evaluates it in ASSUME, Init, Next and ENABLED.
EXTENDS Naturals
VARIABLE x
N == 3
INSTANCE AssumeInner
ASSUME NIsSmall
Init == x = 0 /\ NIsSmall
Next == NIsSmall /\ x < 2 /\ x' = x + 1
Inv == x = 2 \/ ENABLED (NIsSmall /\ x' = x + 1)
====
