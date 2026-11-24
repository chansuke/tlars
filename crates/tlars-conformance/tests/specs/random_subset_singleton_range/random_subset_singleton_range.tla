---- MODULE random_subset_singleton_range ----
EXTENDS Randomization, Naturals, TLC
VARIABLE x
\* needBigInteger answers the empty and singleton co-domain cases before it multiplies, so
\* a function set whose cardinality is 1 is indexed with ints however large its domain is.
\* The other three draws pin the mixed-radix walk over a product, a record set and a
\* function set whose constituents have different cardinalities.
Init == /\ PrintT(RandomSubset(2, [1..100 -> {7}]))
        /\ PrintT(RandomSubset(3, (1..2) \X (1..3) \X (1..4)))
        /\ PrintT(RandomSubset(3, [a : 1..2, b : 1..5]))
        /\ PrintT(RandomSubset(3, [{"p", "q"} -> 1..4]))
        /\ x = 0
Next == x' = 1
====
