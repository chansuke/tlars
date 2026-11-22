---- MODULE any_compare_other_special_set ----
EXTENDS TLC, Naturals
VARIABLE x
\* `AnySet.compareTo` throws whatever value it is handed, so with `ANY` on the left the
\* message names `ANY` as the receiver. The corpus only has the other direction
\* (`Nat = Any`), whose text `Naturals.compareTo` builds.
Init == x = 0
Next == x' = x
Inv == Any # Nat
====
