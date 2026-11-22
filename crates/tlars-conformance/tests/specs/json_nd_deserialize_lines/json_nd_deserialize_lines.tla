---- MODULE json_nd_deserialize_lines ----
EXTENDS Json, TLC, Naturals, Sequences
\* `ndJsonDeserialize` reads with `BufferedReader.readLine`, so a line ends at \n, \r or
\* \r\n and the last one needs no terminator, and it skips a line that `String.trim()`
\* empties. The corpus has all of these cases and reaches none of them, for want of the
\* fixture files.
ASSUME PrintT(ndJsonDeserialize("lines.ndjson"))
====
