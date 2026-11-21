---- MODULE alias_record_written_order ----
VARIABLE a, z
\* `RecordValue.toState()` takes a snapshot rather than normalising, so the alias record's
\* fields print in the order the spec wrote them, not in intern order.
Init == a = 0 /\ z = 0
Next == a' = a /\ z' = z
Alias == [z |-> 1, a |-> a]
Inv == a = 1
====
