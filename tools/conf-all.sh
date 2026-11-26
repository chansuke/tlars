#!/usr/bin/env bash
# The conformance targets of the M3 and M4 completion conditions, with the summary line of each.
# Extra arguments pass through, so `tools/conf-all.sh --check` is the same check CI makes
# target by target, and the exit status is non-zero when any target fails.
# Run inside `nix develop` from the repository root.
set -u
: "${TLA_TS_CORPUS:?the dev shell sets this; run inside nix develop}"
status=0
for target in "expressions assume" "expressions init" "expressions next" "expressions invariant" \
              "standard assume" "standard init" "standard next" "standard invariant" \
              "standard model" "checker model" "temporal model" "expressions property"; do
  suite=${target% *}
  context=${target#* }
  printf '%-22s ' "$target"
  out=$(cargo run --release -q -p xtask -- conformance run --suite "$suite" --context "$context" "$@" 2>&1) || status=1
  printf '%s\n' "$out" | grep -E 'pass,|^error' | tail -1
done
exit "$status"
