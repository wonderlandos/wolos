#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# run.sh NAME [ENV=VAL ...] -- [bun flags]
cd /home/ubuntu/wolos/site
B=/tmp/bunx/bin/bun-linux-x64/bun
name=$1; shift; envs=(); while [ "$1" != -- ]; do envs+=("$1"); shift; done; shift
r=(); w=()
for i in 1 2 3 4 5; do
  env "${envs[@]}" /usr/bin/time -o /tmp/bunx/t.txt -f "%M %e" $B "$@" --bun x vite build --outDir /tmp/bunx/out2 --emptyOutDir >/dev/null 2>&1
  r+=($(cut -d' ' -f1 /tmp/bunx/t.txt)); w+=($(cut -d' ' -f2 /tmp/bunx/t.txt))
done
echo "$name rss_MB: $(( $(printf '%s\n' "${r[@]}" | sort -n | sed -n 3p) / 1024 )) wall_s: $(printf '%s\n' "${w[@]}" | sort -n | sed -n 3p)  all_rss: ${r[*]}"
