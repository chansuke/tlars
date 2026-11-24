---- MODULE tlceval_nested_constant ----
EXTENDS TLC, Naturals
VARIABLE x
\* A TLCEval whose argument is another TLCEval takes the cached path twice, each under its
\* own node, so the inner Print runs once however often the outer one is evaluated. TLC holds
\* a reentrant write lock across the inner evaluation; releasing it first, as tlars does,
\* reaches the same value per node.
Init == x = 0
Next == x' = (x + 1) % 3 /\ TLCEval(TLCEval(Print("nested", 1..2))) = {1, 2}
====
