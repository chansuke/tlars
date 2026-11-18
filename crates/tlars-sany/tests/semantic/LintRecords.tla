---- MODULE LintRecords ----
\* Record fields named like definitions, whose values are not built from declarations.
EXTENDS Naturals, LintRecordsBase
bar == 23
R == [bar |-> 42]
U == [bar |-> 2, qux |-> 3]
W == [base |-> 1 + 1]
====
