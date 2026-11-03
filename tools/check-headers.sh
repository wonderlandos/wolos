#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Every shipped script has the header docs/STANDARDS.md §2 asks for: a shebang, the SPDX line on line
# 2 and at least one comment line of purpose before the first command. Why: a root helper under
# /usr/lib/wolos with no header is a script nobody can audit. How: `tools/check-headers.sh [file...]`
# (default: files/usr/lib/wolos/* and desktop/overlay/usr/bin/*); also parses each with `sh -n`,
# `bash -n` or `ruby -wc` after its shebang (ADR 0041). One line per problem on stderr, exit 1 if
# any.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
if [ $# -eq 0 ]; then
  set -- "$root"/files/usr/lib/wolos/* "$root"/desktop/overlay/usr/bin/*
fi
fail=0
for f in "$@"; do
  [ -f "$f" ] || continue
  l1=$(sed -n 1p "$f")
  case $l1 in
    '#!/bin/sh'* | '#!/usr/bin/env sh'*) sh -n "$f" || fail=1 ;;
    '#!/bin/bash'* | '#!/usr/bin/env bash'*) bash -n "$f" || fail=1 ;;
    '#!/usr/bin/ruby'* | '#!/usr/bin/env ruby'*)
      if command -v ruby >/dev/null; then ruby -wc "$f" >/dev/null || fail=1
      else echo "header: $f: ruby not installed, syntax not checked" >&2; fi ;;
    '#!'*) echo "header: $f: shebang $l1 is not sh, bash or ruby" >&2; fail=1; continue ;;
    *) continue ;; # not a script (a unit, a config)
  esac
  sed -n 2p "$f" | grep -q '^# SPDX-License-Identifier: ' || { echo "header: $f: line 2 is not the SPDX line" >&2; fail=1; }
  sed -n 3p "$f" | grep -q '^# ' || { echo "header: $f: line 3 is not a comment saying what the script is for" >&2; fail=1; }
done
[ "$fail" = 0 ] && echo "headers: ok"
exit "$fail"
