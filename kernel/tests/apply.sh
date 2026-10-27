#!/bin/sh
# SPDX-License-Identifier: LicenseRef-Wonderland-1.0
# Copyright (c) 2026 Julia Klee. WolOS kernel work under the Wonderland License 1.0: see LICENSES/Wonderland-1.0.md.
# Do the patches apply to what the build fetches? Applies kernel/patches (in name order) to the files
# they touch at REF, and kernel/drm/patches to drm-kmod at the tag kernel/drm/build.sh pins.
# Usage: kernel/tests/apply.sh [REF]   REF default releng/15.1 (the branch fetch-src.sh clones).
# Sparse fetch from GitHub: no source tree needed. Exit 0 = all apply.
set -eu
R=$(cd "$(dirname "$0")/../.." && pwd)
REF=${1:-releng/15.1}
TAG=$(sed -n 's/^TAG=\([^ ]*\).*/\1/p' "$R/kernel/drm/build.sh")
[ -n "$TAG" ] || { echo "no TAG in kernel/drm/build.sh" >&2; exit 2; }
T=$(mktemp -d "${TMPDIR:-/tmp}/wolkernel-apply.XXXXXX"); trap 'rm -rf "$T"' EXIT
get() { mkdir -p "$(dirname "$2")"
  if command -v fetch >/dev/null; then fetch -q -o "$2" "$1"; else curl -sfL -o "$2" "$1"; fi; }
fail=0
run() { # repo ref strip-prefix workdir -p patches...
  repo=$1 ref=$2 strip=$3 w=$4 p=$5; shift 5
  for f in $(awk '/^\+\+\+ /{print $2}' "$@" | sed "s|^$strip||" | sort -u); do
    get "https://raw.githubusercontent.com/$repo/$ref/$f" "$w/$f" || { echo "FAIL fetch $repo@$ref $f"; fail=1; }
  done
  for x in "$@"; do
    if (cd "$w" && patch -s -f "$p" < "$x" >/dev/null); then echo "applies: ${x##*/} ($repo@$ref)"
    else echo "FAIL does not apply: ${x##*/} ($repo@$ref)"; fail=1; fi
  done
}
run freebsd/freebsd-src "$REF" "" "$T/src" -p0 "$R"/kernel/patches/*.patch
run freebsd/drm-kmod "$TAG" "b/" "$T/drm" -p1 "$R"/kernel/drm/patches/*.patch
[ $fail = 0 ] && echo "apply: ok ($REF, drm-kmod $TAG)"; exit $fail
