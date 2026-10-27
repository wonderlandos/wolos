#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
# Dry run of kernel/build.sh against a source tree: nothing is built, SRC is not modified.
# Usage: kernel/check.sh [SRC]   (FreeBSD: full checks; other hosts: patches and paths only)
# env: WOLOS_KERNEL_PATCHES, WOLOS_EXTRA_KMODS as kernel/build.sh. Exit 0 = build.sh would get this far.
set -eu
K=$(cd "$(dirname "$0")" && pwd)
SRC=$(cd "${1:-${WOLOS_SRC:-/usr/src}}" && pwd -P)
P=${WOLOS_KERNEL_PATCHES:-$K/patches}
EXTRA=${WOLOS_EXTRA_KMODS-mt76/core mt76/mt7921}
T=$(mktemp -d "${TMPDIR:-/tmp}/wolkernel-check.XXXXXX"); trap 'rm -rf "$T"' EXIT
fail=0
say() { echo "check: $*"; }
bad() { echo "check FAILED: $*" >&2; fail=1; }
# 1. Kernel config: ident is the file name (kern.ident, uname -i).
grep -q '^ident[[:space:]]*WolKernel$' "$K/conf/WolKernel" && say "conf WolKernel: ident WolKernel" || bad "conf: ident"
if [ "$(uname -s)" = FreeBSD ] && [ -f "$SRC/sys/amd64/conf/GENERIC" ]; then
  cp "$K/conf/WolKernel" "$T/WolKernel"
  (cd "$SRC/sys/amd64/conf" && config -d "$T/compile" "$T/WolKernel" >/dev/null) &&
    say "config(8) accepts WolKernel" || bad "config(8) rejects WolKernel"
fi
# 2. WolKernel patches apply in name order (on copies of the files they touch).
n=0
for p in "$P"/*.patch; do
  [ -f "$p" ] || continue
  for f in $(awk '/^\+\+\+ /{print $2}' "$p"); do
    [ -f "$T/src/$f" ] && continue
    [ -f "$SRC/$f" ] || { bad "${p##*/}: $f not in $SRC"; continue; }
    mkdir -p "$T/src/${f%/*}" && cp "$SRC/$f" "$T/src/$f"
  done
  if (cd "$T/src" && patch -s -f -p0 < "$p" >/dev/null 2>&1); then n=$((n + 1)); say "patch applies: ${p##*/}"
  elif (cd "$SRC" && patch -R -s -f --dry-run -p0 < "$p" >/dev/null 2>&1); then n=$((n + 1)); say "patch already in $SRC: ${p##*/}"
  else bad "patch does not apply: ${p##*/}"; fi
done
say "patches: $n"
# 3. Extra modules: directory, Makefile, and (FreeBSD) every source make(1) would need.
for m in $EXTRA; do
  d=$SRC/sys/modules/$m
  [ -f "$d/Makefile" ] || { bad "extra module $m: no $d/Makefile"; continue; }
  if [ "$(uname -s)" = FreeBSD ]; then
    if MAKEOBJDIRPREFIX=$T/obj make -C "$d" -n SYSDIR="$SRC/sys" all > "$T/mk.log" 2>&1; then
      say "extra module $m: make -n ok"
    else bad "extra module $m: make -n failed"; tail -n 5 "$T/mk.log" >&2; fi
  else
    say "extra module $m: Makefile present (make -n needs FreeBSD)"
  fi
done
[ "$fail" = 0 ] && say ok
exit "$fail"
