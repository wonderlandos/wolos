#!/usr/bin/env bash
# Build the [wolos] pacman repo: linux-wolos (kernel/linux-wolos), hyprland-wolos, wolos-system, then
# wolos-base (ADR 0017: its manifest hashes every base file, so it is built last), each signed, and
# a signed database. Output: $WOLOS_REPO_DIR/x86_64/wolos.db + packages, $WOLOS_REPO_DIR/revision.
# Env: WOLOS_SIGN_KEY (gpg key id; required unless WOLOS_UNSIGNED=1 for local tests),
#      WOLOS_WORK (default ~/WolOS-vm/build), WOLOS_REPO_DIR (default $WORK/repo),
#      WOLOS_REPO_ONLY="pkg ..." (rebuild only these; the rest are kept from the last build).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); IMG=$(dirname "$HERE"); REPO=$(dirname "$IMG")
WORK=${WOLOS_WORK:-${WOLOS_VM:-$HOME/WolOS-vm}/build}
RD=${WOLOS_REPO_DIR:-$WORK/repo}
KEY=${WOLOS_SIGN_KEY:-}
if [ -z "$KEY" ] && [ "${WOLOS_UNSIGNED:-0}" != 1 ]; then
  echo "repo/build.sh: set WOLOS_SIGN_KEY=<gpg key id> (or WOLOS_UNSIGNED=1 for a local test repo)" >&2
  exit 2
fi
export PKGDEST=$RD/x86_64
mkdir -p "$PKGDEST" "$WORK"

build() { # NAME DIR
  case " ${WOLOS_REPO_ONLY:-$1} " in *" $1 "*) ;; *) return 0 ;; esac
  rm -f "$PKGDEST/$1"-[0-9]*.pkg.tar.zst "$PKGDEST/$1"-[0-9]*.pkg.tar.zst.sig
  (cd "$2" && makepkg --syncdeps --noconfirm --cleanbuild --force)
}
build linux-wolos "$REPO/kernel/linux-wolos"
build hyprland-wolos "$HERE/hyprland-wolos"
build wolos-system "$HERE/wolos-system"

# wolos-base manifest: every file of the base packages at the snapshot (downloaded with an own
# database, never the host's) plus the WolOS packages above.
conf=$WORK/snapshot-pacman.conf
awk '/^\[wolos\]/ {skip = 1; next} /^\[/ {skip = 0} !skip' "$IMG/arch/pacman.conf" > "$conf"
mkdir -p "$WORK/pacdb" "$WORK/pkgcache"
base_parts="base uutils-coreutils sudo-rs libarchive zsh systemd"
fakeroot -- pacman -Syw --noconfirm --config "$conf" --dbpath "$WORK/pacdb" --cachedir "$WORK/pkgcache" $base_parts
"$HERE/base-manifest.sh" "$WORK"/pkgcache/*.pkg.tar.zst "$PKGDEST"/linux-wolos-*.pkg.tar.zst \
  "$PKGDEST"/wolos-system-*.pkg.tar.zst > "$WORK/base.manifest"
WOLOS_BASE_MANIFEST=$WORK/base.manifest build wolos-base "$HERE/wolos-base"

# Sign packages, then the database.
if [ -n "$KEY" ]; then
  for p in "$PKGDEST"/*.pkg.tar.zst; do
    [ -e "$p.sig" ] || gpg --detach-sign --use-agent --no-armor -u "$KEY" "$p"
  done
fi
repo-add ${KEY:+--sign --key "$KEY"} --remove "$PKGDEST/wolos.db.tar.zst" "$PKGDEST"/*.pkg.tar.zst
echo "$(date -u +%Y%m%d)-$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo local)" > "$RD/revision"
echo "repo: $PKGDEST (revision $(cat "$RD/revision"))"
