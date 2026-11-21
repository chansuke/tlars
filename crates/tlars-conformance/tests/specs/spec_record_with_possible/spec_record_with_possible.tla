---- MODULE spec_record_with_possible ----
EXTENDS TLC
VARIABLE x
\* `TLCGet("spec")` names the constraints, and with _POSSIBLE in the cfg those include the
\* `_Track` wrappers of the module TLC generated for it.
Init == x = 0
Next == x' = 0
P == x = 0
Post == PrintT(<<TLCGet("spec").constraints, TLCGet("spec").actionconstraints>>) /\ TRUE
====
