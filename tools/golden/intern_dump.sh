#!/usr/bin/env bash
# Prints TLC's whole `UniqueString` intern table after a run of a spec, as
# "INTERN <token> <string>" in token order (tools/golden/InternDump.java).
#
# It is the TLC side of the comparison `xtask intern-dump-diff` is planned to make: the token
# order decides how strings, record fields and model values sort everywhere, so a divergence
# here is a fidelity bug even when no spec's output shows it yet.
#
# Usage, inside `nix develop` from the repository root:
#   tools/golden/intern_dump.sh <directory> <spec name> [extra TLC arguments]
set -euo pipefail
dir=$1
name=$2
shift 2
mkdir -p target/golden
javac -encoding UTF-8 -d target/golden -cp "$TLA2TOOLS_JAR" tools/golden/InternDump.java
cd "$dir"
java -Dtlc2.output.MP.noTimestamps=true -cp "$OLDPWD/target/golden:$TLA2TOOLS_JAR" InternDump \
  "$@" -config "$name.cfg" "$name.tla" 2>&1 >/dev/null | grep '^INTERN'
