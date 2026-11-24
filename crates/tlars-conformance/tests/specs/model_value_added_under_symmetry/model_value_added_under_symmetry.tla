---- MODULE model_value_added_under_symmetry ----
\* `TLCExt!TLCModelValue` creates a model value while the search runs, so its creation
\* index is past the end of every `MVPerm` the SYMMETRY group was sized for.
\* `ModelValue.add`'s own comment calls this bogus behaviour.
EXTENDS TLCExt, TLC
CONSTANT a, b
VARIABLE x
Symm == Permutations({a, b})
Init == x = a
Next == x' = TLCModelValue("c")
====
