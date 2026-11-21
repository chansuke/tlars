---- MODULE Spec ----
EXTENDS TLC, Sequences
VARIABLE x
Dyn == "z" \o "z"
ASSUME PrintT({Dyn} \cup DOMAIN TLCGet("stats"))
Init == x = 0
Next == x' = x
====
