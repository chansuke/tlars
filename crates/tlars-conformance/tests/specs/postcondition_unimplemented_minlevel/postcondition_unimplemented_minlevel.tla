---- MODULE postcondition_unimplemented_minlevel ----
EXTENDS TLCExt
VARIABLE x
Init == x = 0
Next == x' = x
Post == TRUE \/ (CounterExample = <<>>)
====
