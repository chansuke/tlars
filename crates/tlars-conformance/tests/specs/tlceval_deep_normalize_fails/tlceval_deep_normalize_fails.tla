---- MODULE tlceval_deep_normalize_fails ----
EXTENDS TLC, Naturals
VARIABLE x
\* The cached path deep-normalises what it stores, because every worker shares it. A set of
\* values that cannot be ordered therefore fails inside TLCEval, where the same set on its own
\* is only refused once something compares its elements.
Init == x = 0 /\ TLCEval({1, "a"}) # {}
Next == x' = x
====
