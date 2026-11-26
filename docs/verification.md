# Verification

`scripts/verify` is the one entry point. It writes a log per step and
`target/verify/summary.md` (`summary-<mode>.md` for `conformance` and `bench`), which ends with the items below that the change touches.
CI runs `scripts/verify check` and `scripts/verify conformance`.

| Mode | Steps |
|---|---|
| `check` (default) | `cargo fmt --check`, clippy with `-D warnings`, `scripts/check-escape-hatches.sh` on the added lines, `xtask gen-fidelity --check` and `xtask jj2rs emit --check`, `cargo test --profile ci` |
| `conformance` | `tools/conf-all.sh --check`: the ten corpus targets against their `*.passing` lists |
| `full` | `check`, `conformance`, `nix build`, and every golden recorded again from the pinned TLC, which must leave `crates/*/tests/golden` unchanged |
| `bench` | `xtask bench run --engine tlars`; compare with `bench/baselines/` by `xtask bench compare` |

## Checklist

The steps cannot see these; check each one the summary lists.

- cfg parsing changed: a spec fixture under `crates/tlars-conformance/tests/specs/` shows
  the new behaviour, recorded with `tools/golden/spec_golden.sh`.
- A `*.passing` list changed: it only gained cases.
- The CLI or `Env` changed: a case in `tools/golden/cli_cases.txt` covers it, recorded with
  `tools/golden/cli_golden.sh`.
- A value class changed: the fingerprint, value stream and print goldens still pass, and the
  Java golden generator has a case for any output that differs from TLC.
- The checker's concurrency changed: `runs_in_one_process_do_not_share_state` passes and
  `scripts/verify bench` shows no slowdown.
- A generator in `crates/xtask` changed: its output is regenerated and committed in the
  same change.
- `flake.nix`, `flake.lock` or `Cargo.lock` changed: `scripts/verify full` passes.
