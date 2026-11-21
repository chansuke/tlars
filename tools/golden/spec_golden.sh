#!/usr/bin/env bash
# Records TLC's `-tool` transcript and exit status for each case under
# crates/tlars-check/tests/specs/<case>/ into crates/tlars-check/tests/golden/specs/<case>.txt.
#
# A case directory holds the root module Spec.tla, its Spec.cfg, any further modules, and an
# optional `args` file with extra leading arguments (one line, space separated). The golden
# file starts with "exit <status>" and then holds the transcript with the run directory
# rewritten to <dir>, which crates/tlars-check/tests/spec_golden.rs normalizes the same way as
# the conformance harness.
#
# Run inside `nix develop` from the repository root, optionally with case names to re-record.
set -u
cases=crates/tlars-check/tests/specs
out=crates/tlars-check/tests/golden/specs
mkdir -p "$out"
names=("$@")
if [ ${#names[@]} -eq 0 ]; then
  names=()
  for d in "$cases"/*/; do names+=("$(basename "$d")"); done
fi
for name in "${names[@]}"; do
  dir=$(mktemp -d)
  cp "$cases/$name"/* "$dir/"
  args=()
  if [ -f "$dir/args" ]; then
    IFS=' ' read -r -a args < "$dir/args"
    rm "$dir/args"
  fi
  transcript=$(cd "$dir" && tlc "${args[@]}" -tool -workers 1 -fp 0 -seed 4 -lncheck final \
    -noGenerateSpecTE -config Spec.cfg Spec.tla 2>/dev/null)
  status=$?
  { printf 'exit %s\n' "$status"; printf '%s\n' "$transcript" | sed "s|$dir|<dir>|g"; } > "$out/$name.txt"
  rm -rf "$dir"
done
