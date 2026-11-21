---- MODULE postcondition_skipped_on_partial ----
EXTENDS TLC
VARIABLE x, y
\* An incomplete successor was never written to the trace, so `doPostCondition` indexes past
\* the end of it and no POSTCONDITION is checked: nothing follows the trace.
Init == x = 0 /\ y = 0
Next == x' = 1
Post == PrintT(<<"POST_RAN", TLCGet("distinct")>>) /\ FALSE
====
