#!/usr/bin/env bash
# Records TLC's value stream encoding (tools/golden/ValueStreamGolden.java) into
# crates/tlars-tool/tests/golden/value_stream.txt.
# Run inside `nix develop` from the repository root.
set -euo pipefail
mkdir -p crates/tlars-tool/tests/golden target/golden
javac -encoding UTF-8 -d target/golden -cp "$TLA2TOOLS_JAR" tools/golden/ValueStreamGolden.java
java -cp "target/golden:$TLA2TOOLS_JAR" ValueStreamGolden crates/tlars-tool/tests/golden/value_stream.txt
