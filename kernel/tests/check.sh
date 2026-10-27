#!/bin/sh
# SPDX-License-Identifier: LicenseRef-Wonderland-1.0
# Copyright (c) 2026 Julia Klee. WolOS kernel work under the Wonderland License 1.0: see LICENSES/Wonderland-1.0.md.
# Patch contract: every patch and fixture is present, each carries the marker the image relies on, and
# every promise of the kernel page maps to a patch and to the marker verify.sh finds in the build.
set -eu
R=$(cd "$(dirname "$0")/../.." && pwd); fail=0
has() { grep -qF -- "$2" "$R/$1" || { echo "FAIL $1 lacks: $2"; fail=1; }; }
for f in kernel/patches/0001-linux-futex_waitv.patch kernel/patches/0002-linux-sysret-fast-return.patch \
         kernel/patches/0003-linux-futex-wait-spin.patch kernel/patches/tests/futex_waitv.c \
         kernel/patches/tests/syscall_regs.c kernel/wolos_qos/wolos_qos.c; do
  [ -f "$R/$f" ] || { echo "FAIL missing $f"; fail=1; }
done
P=kernel/drm/patches/0001-i915-deadline-boost-foreground-waitboost.patch
has $P 'module_param_named(wol_fg_pid'
has $P 'intel_rps_wol_waiter_may_boost()'
# VM passthrough: no OpRegion/ROM reaches the guest, so the panel stays dark without a VBT.
V=kernel/drm/patches/0002-i915-vbt-file-fallback.patch
has $V 'firmware_get_vbt(display, WOL_VBT_FALLBACK, sizep)'
has $V '"i915/vbt.bin"'
promise() { # what | patch file | marker in patch | verify script | marker in build
  has "$2" "$3"; grep -qF -- "$5" "$R/$4" || { echo "FAIL $4 does not check: $5"; fail=1; }; }
promise "Windows games' wait-on-many call" kernel/patches/0001-linux-futex_waitv.patch 'linux_futex_waitv(' kernel/verify.sh linux_futex_waitv
promise "Linux apps returning from the kernel" kernel/patches/0002-linux-sysret-fast-return.patch 'linux_sysret_fast' kernel/verify.sh sysret_fast
promise "Waiting on a busy lock" kernel/patches/0003-linux-futex-wait-spin.patch 'linux_futex_spin != 0' kernel/verify.sh futex_spin
promise "Graphics chip boost" $P 'module_param_named(wol_waitboost' kernel/drm/verify.sh wol_waitboost
promise "A frame about to be late" $P '.set_deadline = i915_fence_set_deadline' kernel/drm/verify.sh wol_deadline_boost
for want in '"$P"/*.patch' '$K/verify.sh' 'exit 3'; do
  grep -qF -- "$want" "$R/kernel/build.sh" || { echo "FAIL kernel/build.sh lacks: $want"; fail=1; }
  grep -qF -- "$want" "$R/kernel/drm/build.sh" || { echo "FAIL kernel/drm/build.sh lacks: $want"; fail=1; }
done
[ $fail = 0 ] && echo "patches: ok"; exit $fail
