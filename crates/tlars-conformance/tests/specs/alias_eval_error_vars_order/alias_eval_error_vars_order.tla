---- MODULE alias_eval_error_vars_order ----
EXTENDS Integers
VARIABLE z, a
\* An ALIAS that fails prints the state's own variables in `vars[]` order with
\* `_ALIASEvalError` appended, because the record is built by appending to a snapshot.
Init == z = 0 /\ a = 0
Next == z' = z /\ a' = a
Alias == 1 \div 0
Inv == a = 1
====
