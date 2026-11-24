---- MODULE random_element_lazy_set ----
EXTENDS Randomization, Naturals, TLC
VARIABLE x
\* RandomElement's default branch: everything that is not one of the three product kinds or
\* an interval is converted with toSetEnum and then indexed. The corpus reaches it through
\* {x \in S : P} and SUBSET {}; these are the remaining lazy kinds, plus a function set
\* whose co-domain is itself a function set so the recursion nests.
Init == /\ PrintT(RandomElement((1..4) \cup (7..9)))
        /\ PrintT(RandomElement(UNION {{1, 2}, {5, 6}}))
        /\ PrintT(RandomElement((1..9) \ {2, 4}))
        /\ PrintT(RandomElement((1..9) \cap (4..20)))
        /\ PrintT(RandomElement(SUBSET {"a", "b"}))
        /\ PrintT(RandomElement([{1, 2} -> [{3} -> 4..6]]))
        /\ x = 0
Next == x' = 1
====
