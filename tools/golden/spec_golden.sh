#!/usr/bin/env bash
# Records TLC's exit status and normalised `-tool` transcript for each spec fixture under
# crates/tlars-conformance/tests/specs/<case>/ into
# crates/tlars-conformance/tests/golden/specs.txt, as
# "<case>\x1f<status>\x1f<transcript with \\ and \n escaped>".
#
# A fixture is a directory holding <case>.tla (plus any module it EXTENDS) and <case>.cfg.
# An `args` file, if present, holds extra TLC arguments, one per line, inserted before
# `-config`; a `-D<prop>=<value>` among them is a JVM argument for TLC, which tlars takes on
# its own command line. The fixtures run with the corpus' own deterministic arguments, so a
# recording here and a corpus recording disagree only where the spec does.
#
# A `blocked` file marks a fixture tlars is expected to refuse rather than match; it holds
# the reason, and the recording is still made so the reason can be checked against TLC.
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
  rm -f "$dir/args" "$dir/blocked"
  args=(-tool -workers 1 -fp 0 -seed 4 -lncheck final -noGenerateSpecTE -metadir "$dir/.meta")
  props=(-Dtlc2.output.MP.noTimestamps=true)
  if [ -f "$case_dir/args" ]; then
    while IFS= read -r a; do
      case "$a" in
        "") ;;
        -D*) props+=("$a") ;;
        *) args+=("$a") ;;
      esac
    done < "$case_dir/args"
  fi
  args+=(-config "$name.cfg" "$name.tla")
  # LC_ALL as `tlars_conformance::RECORDING_LOCALE`: the JVM's strerror text follows it.
  stdout=$(cd "$dir" && LC_ALL=C.UTF-8 java -XX:+UseSerialGC -XX:TieredStopAtLevel=1 -XX:-UsePerfData -Xmx1g -Xss2m \
             "${props[@]}" -cp "$TLA2TOOLS_JAR" tlc2.TLC "${args[@]}" 2>/dev/null)
  status=$?
  # The normalisation the test applies to tlars' output, so both sides are compared the
  # same way: `cargo run -p xtask -- normalize-transcript <directory>` reads stdin.
  transcript=$(printf '%s' "$stdout" | cargo run -q -p xtask -- normalize-transcript "$dir")
  escaped=$(printf '%s' "$transcript" | sed 's/\\/\\\\/g' | awk 'BEGIN{ORS="\\n"} {print}')
  printf '%s\x1f%s\x1f%s\n' "$name" "$status" "$escaped" >> "$out"
  rm -rf "$dir"
done
echo "recorded $(wc -l < "$out") fixtures into $out"
