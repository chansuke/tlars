---- MODULE alias_prime_on_partial ----
VARIABLE x, y
\* The successor is incomplete, yet it is still the `s2` the ALIAS reads a prime from, and
\* it goes through the ALIAS itself rather than being printed with its nulls.
Init == x = 0 /\ y = 0
Next == x' = 1
Inv == x = 0
Alias == [x |-> x, xp |-> x']
====
