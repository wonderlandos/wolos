#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# rsh start-up and size: N runs of `rsh -c nil` and of a one-line script; median and p95 in ms.
# Usage: sh tests/bench.sh path/to/rsh [runs]
set -eu
rsh=$1
runs=${2:-200}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
printf '#!/usr/bin/rsh\necho ok >/dev/null\n' >"$tmp/one.rsh"
measure() {
	i=0
	: >"$tmp/t"
	while [ "$i" -lt "$runs" ]; do
		a=$(date +%s%N)
		"$@" >/dev/null
		b=$(date +%s%N)
		echo $(((b - a) / 1000)) >>"$tmp/t"
		i=$((i + 1))
	done
	sort -n "$tmp/t" | awk -v n="$runs" '{v[NR]=$1} END {printf "median %.2f ms  p95 %.2f ms\n", v[int(n/2)]/1000, v[int(n*0.95)]/1000}'
}
printf 'size     %s bytes\n' "$(wc -c <"$rsh")"
printf 'rsh -c nil       '
measure "$rsh" -c nil
printf 'one-line script  '
measure "$rsh" "$tmp/one.rsh"
