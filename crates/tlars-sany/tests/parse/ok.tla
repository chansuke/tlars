garbage before
---- MODULE Ok ----
EXTENDS Naturals
CONSTANT N
VARIABLE v
Init == v = 0
Next == v' = v + 1 /\ v < N
Spec == Init /\ [][Next]_v /\ WF_v(Next)
THEOREM Spec => []TRUE
====
trailing
