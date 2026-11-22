---- MODULE bags_diff_unchecked_count ----
EXTENDS Bags, TLC, Naturals
VARIABLE x
\* `(-)` checks neither operand with `IsABag`, so a count that is not an integer reaches
\* `(IntValue) values1[i]` and fails as a `ClassCastException` rather than as the argument
\* error `(+)` would raise (`Bags.java:215-262`). The corpus reaches the same cast through
\* `\sqsubseteq` and `BagUnion`, but not through `(-)`.
Init == x = TRUE
Next == UNCHANGED x
Inv == x /\ ((1 :> "a") (-) EmptyBag) = EmptyBag
====
