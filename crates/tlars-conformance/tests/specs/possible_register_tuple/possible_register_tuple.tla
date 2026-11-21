---- MODULE possible_register_tuple ----
EXTENDS TLC
VARIABLE x
\* The spec writes `_Possible`'s own register. A non-empty tuple there makes `DOMAIN old` an
\* integer interval, so `_Track`'s `name \in DOMAIN old` fails and the run must not pass.
Init == x = 0 /\ TLCSet("s:_possible", <<1>>)
Next == x' = 0
P == x = 0
====
