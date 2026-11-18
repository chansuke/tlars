---- MODULE LintRecordsSuppressed ----
\* Each clash is suppressed: by its own right-hand side, by another field of the same record,
\* or by another record with the same fields.
EXTENDS Naturals, Integers
CONSTANT c
VARIABLE v
bar == 23
baz == 24
S == [bar |-> c]
T == [bar |-> 1, other |-> v]
U == [baz |-> 1]
U2 == [baz |-> Nat]
X == [bar |-> Int, baz |-> 2]
F(p) == [bar |-> p]
G == [bar |-> BOOLEAN, baz |-> 0]
H == [bar |-> F(1)]
====
