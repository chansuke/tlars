#!/usr/bin/env bash
# Records what TLC's SANY reports (tools/golden/SemanticGolden.java) into
# crates/tlars-sany/tests/golden/semantic-examples.txt for the tlaplus/Examples specs listed
# in tools/golden/semantic_examples.txt. With --all, writes target/semantic-all.txt for every
# module in tlaplus/Examples as a root instead (too large to check in; the ignored test
# `parser::semantic_tests::semantic_all_examples` compares against it).
# Run inside `nix develop` from the repository root.
set -euo pipefail
DEPTH=8
mkdir -p crates/tlars-sany/tests/golden target
javac -d target/golden -cp "$TLA2TOOLS_JAR" tools/golden/SemanticGolden.java
# One JVM per spec, as TLC runs: interned strings and SANY's statics start fresh for each.
# run OUT ROOT FILE...
run() {
  local out=$1 root=$2; shift 2
  local part=target/semantic-part.txt
  echo "depth $DEPTH" > "$out"
  for f in "$@"; do
    java -cp "target/golden:$TLA2TOOLS_JAR" tla2sany.drivers.SemanticGolden "$part" "$root" "$DEPTH" "$f"
    tail -n +2 "$part" >> "$out"
  done
}
if [[ "${1:-}" == --all ]]; then
  DEPTH=3
  mapfile -t files < <(cd "$TLAPLUS_EXAMPLES" && find . -name '*.tla' | sed 's|^\./||' | LC_ALL=C sort)
  run target/semantic-all.txt "$TLAPLUS_EXAMPLES" "${files[@]}"
  exit
fi
mapfile -t files < tools/golden/semantic_examples.txt
run crates/tlars-sany/tests/golden/semantic-examples.txt "$TLAPLUS_EXAMPLES" "${files[@]}"
