# Working on tlars

tlars reimplements TLC in Rust and must match it bit for bit. Read
[docs/DESIGN.md](docs/DESIGN.md) for the crates, the decisions, and the table of who owns
each invariant. Read [docs/feature-map.md](docs/feature-map.md) for the names of features,
their milestone, and the module that owns each. Use those names in commits and pull requests.

The checks hold the rules. `crates/tlars-tool/tests/architecture.rs` checks the module
layers, cycles, `thread_local!`, the hash container exception and external dependencies;
`clippy.toml` and `[workspace.lints]` ban the host reads that bypass `Env`, prints outside
`Report`, and lint exceptions without a reason; `scripts/check-escape-hatches.sh` checks
added lines. When one fails, its message names the fix. Change the design to pass it. Do
not silence the check, and do not add to an allowlist that may only shrink.

## Before you call a change done

- Run `scripts/verify`, then read `target/verify/summary.md` and check each
  `docs/verification.md` item it lists.
- Evaluator, checker or SANY change: also run `scripts/verify conformance`.
- Change to `flake.nix`, `flake.lock` or `Cargo.lock`: run `scripts/verify full`.
- Generated files (`@generated`) change only by fixing `crates/xtask` and regenerating.
- Unfinished TLC behaviour is `todo!("M<n>: ...")`, or `EvalError::unimplemented` inside
  the evaluator.

## When a bug is fixed or a review finds something

First ask whether a lint, a type, a test, the architecture test, or a
`docs/verification.md` item can stop it from coming back, and add that. Write a rule here
only when none of them can express it.
