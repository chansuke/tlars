---- MODULE cache_keys_of_two_kinds ----
\* `TLCExt!TLCCache` keys its per-node cache in a `java.util.HashMap`, so a stored key is
\* only compared with `Value.equals` when its hash matches. Two keys of different kinds
\* therefore both miss instead of raising the comparison error that `{} = 1` would.
EXTENDS TLCExt, TLC, Naturals
VARIABLE x
Init == x = 0
Next == UNCHANGED x
Inv == \A k \in {1, 2} : TLCCache(Print("computed", 7), IF k = 1 THEN {} ELSE 1) = 7
====
