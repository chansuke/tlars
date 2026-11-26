Reference results from `xtask bench run`, one file per engine, version and host:
`<engine>-<version>-<jdk>-w<workers>-<host>.jsonl`. Timings are only comparable on the
same host. `tlc-bc1b506-jdk21-w1-carbon.jsonl` is TLC built by `nix build .#tlc` on
carbon (24 cores), default heap, `-XX:+UseParallelGC`, recorded 2026-11-17.

`tlc-bc1b506-jdk21-w1-chansuke.jsonl` is the same TLC build on chansuke (24 cores), same
settings, recorded 2026-11-25. It has no btree/MCbtree row, which left the suite then;
the carbon file still has one. docs/M3.5.md compares tlars against this file.
