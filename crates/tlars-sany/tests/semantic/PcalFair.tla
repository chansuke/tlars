---- MODULE PcalFair ----
EXTENDS Naturals
(* --fair algorithm Two
variables y = 0;
process P \in 1..2
begin
  A: y := y + self;
end process;
end algorithm; *)
\* BEGIN TRANSLATION (chksum(pcal) = "7c28162a" /\ chksum(tla) = "28d67f47")
VARIABLES pc, y

vars == << pc, y >>

ProcSet == (1..2)

Init == (* Global variables *)
        /\ y = 0
        /\ pc = [self \in ProcSet |-> "A"]

A(self) == /\ pc[self] = "A"
           /\ y' = y + self
           /\ pc' = [pc EXCEPT ![self] = "Done"]

P(self) == A(self)

(* Allow infinite stuttering to prevent deadlock on termination. *)
Terminating == /\ \A self \in ProcSet: pc[self] = "Done"
               /\ UNCHANGED vars

Next == (\E self \in 1..2: P(self))
           \/ Terminating

Spec == /\ Init /\ [][Next]_vars
        /\ WF_vars(Next)

Termination == <>(\A self \in ProcSet: pc[self] = "Done")

\* END TRANSLATION
====
