---- MODULE symmetry_duplicate_perm ----
EXTENDS TLC
VARIABLE x
CONSTANT m1
\* {f, f} holds one permutation once normalised, so the set is still too small to warrant
\* symmetry and warning 2300 names it.
Pair(f) == {f, f}
Sym == Pair(m1 :> m1)
Init == x = 0
Next == x' = 0
====
