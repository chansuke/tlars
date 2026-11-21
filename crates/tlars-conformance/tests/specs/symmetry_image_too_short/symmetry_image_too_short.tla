---- MODULE symmetry_image_too_short ----
EXTENDS TLC
VARIABLE x
CONSTANT Permutations1
\* Warning 2300 names the argument by its source image, which TLC unwraps by cutting
\* "Permutations(" off the front and ")" off the end whether or not they are there.
Foo(a) == {a :> a}
Sym == Foo(Permutations1)
Init == x = 0
Next == x' = 0
====
