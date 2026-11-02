#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Every decision record has the four parts a reader looks for (docs/STANDARDS.md §5): Status, Context,
# Decision, Consequences, each as a "Name:" line or a "## Name" heading. Why: an ADR without a
# Consequences part hides what the decision costs, and one without a Status cannot be superseded.
# How: `tools/check-adrs.sh [dir]` (default docs/adr next to this repo's root); prints one line per
# missing part and exits 1 if any is missing, 0 and "adrs: ok <n>" otherwise.
set -euo pipefail
dir=${1:-"$(cd "$(dirname "$0")/.." && pwd)/docs/adr"}
fail=0 n=0
for f in "$dir"/[0-9][0-9][0-9][0-9]-*.md; do
  n=$((n + 1))
  for part in Status Context Decision Consequences; do
    grep -qE "^(#+ *)?(\*\*)?$part(\*\*)?(:|\s|$)" "$f" || { echo "adr: $(basename "$f") lacks $part" >&2; fail=1; }
  done
done
[ "$n" -gt 0 ] || { echo "adr: no records in $dir" >&2; exit 1; }
[ "$fail" = 0 ] && echo "adrs: ok $n"
exit "$fail"
