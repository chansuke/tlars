#!/usr/bin/env bash
# Checks that TLC and tlars read each other's `_TLCTrace` files.
#
# Runs the same spec under both engines in directories of their own, then reads both
# trace.bin files with TLC's own ValueInputStream (tools/golden/TraceInterop.java) and with
# tlars (the second run of a spec deserializes the file during constant processing). The gzip
# payload differs - tlars writes stored DEFLATE blocks where zlib writes Huffman codes - so
# what is compared is the value each reader gets and the inflated value stream, not the
# compressed bytes.
#
# Run inside `nix develop` from the repository root. Prints PASS or FAIL per check.
set -u
spec=${1:-tools/golden/trace_interop}
# The model values the cfg declares, in cfg order, for TraceInterop's `ModelValue.mvs`.
model_values=${2:-MV}
name=$(basename "$(ls "$spec"/*.tla)" .tla)
args=(-tool -workers 1 -fp 0 -seed 4 -lncheck final -noGenerateSpecTE)
status=0

cargo build --release -q -p tlars
tlars=$PWD/target/release/tlars
work=$(mktemp -d)
mkdir -p "$work/tlc" "$work/tlars" "$work/reread"
cp "$spec"/*.tla "$spec"/*.cfg "$work/tlc"/
cp "$spec"/*.tla "$spec"/*.cfg "$work/tlars"/

(cd "$work/tlc" && java -Dtlc2.output.MP.noTimestamps=true -cp "$TLA2TOOLS_JAR" tlc2.TLC \
   "${args[@]}" -metadir .meta -config "$name.cfg" "$name.tla" > out.txt 2>&1)
(cd "$work/tlars" && "$tlars" -Dtlc2.output.MP.noTimestamps=true \
   "${args[@]}" -metadir .meta -config "$name.cfg" "$name.tla" > out.txt 2>&1)

check() {
  if [ "$2" = "$3" ]; then
    printf 'PASS %s\n' "$1"
  else
    printf 'FAIL %s\n  tlc:   %s\n  tlars: %s\n' "$1" "$2" "$3"
    status=1
  fi
}

# 1. TLC's reader on both files: the value it gets must be the same.
mkdir -p target/golden
javac -encoding UTF-8 -d target/golden -cp "$TLA2TOOLS_JAR" tools/golden/TraceInterop.java
read_with_tlc() {
  java -cp "target/golden:$TLA2TOOLS_JAR" TraceInterop -mv "$model_values" "$1" | cut -f2-
}
check "TLC reads the file tlars wrote" "$(read_with_tlc "$work/tlc/trace.bin")" \
                                       "$(read_with_tlc "$work/tlars/trace.bin")"

# 2. tlars' reader on TLC's file: the spec deserializes _TLCTraceInputFile during constant
#    processing, so running tlars over TLC's directory must produce the same transcript as
#    running it over its own, and must not stop at an unimplemented construct.
cp "$spec"/*.tla "$spec"/*.cfg "$work/reread"/
cp "$work/tlc/trace.bin" "$work/reread"/
(cd "$work/reread" && "$tlars" -Dtlc2.output.MP.noTimestamps=true \
   "${args[@]}" -metadir .meta -config "$name.cfg" "$name.tla" > out.txt 2>&1)
# The same normalisation the conformance harness compares transcripts with: it folds the
# scratch directory, the pid, the timings and the rates.
normalize() { cargo run --release -q -p xtask -- normalize-transcript "$1" < "$1/out.txt"; }
check "tlars reads the file TLC wrote" "$(normalize "$work/tlars")" "$(normalize "$work/reread")"

# 3. The inflated value streams, side by side. Not a pass or a fail: two differences are
#    known and neither is in the encoding.
#    - A UniqueString's token depends on everything the run interned before it, and tlars
#      does not intern the Java method names TLC's processModuleOverrides does, so its tokens
#      sit lower by the number of those names (tools/golden/intern_dump.sh prints TLC's
#      sequence).
#    - TLC writes a DUMMYVALUE back-reference for a StringValue *object* it has already
#      written, which a spec produces by holding one string in several states. `Value::Str`
#      carries an intern token and no identity, so this port writes the string again (see
#      `HandleTable` in crates/tlars-tool/src/value/stream.rs).
gunzip -c "$work/tlc/trace.bin" > "$work/tlc.raw"
gunzip -c "$work/tlars/trace.bin" > "$work/tlars.raw"
printf 'value stream: tlc %s bytes, tlars %s bytes\n' "$(stat -c%s "$work/tlc.raw")" "$(stat -c%s "$work/tlars.raw")"
diff <(xxd "$work/tlc.raw") <(xxd "$work/tlars.raw") | sed -n '1,30p'

printf 'files: %s\n' "$work"
exit "$status"
