#!/usr/bin/env bash
# base-manifest.sh PKG...: "sha256  /path" for every regular file and symlink in the given pacman
# packages, sorted by path (ADR 0017: `wol base verify` checks the installed base against it).
# Package metadata (.PKGINFO, .MTREE, .BUILDINFO, .INSTALL) is not a system file.
# Contents only: xattrs (file capabilities) need root to restore and are not hashed.
set -euo pipefail
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
for p in "$@"; do
  [ -f "$p" ] || continue
  bsdtar -xpf "$p" -C "$T" --no-xattrs --exclude '.PKGINFO' --exclude '.MTREE' --exclude '.BUILDINFO' --exclude '.INSTALL' 2>/dev/null ||
    { echo "base-manifest.sh: cannot read $p" >&2; exit 1; }
done
cd "$T"
find . \( -type f -o -type l \) -printf '%P\n' | LC_ALL=C sort | while IFS= read -r f; do
  if [ -L "$f" ]; then
    printf '%s  /%s\n' "$(readlink "$f" | sha256sum | cut -d' ' -f1)" "$f"
  else
    printf '%s  /%s\n' "$(sha256sum < "$f" | cut -d' ' -f1)" "$f"
  fi
done
