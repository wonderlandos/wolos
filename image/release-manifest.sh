#!/usr/bin/env bash
# Release manifest: ISO SNAPSHOT(YYYY-MM-DD) REPO_REVISION BASE_URL [VERSION KERNEL] -> release.json
# on stdout, the form `wol update` reads (wol_core::update::check::Manifest): version, snapshot
# (YYYY/MM/DD, the Arch Linux Archive path), wolos_repo, kernel, image URL, sha256, size.
# VERSION defaults to the newest CHANGELOG.md entry, KERNEL to the linux-wolos PKGBUILD.
set -euo pipefail
R=$(cd "$(dirname "$0")/.." && pwd)
ISO=${1:?ISO}; SNAP=${2:?SNAPSHOT}; REV=${3:?REVISION}; URL=${4:?BASE_URL}
VER=${5:-$(awk '/^## [0-9]/ {print $2; exit}' "$R/CHANGELOG.md")}
KV=${6:-$(awk -F= '/^pkgver=/ {v = $2} /^pkgrel=/ {r = $2} END {print v "-" r}' "$R/kernel/linux-wolos/PKGBUILD")}
sum=$(sha256sum "$ISO" | cut -d' ' -f1)
size=$(stat -c %s "$ISO")
printf '{"version":"%s","snapshot":"%s","wolos_repo":"%s","kernel":"%s","image":"%s/%s","sha256":"%s","size":%s}\n' \
  "$VER" "${SNAP//-//}" "$REV" "$KV" "${URL%/}" "$(basename "$ISO")" "$sum" "$size"
