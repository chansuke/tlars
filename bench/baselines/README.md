Reference results from `xtask bench run`, one file per engine, version and host:
`<engine>-<version>-<jdk>-w<workers>-<host>.jsonl`. Timings are only comparable on the
same host. `tlc-bc1b506-jdk21-w1-carbon.jsonl` is TLC built by `nix build .#tlc` on
carbon (24 cores), default heap, `-XX:+UseParallelGC`, recorded 2026-11-17.
