---- MODULE LintRecordsLet ----
\* Clashes with definitions of an instantiated module and inside LET.
EXTENDS Naturals
I == INSTANCE LintRecordsBase
base == 0
L == LET q == 1 IN [base |-> q, q |-> 2]
M == \A x \in {1} : [base |-> x] = [base |-> 1]
====
