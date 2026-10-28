#!/bin/sh
# linux-wolos prepare step (PKGBUILD prepare()): SRC BASECONFIG FRAGMENT.
# 1. Extra patches from $WOLOS_OVERLAY/kernel/linux/patches when present, in name order (patch -Np1, no fuzz).
#    A patch that does not apply stops with exit 3.
# 2. BASECONFIG (Arch config.x86_64) + FRAGMENT (kernel/conf/wolos.config), then make olddefconfig.
set -eu
SRC=${1:?SRC}; BASE=${2:?BASECONFIG}; FRAG=${3:?FRAGMENT}
BASE=$(cd "$(dirname "$BASE")" && pwd)/$(basename "$BASE")
FRAG=$(cd "$(dirname "$FRAG")" && pwd)/$(basename "$FRAG")
cd "$SRC"
n=0
for p in "${WOLOS_OVERLAY:-/nonexistent}"/kernel/linux/patches/*.patch; do
  [ -f "$p" ] || continue
  patch -Np1 -s --fuzz=0 < "$p" || { echo "extra patch does not apply: ${p##*/}" >&2; exit 3; }
  echo "extra patch applied: ${p##*/}"
  n=$((n + 1))
done
echo "extra patches: $n"
cp "$BASE" .config
sh scripts/kconfig/merge_config.sh -m .config "$FRAG"
make olddefconfig
