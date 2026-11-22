---- MODULE random_subset_negative_product ----
EXTENDS Randomization, Integers, TLC
VARIABLE x
\* The same NegativeArraySizeException on SetOfFcnsOrRcdsValue's own getRandomSubset, which
\* allocates before it decides whether the product needs big integers.
Init == PrintT(RandomSubset(-1, {1} \X {2})) /\ x = 0
Next == x' = 1
====
