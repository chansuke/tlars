---- MODULE random_subset_negative_interval ----
EXTENDS Randomization, Integers, TLC
VARIABLE x
\* RandomSubset does not check the sign of its first argument, so a negative count reaches
\* `new ValueVec(k)` and fails with the JVM's NegativeArraySizeException before any draw.
\* The corpus covers that only for an enumerated set; this is the interval's own
\* getRandomSubset.
Init == PrintT(RandomSubset(-1, 1..3)) /\ x = 0
Next == x' = 1
====
