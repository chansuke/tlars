---- MODULE symmetry_two_model_values ----
EXTENDS TLC
VARIABLE x
CONSTANT S
\* Two model values in one permuted set: the subgroup is computed and collapses the two
\* initial states into one.
Sym == Permutations(S)
Init == x \in S
Next == x' = x
====
