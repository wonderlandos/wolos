#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# pin-shards.sh COMMIT [ROOT]: move the in-tree curated shards to one wolos commit (wolos-shards
# README.md "Shards still in the WolOS tree"). It rewrites three files so they keep agreeing: the
# `commit:` of the cryswol and wolkit rows of wolos-shards/curated.yml, the cryswol `commit:` pin
# in wolos-crystal/shards/wolkit/shard.yml, and that shard's shard.lock (regenerated, with
# cryswol's version from its shard.yml in the tree). Why: gen_index.cr check refuses a list whose
# wolkit pin differs from the cryswol row. The release runbook runs it before the wolos content
# commit, so the pin names the previous release commit. Running it twice changes nothing.
# ROOT defaults to this checkout (the test passes a temporary copy). Exit 2 on a bad argument,
# 1 when a file is missing.
set -eu
commit=${1:-}
root=${2:-$(cd "$(dirname "$0")/../.." && pwd)}
case $commit in
  *[!0-9a-f]* | '') echo "usage: pin-shards.sh COMMIT [ROOT] (COMMIT: 40 lowercase hex digits)" >&2; exit 2 ;;
esac
if [ "${#commit}" -ne 40 ]; then
  echo "usage: pin-shards.sh COMMIT [ROOT] (COMMIT: 40 lowercase hex digits)" >&2
  exit 2
fi
curated=$root/wolos-shards/curated.yml
yml=$root/wolos-crystal/shards/wolkit/shard.yml
lock=$root/wolos-crystal/shards/wolkit/shard.lock
cryswol=$root/wolos-crystal/shards/cryswol/shard.yml
for f in "$curated" "$yml" "$cryswol"; do
  [ -f "$f" ] || { echo "pin-shards.sh: $f is missing" >&2; exit 1; }
done

# replace FILE AWK-PROGRAM: run the program over FILE with `c` set, write back only on a change.
replace() {
  tmp=$1.pin.$$
  awk -v c="$commit" "$2" "$1" > "$tmp"
  if cmp -s "$tmp" "$1"; then rm -f "$tmp"; else cat "$tmp" > "$1" && rm -f "$tmp"; fi
}

# The awk programs are single-quoted on purpose: their $3 and $0 are awk's, not the shell's.
# shellcheck disable=SC2016
# curated.yml: the commit line of the cryswol and wolkit rows (a row starts at `  - name:`).
replace "$curated" '
  /^  - name: / { row = $3 }
  /^    commit: / && (row == "cryswol" || row == "wolkit") { print "    commit: " c; next }
  { print }'

# shellcheck disable=SC2016
# wolkit shard.yml: the commit line inside `dependencies:` > `cryswol:`.
replace "$yml" '
  /^[^ #]/ { deps = ($0 ~ /^dependencies:/); dep = "" }
  deps && /^  [a-z]/ { dep = $1 }
  deps && dep == "cryswol:" && /^    commit: / { print "    commit: " c; next }
  { print }'

git=$(awk '/^  cryswol:/ { d = 1; next } /^  [a-z]/ { d = 0 } d && /^    git: / { print $2; exit }' "$yml")
version=$(sed -n 's/^version: *//p' "$cryswol")
if [ -z "$git" ] || [ -z "$version" ]; then
  echo "pin-shards.sh: no cryswol git: in $yml or no version in $cryswol" >&2
  exit 1
fi
want=$(printf 'version: 2.0\nshards:\n  cryswol:\n    git: %s\n    version: %s+git.commit.%s\n' "$git" "$version" "$commit")
if [ ! -f "$lock" ] || [ "$(cat "$lock")" != "$want" ]; then
  printf '%s\n' "$want" > "$lock"
fi
echo "pinned cryswol and wolkit at $commit"
