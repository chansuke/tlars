#!/usr/bin/env bash
# Records TLC's exit status and normalised `-tool` transcript for each spec fixture under
# crates/tlars-conformance/tests/specs/<case>/ into
# crates/tlars-conformance/tests/golden/specs.txt, as
# "<case>\x1f<status>\x1f<transcript with \\ and \n escaped>".
#
# A fixture is a directory holding <case>.tla (plus any module it EXTENDS) and <case>.cfg.
# An `args` file, if present, holds extra TLC arguments, one per line, inserted before
# `-config`. The fixtures run with the corpus' own deterministic arguments, so a recording
# here and a corpus recording disagree only where the spec does.
#
# Run inside `nix develop` from the repository root.
set -u
specs=crates/tlars-conformance/tests/specs
out=crates/tlars-conformance/tests/golden/specs.txt
mkdir -p "$(dirname "$out")"
: > "$out"
for case_dir in "$specs"/*/; do
  name=$(basename "$case_dir")
  dir=$(mktemp -d)
  cp "$case_dir"/* "$dir"/
  rm -f "$dir/args"
  args=(-tool -workers 1 -fp 0 -seed 4 -lncheck final -noGenerateSpecTE -metadir "$dir/.meta")
  if [ -f "$case_dir/args" ]; then
    while IFS= read -r a; do [ -n "$a" ] && args+=("$a"); done < "$case_dir/args"
  fi
  args+=(-config "$name.cfg" "$name.tla")
  stdout=$(cd "$dir" && java -XX:+UseSerialGC -XX:TieredStopAtLevel=1 -XX:-UsePerfData -Xmx1g -Xss2m \
             -Dtlc2.output.MP.noTimestamps=true -cp "$TLA2TOOLS_JAR" tlc2.TLC "${args[@]}" 2>/dev/null)
  status=$?
  # The normalisation the test applies to tlars' output, so both sides are compared the
  # same way: `cargo run -p xtask -- normalize-transcript <directory>` reads stdin.
  transcript=$(printf '%s' "$stdout" | cargo run -q -p xtask -- normalize-transcript "$dir")
  escaped=$(printf '%s' "$transcript" | sed 's/\\/\\\\/g' | awk 'BEGIN{ORS="\\n"} {print}')
  printf '%s\x1f%s\x1f%s\n' "$name" "$status" "$escaped" >> "$out"
  rm -rf "$dir"
done
echo "recorded $(wc -l < "$out") fixtures into $out"
