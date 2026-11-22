---- MODULE random_subset_lazy_set ----
EXTENDS Randomization, Naturals, TLC
VARIABLE x
\* A lazy set is none of the three kinds that index themselves, so
\* EnumerableValue.getRandomSubset converts it to a SetEnumValue first and the LCG indexes
\* the converted, normalised elements. The corpus reaches this path only through
\* {x \in S : P}; these are the other lazy kinds.
Init == /\ PrintT(RandomSubset(2, (1..4) \cup (7..9)))
        /\ PrintT(RandomSubset(2, UNION {{1, 2}, {5, 6}}))
        /\ PrintT(RandomSubset(2, (1..9) \ {2, 4}))
        /\ PrintT(RandomSubset(2, (1..9) \cap (4..20)))
        /\ PrintT(RandomSubset(2, SUBSET {"a", "b"}))
        /\ x = 0
Next == x' = 1
====
