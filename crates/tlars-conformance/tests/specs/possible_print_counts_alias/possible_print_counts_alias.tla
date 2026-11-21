---- MODULE possible_print_counts_alias ----
EXTENDS _Possible, Naturals
\* `_Possible!_PrintCounts` has no Java override, so TLC evaluates its TLA+ body: it prints
\* `_Counts` and answers TRUE. An ALIAS that is not a record leaves the state printed plainly.
VARIABLE x
Init == x = 0
Next == x' = x + 1 /\ x < 2
Inv == x < 2
====
