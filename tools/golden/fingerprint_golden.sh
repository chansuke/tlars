#!/usr/bin/env bash
# Records TLC's value fingerprints (tools/golden/FingerprintGolden.java) into
# crates/tlars-tool/tests/golden/fingerprint.txt.
# Run inside `nix develop` from the repository root.
set -euo pipefail
mkdir -p crates/tlars-tool/tests/golden target/golden
javac -d target/golden -cp "$TLA2TOOLS_JAR" tools/golden/FingerprintGolden.java
java -cp "target/golden:$TLA2TOOLS_JAR" FingerprintGolden crates/tlars-tool/tests/golden/fingerprint.txt
