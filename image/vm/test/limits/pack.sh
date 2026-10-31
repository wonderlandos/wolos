#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Test files for the guest's second disk: pack.sh OUT.tar SWEEP (the built limits_sweep example).
# t/ holds binaries and scripts, after/ the limits under test as /run overrides.
set -e
H=$(cd "$(dirname "$0")" && pwd); S=$H/../../../files/usr/lib; P=$(mktemp -d)
mkdir -p "$P"/t "$P"/after/systemd/system/sysinit.target.wants "$P"/after/sysctl.d
for f in kb comp tabs; do gcc -O2 -static -pthread "$H/$f.c" -o "$P/t/$f"; done
cp "$H"/guest-run.sh "$H"/load.sh "$P"/t/; chmod +x "$P"/t/*
cp "$2" "$P"/t/wol-limits-sweep
for d in session-.scope.d user-.slice.d user.slice.d system.slice.d user@.service.d tmp.mount.d; do
	cp -r "$S/systemd/system/$d" "$P"/after/systemd/system/
done
cp "$S"/systemd/system/wolos-shm-limit.service "$P"/after/systemd/system/
ln -s ../wolos-shm-limit.service "$P"/after/systemd/system/sysinit.target.wants/
cp "$S"/sysctl.d/70-wolos-performance.conf "$P"/after/sysctl.d/
tar cf "$1" -C "$P" t after
rm -rf "$P"
