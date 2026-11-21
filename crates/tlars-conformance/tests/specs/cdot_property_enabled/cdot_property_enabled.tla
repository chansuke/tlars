---- MODULE cdot_property_enabled ----
VARIABLE x
Init == x = 0
Next == (x' = 1) \cdot (x' = 2)
====
