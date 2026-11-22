---- MODULE any_in_state_variable ----
EXTENDS TLC
CONSTANT mv
VARIABLE x
\* `UserValue.compareTo` ranks every model value below an overridden value instead of
\* refusing the comparison, so `Any = mv` is answered. It has no `fingerPrint`, so the state
\* that holds `ANY` reaches `Value.fingerPrint` and TLC refuses it.
Init == PrintT(<<Any = mv, mv = Any>>) /\ x = Any
Next == x' = x
====
