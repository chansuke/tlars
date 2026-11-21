---- MODULE tlcget_all_named_order ----
EXTENDS TLC
VARIABLE x
Keys == <<"s:a", "s:q", "s:ah", "s:k0", "s:k21", "s:b", "s:c", "s:d", "s:e", "s:f",
          "s:g", "s:h", "s:i", "s:j", "s:k", "s:l", "s:m">>
Set(i) == TLCSet(Keys[i], i)
Init == /\ x = 0
        /\ Set(1) /\ Set(2) /\ Set(3) /\ Set(4) /\ Set(5) /\ Set(6)
        /\ Set(7) /\ Set(8) /\ Set(9) /\ Set(10) /\ Set(11) /\ Set(12)
        /\ Set(13) /\ Set(14) /\ Set(15) /\ Set(16) /\ Set(17)
        /\ PrintT(ToString(TLCGet("all:named")))
        /\ PrintT(ToString(DOMAIN TLCGet("all:named")))
Next == x' = x
====
