---- MODULE Deep ----
x == LET a == 1 IN CASE a = 1 -> {y \in 1..2 : y > a} [] OTHER -> [i \in 1..3 |-> i]
y == [x EXCEPT ![1].a = @ + 1, !.b = 2]
z == WF_<<x, y>>(x' = 1) /\ \A q \in {} : <>[]q
====
