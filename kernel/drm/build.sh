#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
# WolOS i915kms: drm-kmod at the exact tag of pkg drm-612-kmod + kernel/drm/patches (ADR 0008).
# env: WOLOS_DRM_PATCHES (default kernel/drm/patches).
# Usage: kernel/drm/build.sh SRC OUT   (FreeBSD 15.1, root). SRC = tree from kernel/fetch-src.sh.
# OUT gets boot/modules/{dmabuf,ttm,drm,i915kms}.ko; image/build.sh tars it into the offline repo.
# amdgpu/radeon stay from pkg: same tag, so drm.ko keeps their ABI.
set -eu
K=$(cd "$(dirname "$0")" && pwd)
SRC=$1 OUT=$2
P=${WOLOS_DRM_PATCHES:-$K/patches}
TAG=drm_v6.12.85_3   # graphics/drm-612-kmod 2026Q4 (pkg 6.12.85.1501000_2)
SUM=664bc6d77ddc0e103d523563a804644aca14b46d8a3c1af268ea79846139ef60
DL=${WOLOS_DL:-/work/dl}; W=${WOLOS_DRM_WORK:-/work/drm}
[ "$(uname -s)" = FreeBSD ] || { echo "run on FreeBSD" >&2; exit 1; }
[ -f "$SRC/sys/conf/kern.opts.mk" ] || { echo "no kernel source in $SRC" >&2; exit 1; }
mkdir -p "$DL"; T=$DL/freebsd-drm-kmod-$TAG.tar.gz
[ "$(sha256 -q "$T" 2>/dev/null)" = "$SUM" ] || fetch -o "$T" "https://codeload.github.com/freebsd/drm-kmod/tar.gz/$TAG"
[ "$(sha256 -q "$T")" = "$SUM" ] || { echo "drm-kmod tarball sha256 mismatch" >&2; exit 2; }
rm -rf "$W"; mkdir -p "$W"; tar -C "$W" --strip-components 1 -xzf "$T"
# WolOS drm patches, in name order; one that does not apply stops the build (fresh tree each run).
n=0
for p in "$P"/*.patch; do
  [ -f "$p" ] || continue
  (cd "$W" && patch -s -f -p1 < "$p") || { echo "drm patch does not apply: ${p##*/} (drm-kmod is not $TAG)" >&2; exit 3; }
  echo "drm patch applied: ${p##*/}"; n=$((n + 1))
done
echo "drm patches: $n"
export MAKEOBJDIRPREFIX=$W/obj; mkdir -p "$MAKEOBJDIRPREFIX"
mk() { make -C "$W" SYSDIR="$SRC/sys" KMODS="dmabuf ttm drm i915" WITHOUT_DEBUG_FILES=yes "$@"; }
mk obj >/dev/null
mk -j"$(sysctl -n hw.ncpu)"
mkdir -p "$OUT/boot/modules"
mk install DESTDIR="$OUT" KMODDIR=/boot/modules NO_XREF=1
ls "$OUT/boot/modules"
# The built modules carry what the patches add.
sh "$K/verify.sh" "$OUT/boot/modules"
rm -rf "$W"
