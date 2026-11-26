---- MODULE operator_argument_reads_state ----
\* An operator handed to a Java override evaluates in the states it was built in, so a test
\* that reads a variable through a LET-bound parameter sees the current state.
EXTENDS Sequences, Naturals, TLC, Bags
VARIABLES pending, seen
Unseen(s) ==
    LET keep(c) == c \notin s
    IN  SelectSeq(pending, keep)
Sorted(s) ==
    LET lt(a, b) == (a \notin s /\ b \in s) \/ ((a \in s) = (b \in s) /\ a < b)
    IN  SortSeq(pending, lt)
Counted(s) ==
    LET f(c) == IF c \in s THEN 1 ELSE 0
    IN  BagOfAll(f, SetToBag({1, 2, 3}))
Init == pending = <<1, 2, 3>> /\ seen = {2}
Next == /\ Len(Unseen(seen)) > 0
        /\ pending' = Unseen(seen)
        /\ seen' = seen \cup {Head(pending)}
Inv == /\ Len(Sorted(seen)) = Len(pending)
       /\ BagToSet(Counted(seen)) \subseteq {0, 1}
====
