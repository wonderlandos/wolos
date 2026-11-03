# shellcheck shell=sh
# SPDX-License-Identifier: BSD-2-Clause
# The tutorial's programs (docs/modules/wolkit-tutorial.md) must compile: every fenced crystal block
# that starts with `require "wolkit"` is written to a file and type-checked against this checkout's
# shards, so the tutorial cannot drift from the code. A script because specs may not start the
# compiler (RCR-021).
set -eu
cd "$(dirname "$0")/../.."
doc=../../../docs/modules/wolkit-tutorial.md
if [ ! -f "$doc" ]; then
  echo "skip: $doc not found"
  exit 0
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
awk -v dir="$tmp" '
  /^```crystal$/ { inside = 1; n++; buf = ""; next }
  /^```/ && inside {
    if (buf ~ /^require "wolkit"/) { file = dir "/program" n ".cr"; printf "%s", buf > file; close(file) }
    inside = 0; next
  }
  inside { buf = buf $0 "\n" }
' "$doc"
shards=$(cd ../ && pwd)
system_path=$(crystal env CRYSTAL_PATH)
path="$shards:$system_path"
count=0
failed=0
for program in "$tmp"/program*.cr; do
  [ -f "$program" ] || continue
  count=$((count + 1))
  if (cd "$tmp" && CRYSTAL_PATH="$path" crystal build --no-codegen --error-on-warnings "$program" 2>&1); then
    echo "ok   $(basename "$program")"
  else
    echo "FAIL $(basename "$program")"
    failed=1
  fi
done
if [ "$count" -lt 3 ]; then
  echo "FAIL: found $count programs in the tutorial, expected at least 3"
  failed=1
fi
exit "$failed"
