---- MODULE PcalOptions ----
EXTENDS Naturals
\* PlusCal options (-termination, wf)
(* --algorithm Opt {
  variables z = 0;
  { L: z := z + 1; }
} *)
\* BEGIN TRANSLATION (chksum(pcal) = "cacf018e" /\ chksum(tla) = "ed4ecd79")
VARIABLES pc, z

vars == << pc, z >>

Init == (* Global variables *)
        /\ z = 0
        /\ pc = "L"

L == /\ pc = "L"
     /\ z' = z + 1
     /\ pc' = "Done"

(* Allow infinite stuttering to prevent deadlock on termination. *)
Terminating == pc = "Done" /\ UNCHANGED vars

Next == L
           \/ Terminating

Spec == /\ Init /\ [][Next]_vars
        /\ WF_vars(Next)

Termination == <>(pc = "Done")

\* END TRANSLATION
====
