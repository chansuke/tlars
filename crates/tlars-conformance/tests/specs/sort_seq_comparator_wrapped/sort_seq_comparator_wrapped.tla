---- MODULE sort_seq_comparator_wrapped ----
EXTENDS TLC, Naturals
VARIABLE x
\* SortSeq is a MethodValue, so a failure its body lets out is rewrapped as 2154 naming the
\* Java method. The comparator runs inside the body, and a failed CHOOSE is a
\* TLCRuntimeException rather than an EvalException, so it is the one the catch rewraps.
\* The corpus only reaches the comparator with an EvalException (sort_comparator_error),
\* which passes through unwrapped.
Init == PrintT(SortSeq(<<1, 2>>, LAMBDA a, b : CHOOSE y \in {} : TRUE)) /\ x = 0
Next == x' = 1
====
