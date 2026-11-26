#!/usr/bin/env bash
# Rejects added lines that put off a fix or switch a check off without a reason. Only the
# lines a change adds are read, so existing code never fails it. Clippy already rejects
# #[allow], reasonless #[expect], undocumented unsafe and sleeps (Cargo.toml, clippy.toml);
# `xtask gen-fidelity --check` and `xtask jj2rs emit --check` reject hand edits of
# generated files.
#
# usage: scripts/check-escape-hatches.sh --staged     (the index against HEAD)
#        scripts/check-escape-hatches.sh BASE [HEAD]  (BASE to HEAD, or to the working tree)
set -euo pipefail

case "${1:-}" in
--staged) diff=(git diff --cached --no-color --no-ext-diff -U0 --) ;;
"")
  echo "usage: $0 --staged | BASE [HEAD]" >&2
  exit 2
  ;;
*) diff=(git diff --no-color --no-ext-diff -U0 "$@" --) ;;
esac

# Each rule: comment or code, a file glob (`*` also matches `/`), an extended regular
# expression for the added text, and what to do instead.
rules=(
  $'comment\t*\t(^|[^A-Za-z])(TODO|FIXME|HACK|XXX)([^A-Za-z]|$)\tFix it in this change. Unfinished TLC behaviour is todo!("M<n>: <what>") in code (milestones: docs/DESIGN.md); otherwise describe what the code does now as a fact.'
  $'comment\t*\t[Ww]ork-?around|[Ff]or now|[Qq]uick fix|[Bb]and-?aid|[Tt]emporary (fix|hack|patch)\tDo not explain away a workaround. Find the cause and fix it; if TLC itself behaves this way, cite the TLC source line the code reproduces.'
  $'code\t*.rs\t(todo|unimplemented)!\\((\\)|"("|[^MT"]|M[^0-9]|T([^L]|L([^C]|C[^:]))))\tSay what is missing and where it lands: todo!("M5: <TLC behaviour>") with the milestone from docs/DESIGN.md, or todo!("TLC: <class.method>"). Inside the evaluator, return EvalError::unimplemented(..) instead.'
  $'code\t*.rs\t#\\[ignore\\]\tGive the reason: #[ignore = "why it does not run by default"]. A test that fails is fixed, not ignored.'
  $'code\t*.rs\t\\.unwrap\\(\\)\tSay why it cannot fail with .expect("..."), as TLC\'s Java exceptions are named (expect("ArrayIndexOutOfBoundsException")), or return the error.'
)

# The comment part of LINE in FILE, empty when it has none.
comment_of() { # FILE LINE
  case $1 in
  *.rs) [[ $2 == *//* ]] && printf '%s' "//${2#*//}" ;;
  *.sh | *.toml | *.yml | *.nix) [[ $2 =~ (^|[[:space:]])#([^[!]|$) ]] && printf '%s' "#${2#*#}" ;;
  esac
  return 0
}

# Prose, fixtures, recordings and generated files are not checked.
checked() { # FILE
  case $1 in
  docs/* | *.md | *.lock | *.txt | *.tla | *.cfg | */tests/* | crates/xtask/* | tools/*) return 1 ;;
  crates/tlars-sany/src/parser/generated.rs | crates/tlars-msg/src/catalog.rs | crates/tlars-tool/src/fp64_polys.rs) return 1 ;;
  crates/tlars-tool/src/modules/java_overrides.rs | crates/tlars-sany/src/error_codes.rs | crates/tlars-sany/src/standard_modules.rs) return 1 ;;
  scripts/check-escape-hatches.sh) return 1 ;;
  esac
  return 0
}

found=0
file=""
line=0
while IFS= read -r row; do
  case $row in
  '+++ b/'*)
    file=${row#+++ b/}
    continue
    ;;
  '+++ '* | '--- '*) continue ;;
  '@@ '*)
    # @@ -a,b +c,d @@: the next added line is line c of the new file.
    line=${row#*+}
    line=${line%%[, ]*}
    continue
    ;;
  '+'*) ;;
  *) continue ;;
  esac
  text=${row#+}
  if checked "$file"; then
    comment=$(comment_of "$file" "$text")
    for rule in "${rules[@]}"; do
      IFS=$'\t' read -r kind glob pattern advice <<<"$rule"
      # shellcheck disable=SC2053 # the pattern is meant to glob
      [[ $file == $glob ]] || continue
      subject=$text
      [ "$kind" = comment ] && subject=$comment
      if [ -n "$subject" ] && grep -Eq -- "$pattern" <<<"$subject"; then
        printf '%s:%s: %s\n    %s\n' "$file" "$line" "${text#"${text%%[![:space:]]*}"}" "$advice" >&2
        found=$((found + 1))
      fi
    done
  fi
  line=$((line + 1))
done < <("${diff[@]}")

if ((found > 0)); then
  echo "$found added line(s) put off a fix or switch off a check without a reason." >&2
  exit 1
fi
