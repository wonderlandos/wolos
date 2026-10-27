#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
# Patch + build + install WolKernel. Run as root on a FreeBSD 15.1 host/VM.
# Usage: kernel/build.sh [SRC]   env: WOLOS_OBJ (default /usr/obj), INSTALL=0 to skip installkernel,
#   WOLOS_KERNEL_PATCHES (default kernel/patches): WolKernel patches, applied in name order,
#   WOLOS_EXTRA_KMODS (default "mt76/core mt76/mt7921"): sys/modules dirs GENERIC does not build.
# Needs ~8 GB free for objs. Expected log lines: kernel/README.md.
set -eu
K=$(cd "$(dirname "$0")" && pwd)
SRC=${1:-${WOLOS_SRC:-/usr/src}}
OBJ=${WOLOS_OBJ:-/usr/obj}
P=${WOLOS_KERNEL_PATCHES:-$K/patches}
KCONF=WolKernel
EXTRA=${WOLOS_EXTRA_KMODS-mt76/core mt76/mt7921}
# INSTALL is also make(1)'s install-program variable: read it, then keep it out of make's env.
DO_INSTALL=${INSTALL:-1}; unset INSTALL
DEST=${DESTDIR:-}
[ "$(uname -s)" = FreeBSD ] || { echo "run on FreeBSD (see image/vm/build-in-vm.sh)" >&2; exit 1; }
[ -f "$SRC/sys/conf/newvers.sh" ] || { echo "no source in $SRC: kernel/fetch-src.sh $SRC" >&2; exit 1; }
mkdir -p "$OBJ"
free=$(df -Pm "$OBJ" | awk 'NR==2{print $4}')
[ "$free" -ge 8000 ] || { echo "need 8000 MB free in $OBJ, have $free MB" >&2; exit 2; }
cp "$K/conf/$KCONF" "$SRC/sys/amd64/conf/$KCONF"
cd "$SRC"
# WolKernel patches, in name order. A patch that does not apply stops the build with the tree untouched.
n=0
for p in "$P"/*.patch; do
  [ -f "$p" ] || continue
  if patch -R -s -f --dry-run -p0 < "$p" >/dev/null 2>&1; then echo "patch already applied: ${p##*/}"
  elif patch -s -f --dry-run -p0 < "$p" >/dev/null 2>&1; then patch -s -f -p0 < "$p"; echo "patch applied: ${p##*/}"
  else echo "patch does not apply: ${p##*/} (tree is not the releng/15.1 it was made for)" >&2; exit 3; fi
  n=$((n + 1))
done
echo "patches: $n"
for d in amd64/linux amd64/linux32 arm64/linux i386/linux; do make -s -C "sys/$d" sysent >/dev/null; done
export MAKEOBJDIRPREFIX="$OBJ"
NCPU=$(sysctl -n hw.ncpu)
make -j"$NCPU" buildkernel KERNCONF=$KCONF
KOBJ=$OBJ$(pwd -P)/$(uname -m).$(uname -p)/sys/$KCONF
# No .debug files (~0.5 GB) in the image/target.
if [ "$DO_INSTALL" != 0 ]; then
  make installkernel KERNCONF=$KCONF DESTDIR="$DEST" WITHOUT_KERNEL_SYMBOLS=yes
  # Product name for the loader menu (boot/lua/local.lua), with the kernel's size so a kernel
  # installed over it later (another build, a base update) is not shown under this name.
  echo "$KCONF $(stat -f %z "$DEST/boot/kernel/kernel")" > "$DEST/boot/kernel/ident"
fi
# LinuxKPI Wi-Fi drivers in the 15.1 tree that sys/modules/Makefile does not build (it builds only
# iwlwifi, rtw88, rtw89). Built against this kernel's options, in dependency order (if_mt7921 needs
# mt76_core): after a failure the rest are skipped, so their cards report unsupported (hardware
# catalogue: module not on disk) instead of a module that cannot load.
ok=0 bad=0
xmk() { MAKEOBJDIRPREFIX=$KOBJ/extra make -C "sys/modules/$1" SYSDIR="$(pwd -P)/sys" KERNBUILDDIR="$KOBJ" \
  WITHOUT_DEBUG_FILES=yes KMODDIR=/boot/kernel "$@"; }
[ -f "$KOBJ/opt_global.h" ] || { echo "extra modules skipped: no kernel objdir $KOBJ" >&2; EXTRA=; }
for m in $EXTRA; do
  [ "$bad" = 0 ] || { bad=$((bad + 1)); echo "extra module skipped: $m (an earlier one failed)" >&2; continue; }
  log=$KOBJ/extra-$(echo "$m" | tr / _).log
  if xmk "$m" obj > "$log" 2>&1 && xmk "$m" -j"$NCPU" all >> "$log" 2>&1 &&
     { [ "$DO_INSTALL" = 0 ] || xmk "$m" DESTDIR="$DEST" install >> "$log" 2>&1; }; then
    ok=$((ok + 1)); echo "extra module built: $m"
  else
    bad=$((bad + 1)); echo "extra module FAILED: $m (log: $log)" >&2; tail -n 20 "$log" >&2 || :
  fi
done
echo "extra modules: $ok built, $bad failed"
# wolos_qos kmod (out of tree)
Q=$K/wolos_qos
make -C "$Q" SYSDIR="$SRC/sys" MAKEOBJDIRPREFIX="$OBJ" clean all
if [ "$DO_INSTALL" != 0 ]; then
  mkdir -p "$DEST/boot/modules"
  make -C "$Q" SYSDIR="$SRC/sys" MAKEOBJDIRPREFIX="$OBJ" KMODDIR=/boot/modules DESTDIR="$DEST" install
  # The installed kernel carries what its patches add.
  sh "$K/verify.sh" "$DEST/boot/kernel"
fi
if [ "$DO_INSTALL" = 0 ]; then echo "kernel: $KCONF built, not installed"; else echo "kernel: $KCONF installed in $DEST/boot/kernel"; fi
