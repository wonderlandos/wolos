#!/bin/sh
# SPDX-License-Identifier: LicenseRef-Wonderland-1.0
# Copyright (c) 2026 Julia Klee. WolOS kernel work under the Wonderland License 1.0: see LICENSES/Wonderland-1.0.md.
# Called by kernel/drm/build.sh: DIR = OUT/boot/modules with the built i915kms.ko.
set -eu
D=$1; fail=0
has() { grep -aqF -- "$2" "$D/$1" 2>/dev/null && echo "verify: $1 has $2" || { echo "verify FAILED: $1 lacks $2" >&2; fail=1; }; }
has i915kms.ko wol_deadline_boost             # 0001: frame-deadline boost
has i915kms.ko wol_waitboost                  # 0001: waitboost policy (2 = foreground only)
has i915kms.ko wol_fg_pid                     # 0001: frontmost pid from wold
has i915kms.ko i915/vbt.bin                   # 0002: VBT file fallback (passthrough)
[ "$fail" = 0 ] && echo "verify: i915kms ok"; exit "$fail"
