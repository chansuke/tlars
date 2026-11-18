---- MODULE PcalNoChecksum ----
EXTENDS Naturals, TLC
(* --algorithm Counter
variables x = 0;
begin
  Inc: while x < 3 do
         x := x + 1;
       end while;
  assert x = 3;
end algorithm; *)
\* BEGIN TRANSLATION
VARIABLES pc, x

vars == << pc, x >>

Init == (* Global variables *)
        /\ x = 0
        /\ pc = "Inc"

Inc == /\ pc = "Inc"
       /\ IF x < 3
             THEN /\ x' = x + 1
                  /\ pc' = "Inc"
             ELSE /\ Assert(x = 3, 
                            "Failure of assertion at line 9, column 3.")
                  /\ pc' = "Done"
                  /\ x' = x

(* Allow infinite stuttering to prevent deadlock on termination. *)
Terminating == pc = "Done" /\ UNCHANGED vars

Next == Inc
           \/ Terminating

Spec == Init /\ [][Next]_vars

Termination == <>(pc = "Done")

\* END TRANSLATION
====
