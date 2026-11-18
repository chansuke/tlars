#!/usr/bin/env bash
# Records TLC's parser output (tools/golden/ParseGolden.java) into
# crates/tlars-sany/tests/golden/: parse-examples.txt for the tlaplus/Examples modules listed
# in tools/golden/parse_examples.txt, parse-errors.txt for crates/tlars-sany/tests/parse/*.tla.
# With --all, writes target/parse-all.txt for every module in tlaplus/Examples instead (too
# large to check in; the ignored test `parser::tests::all_examples` compares against it).
# Run inside `nix develop` from the repository root.
set -euo pipefail
mkdir -p crates/tlars-sany/tests/golden target
javac -d target/golden -cp "$TLA2TOOLS_JAR" tools/golden/ParseGolden.java
run() { java -cp "target/golden:$TLA2TOOLS_JAR" tla2sany.parser.ParseGolden "$@"; }
if [[ "${1:-}" == --all ]]; then
  mapfile -t files < <(cd "$TLAPLUS_EXAMPLES" && find . -name '*.tla' | sed 's|^\./||' | LC_ALL=C sort)
  run target/parse-all.txt "$TLAPLUS_EXAMPLES" "${files[@]}"
  exit
fi
mapfile -t files < tools/golden/parse_examples.txt
run crates/tlars-sany/tests/golden/parse-examples.txt "$TLAPLUS_EXAMPLES" "${files[@]}"
mapfile -t files < <(cd crates/tlars-sany/tests/parse && ls *.tla | LC_ALL=C sort)
run crates/tlars-sany/tests/golden/parse-errors.txt crates/tlars-sany/tests/parse "${files[@]}"
