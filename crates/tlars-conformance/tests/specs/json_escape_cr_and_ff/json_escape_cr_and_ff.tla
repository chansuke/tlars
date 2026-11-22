---- MODULE json_escape_cr_and_ff ----
EXTENDS Json, TLC, Naturals
\* gson's REPLACEMENT_CHARS has a short escape for every control character it names, and
\* the corpus only reaches \n, \t, \\ and \". These are the two TLA+ string escapes that
\* reach the other two short forms; the remaining characters below 0x20 would print as
\* \u00xx, which no TLA+ string literal can ask for.
ASSUME PrintT(ToJson("a\rb\fc"))
====
