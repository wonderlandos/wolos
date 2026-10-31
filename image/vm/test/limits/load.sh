#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Legitimate heavy use for the limits test: make -j16 over 1500 small C files while a browser-like
# tree (40 processes of 100 threads, 64 MB each) stays alive. Prints the build time.
d=$(mktemp -d /home/wol/build.XXXX); cd "$d"
for i in $(seq 1500); do
	printf '#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\nint f%d(int x){char b[64];snprintf(b,64,"%%d",x);return atoi(b)+%d;}\n' "$i" "$i" > "f$i.c"
done
printf 'SRC=$(wildcard *.c)\nall: $(SRC:.c=.o)\n%%.o: %%.c\n\tgcc -O2 -c $< -o $@\n' > Makefile
/run/kbomb/t/tabs 40 100 64 &
tabs=$!
sleep 3
t0=$(date +%s%N)
make -j16 -s; rc=$?
echo "B make-j16 rc=$rc ms=$((($(date +%s%N) - t0) / 1000000))"
kill -0 "$tabs" && echo "B tabs alive=$(pgrep -c -f /run/kbomb/t/tabs)" || echo "B tabs DEAD"
pkill -f /run/kbomb/t/tabs
rm -rf "$d"
exit "$rc"
