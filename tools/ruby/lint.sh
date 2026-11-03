#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Lints every tracked Ruby script (ADR 0041, docs/RULES-ruby.md): `ruby -wc` must parse it with no
# warning, then RuboCop runs the cops tools/ruby/rubocop.yml lists. Why: the same two checks as
# `wol script check`, for CI and before a commit. How: `tools/ruby/lint.sh [file...]` (default:
# tracked *.rb files and files with a ruby shebang, scanner fixtures left out). Exit 1 on a problem.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
if [ $# -eq 0 ]; then
  mapfile -t files < <(git ls-files -z | xargs -0 -r grep -lI -m1 -E '^#!.*\bruby\b' -- 2>/dev/null || true)
  mapfile -t -O "${#files[@]}" files < <(git ls-files '*.rb')
  mapfile -t files < <(printf '%s\n' "${files[@]}" | grep -v '^crates/controller/tests/fixtures/ruby/' | sort -u | grep . || true)
else
  files=("$@")
fi
if [ "${#files[@]}" -eq 0 ]; then
  echo "ruby lint: no Ruby scripts"
  exit 0
fi
command -v ruby >/dev/null || { echo "ruby lint: ruby is not installed" >&2; exit 1; }
fail=0
for f in "${files[@]}"; do
  if ! out=$(ruby -wc "$f" 2>&1) || grep -q 'warning:' <<<"$out"; then
    printf '%s\n' "$out" >&2
    echo "ruby lint: $f does not parse cleanly" >&2
    fail=1
  fi
done
if command -v rubocop >/dev/null; then
  rubocop -c tools/ruby/rubocop.yml --format emacs -- "${files[@]}" || fail=1
else
  echo "ruby lint: rubocop is not installed; cops not run" >&2
  [ "${WOLOS_REQUIRE_RUBOCOP:-0}" = 1 ] && fail=1
fi
[ "$fail" = 0 ] && echo "ruby lint: ok (${#files[@]} files)"
exit "$fail"
