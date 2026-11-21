---- MODULE symmetry_one_element_set ----
EXTENDS TLC
VARIABLE x
\* One permutation for one permuted set: TLC warns 2300 and computes no subgroup, so the
\* domain and range are never asked to be model values.
Sym == Permutations({1})
Init == x = 0
Next == x' = 0
====
