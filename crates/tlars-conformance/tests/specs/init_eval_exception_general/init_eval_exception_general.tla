---- MODULE init_eval_exception_general ----
EXTENDS TLC, Naturals
VARIABLE x
\* The register makes the invariant fail on its first evaluation only, so `doInit`'s
\* call-stack replay finishes without failing again and the code `modelCheckImpl` chose for
\* the escaping `EvalException` survives. That code is `EC.GENERAL` (exit 255), not the
\* exception's own `TLC_VALUE_ASSERT_FAILED`, because `EvalException` is not an
\* `Assert.TLCRuntimeException`.
ASSUME TLCSet(1, 0)
Init == x = 0
Next == x' = x
Inv == IF TLCGet(1) = 0 THEN TLCSet(1, 1) /\ Assert(FALSE, "boom") ELSE TRUE
====
