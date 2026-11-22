---- MODULE tlceval_enabled_branch_context ----
EXTENDS TLC, Naturals
VARIABLE x
\* ENABLED evaluates its action under `Context.branch(c)`, and `Context.branch(Context.Empty)`
\* is the one `BaseBranch` instance that `isDeepEmpty` accepts. So the constant argument of
\* TLCEval inside ENABLED is still cached on its node, and the Print runs once over the whole
\* search rather than once per state.
Act == TLCEval(Print("once", TRUE)) /\ x' = x + 1
Init == x = 0
Next == ENABLED Act /\ x' = (x + 1) % 3
====
