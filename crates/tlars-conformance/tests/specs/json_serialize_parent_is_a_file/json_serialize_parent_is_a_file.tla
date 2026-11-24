---- MODULE json_serialize_parent_is_a_file ----
EXTENDS Json, TLC, Naturals
\* `JsonSerialize` runs `new File(path).getParentFile().mkdirs()` and ignores its answer, so
\* a parent that is an ordinary file only shows up in the `FileWriter`'s own failure. The
\* corpus reaches a missing file and a directory target; this reaches the third errno, and
\* with it the rule that the path is reported as the spec wrote it rather than resolved.
ASSUME JsonSerialize("json_serialize_parent_is_a_file.tla/out.json", <<1>>)
====
