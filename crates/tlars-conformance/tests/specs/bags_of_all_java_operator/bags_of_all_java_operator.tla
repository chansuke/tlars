---- MODULE bags_of_all_java_operator ----
EXTENDS Bags, TLC, Naturals, Sequences
VARIABLE x
\* `BagOfAll`'s first argument is an operator, and a `MethodValue` is an `OpValue` too, so
\* an operator a Java class overrides may stand there and is applied like any other. The
\* corpus only ever passes a LAMBDA or a definition of the module itself.
Init == x = TRUE
Next == UNCHANGED x
Inv == x /\ BagOfAll(Len, <<1, 2>> :> 2 @@ <<3>> :> 3) = (2 :> 2 @@ 1 :> 3)
====
