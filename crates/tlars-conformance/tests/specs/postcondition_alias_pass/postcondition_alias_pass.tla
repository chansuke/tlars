---- MODULE postcondition_alias_pass ----
EXTENDS TLC, Naturals
VARIABLE x
\* `doPostCondition` evaluates the ALIAS over every action of the rebuilt trace before it
\* checks the POSTCONDITION, so the ALIAS prints twice more than the trace printer did and
\* the register it writes is what the POSTCONDITION reads.
Init == x = 0 /\ TLCSet(0, 0)
Next == x' = 1
Alias == [x |-> x, p |-> PrintT("ALIAS_CALL") /\ TLCSet(0, TLCGet(0) + 1)]
Post == PrintT(<<"POST_CNT", TLCGet(0)>>) /\ TRUE
Inv == x = 0
====
