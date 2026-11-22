---- MODULE random_subset_huge_product ----
EXTENDS Randomization, Naturals, TLC
VARIABLE x
\* 10^10 elements, so needBigInteger holds and TLC draws its indices with
\* BigIntegerSubsetEnumerator, which tlars does not implement.
Init == PrintT(RandomSubset(2, [1..10 -> 1..10])) /\ x = 0
Next == x' = 1
====
