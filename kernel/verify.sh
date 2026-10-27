#!/bin/sh
# SPDX-License-Identifier: LicenseRef-Wonderland-1.0
# Copyright (c) 2026 Julia Klee. WolOS kernel work under the Wonderland License 1.0: see LICENSES/Wonderland-1.0.md.
# Called by kernel/build.sh after installkernel: DIR = the installed /boot/kernel.
# Fails when a promise of the kernel page is missing from the built kernel (tests/check.sh maps them).
set -eu
D=$1; fail=0
has() { grep -aqF -- "$2" "$D/$1" 2>/dev/null && echo "verify: $1 has $2" || { echo "verify FAILED: $1 lacks $2" >&2; fail=1; }; }
has kernel umtx_abs_timeout_getsbt            # 0001: umtx wait-on-many core (kern_umtx.c)
has linux64.ko linux_futex_waitv              # 0001: futex_waitv(2), Linux syscall 449
has linux_common.ko futex_spin                # 0001/0003: compat.linux.futex_spin
has linux64.ko sysret_fast                    # 0002: compat.linux.sysret_fast
[ "$fail" = 0 ] && echo "verify: kernel ok"; exit "$fail"
