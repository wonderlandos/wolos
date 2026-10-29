#!/usr/bin/env bash
# Release manifest: ISO SNAPSHOT(YYYY-MM-DD) REPO_REVISION BASE_URL [VERSION KERNEL CACHY] -> release.json
# on stdout, the form `wol update` reads (wol_core::update::check::Manifest): version, snapshot
# (YYYY/MM/DD, the Arch Linux Archive path), wolos_repo, cachy ([wolos-cachy]
# revision, mirror-cachy.sh; empty when none), kernel, image URL, sha256, size.
# VERSION defaults to the newest CHANGELOG.md entry, KERNEL to the linux-wolos PKGBUILD.
set -euo pipefail
R=$(cd "$(dirname "$0")/.." && pwd)
ISO=${1:?ISO}; SNAP=${2:?SNAPSHOT}; REV=${3:?REVISION}; URL=${4:?BASE_URL}
CACHY=${7:-}
VER=${5:-$(awk '/^## [0-9]/ {print $2; exit}' "$R/CHANGELOG.md")}
# linux-wolos's full version: epoch:wolkernel+linux-pkgrel (kernel/VERSION is the single source).
KV=${6:-$(awk -F= 'FNR == NR { k[$1] = $2; next } /^epoch=/ {e = $2} /^pkgrel=/ {r = $2}
  END {print (e ? e ":" : "") k["wolkernel"] "+" k["linux"] "-" r}' "$R/kernel/VERSION" "$R/kernel/linux-wolos/PKGBUILD")}
sum=$(sha256sum "$ISO" | cut -d' ' -f1)
size=$(stat -c %s "$ISO")
printf '{"version":"%s","snapshot":"%s","wolos_repo":"%s","cachy":"%s","kernel":"%s","image":"%s/%s","sha256":"%s","size":%s}\n' \
  "$VER" "${SNAP//-//}" "$REV" "$CACHY" "$KV" "${URL%/}" "$(basename "$ISO")" "$sum" "$size"
