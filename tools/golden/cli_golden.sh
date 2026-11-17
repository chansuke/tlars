#!/usr/bin/env bash
# Records TLC's stdout and exit status for each argument vector in cli_cases.txt
# (tab-separated arguments, one case per line) into crates/tlars-check/tests/golden/cli.txt,
# as "<args>\x1f<status>\x1f<stdout with \\ and \n escaped>".
# Run inside `nix develop` from the repository root.
set -u
out=crates/tlars-check/tests/golden/cli.txt
: > "$out"
dir=$(mktemp -d)
while IFS= read -r line; do
  IFS=$'\t' read -r -a args <<< "$line"
  stdout=$(cd "$dir" && tlc "${args[@]}" 2>/dev/null)
  status=$?
  # Escape backslashes and newlines so each case is one line.
  escaped=$(printf '%s' "$stdout" | sed 's/\\/\\\\/g' | awk 'BEGIN{ORS="\\n"} {print}')
  printf '%s\x1f%s\x1f%s\n' "$line" "$status" "$escaped" >> "$out"
done < tools/golden/cli_cases.txt
rm -rf "$dir"
