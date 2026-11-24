---- MODULE select_seq_test_wrapped ----
EXTENDS Sequences, TLC, Naturals
VARIABLE x
\* The same rewrapping for SelectSeq, which tlars also registers as a lazy native because
\* its second argument is an operator. No corpus case reaches it.
Init == PrintT(SelectSeq(<<1, 2>>, LAMBDA a : CHOOSE y \in {} : TRUE)) /\ x = 0
Next == x' = 1
====
