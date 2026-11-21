---- MODULE intern_recordvalue_statics ----
EXTENDS TLC, Sequences
VARIABLE x
ASSUME PrintT({"z" \o "z"} \cup DOMAIN TLCGet("action").location)
Init == x = 0
Next == x' = x
====
