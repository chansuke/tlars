---- MODULE enabled_lambda_argument ----
VARIABLE x
Op(F(_)) == ENABLED F(1)
Init == x = 0
Next == x' = x
Inv == Op(LAMBDA a : x' = a)
====
