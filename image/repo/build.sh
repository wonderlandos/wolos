#!/usr/bin/env bash
# Build the [wolos] pacman repo from image/repo/packages, in its order: linux-wolos
# (kernel/linux-wolos), hyprland-wolos, wolos-system, then wolos-base (ADR 0017: its manifest hashes
# every base file, so it is built last), each signed, and a signed database.
# Output: $WOLOS_REPO_DIR/x86_64/wolos.db + packages, $WOLOS_REPO_DIR/revision.
# Env: WOLOS_SIGN_KEY (gpg key id; required unless WOLOS_UNSIGNED=1 for local tests),
#      WOLOS_WORK (default ~/WolOS-vm/build), WOLOS_REPO_DIR (default $WORK/repo),
#      WOLOS_REPO_ONLY="pkg ..." (rebuild only these, e.g. from changed.sh; the rest are kept),
#      WOLOS_CHROOT (devtools chroot dir: self-contained recipes build there),
#      WOLOS_INSTALL_DEPS=1 (pacman -U each local dep before makepkg; CI containers).
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

# NAME DIR: makechrootpkg in a clean chroot ($WOLOS_CHROOT, devtools) for a self-contained recipe;
# makepkg on this host for one that reads the source tree (../ or $startdir/..), as CI does in a
# fresh container. Local deps already built go into the chroot with -I.
build() {
  case " ${WOLOS_REPO_ONLY:-$1} " in *" $1 "*) ;; *) return 0 ;; esac
  rm -f "$PKGDEST/$1"-[0-9]*.pkg.tar.zst "$PKGDEST/$1"-[0-9]*.pkg.tar.zst.sig
  if [ -n "${WOLOS_CHROOT:-}" ] && command -v makechrootpkg >/dev/null &&
    ! grep -q '\.\./' "$2/PKGBUILD"; then
    [ -d "$WOLOS_CHROOT/root" ] || mkarchroot "$WOLOS_CHROOT/root" base-devel
    local inst=() d
    for d in ${3//,/ }; do [ "$d" = - ] || inst+=(-I "$(ls "$PKGDEST/$d"-[0-9]*.pkg.tar.zst | head -1)"); done
    (cd "$2" && PKGDEST=$PKGDEST makechrootpkg -c -r "$WOLOS_CHROOT" "${inst[@]}")
  else
    if [ "${WOLOS_INSTALL_DEPS:-0}" = 1 ]; then # fresh container: local deps are not in any repo yet
      # yes: a local package that replaces an Arch one (wolos-system: default-cursors) asks to remove
      # it, and --noconfirm answers no.
      local d; for d in ${3//,/ }; do [ "$d" = - ] || { yes | sudo pacman -U --needed "$PKGDEST/$d"-*.pkg.tar.zst || [ "${PIPESTATUS[1]}" = 0 ]; }; done
    fi
    (cd "$2" && makepkg --syncdeps --noconfirm --cleanbuild --force)
  fi
}

# Build order and local deps: image/repo/packages. wolos-base comes last with its manifest:
# every file of the base packages at the snapshot (downloaded with an own database, never the
# host's) plus the WolOS packages built before it.
while read -r -u 3 name dir deps _; do
  case $name in '' | '#'*) continue ;; esac
  if [ "$name" = wolos-base ]; then
    case " ${WOLOS_REPO_ONLY:-wolos-base} " in *" wolos-base "*) ;; *) continue ;; esac
    conf=$WORK/snapshot-pacman.conf
    awk '/^\[wolos\]/ {skip = 1; next} /^\[/ {skip = 0} !skip' "$IMG/arch/pacman.conf" > "$conf"
    mkdir -p "$WORK/pacdb" "$WORK/pkgcache"
    base_parts="base uutils-coreutils sudo-rs libarchive zsh systemd"
    fakeroot -- pacman -Syw --noconfirm --config "$conf" --dbpath "$WORK/pacdb" --cachedir "$WORK/pkgcache" $base_parts
    "$HERE/base-manifest.sh" "$WORK"/pkgcache/*.pkg.tar.zst "$PKGDEST"/linux-wolos-*.pkg.tar.zst \
      "$PKGDEST"/wolos-system-*.pkg.tar.zst > "$WORK/base.manifest"
    WOLOS_BASE_MANIFEST=$WORK/base.manifest build "$name" "$REPO/$dir" "$deps"
  else
    build "$name" "$REPO/$dir" "$deps"
  fi
done 3< "$HERE/packages"

# Sign packages, then the database.
if [ -n "$KEY" ]; then
  for p in "$PKGDEST"/*.pkg.tar.zst; do
    [ -e "$p.sig" ] || gpg --detach-sign --use-agent --no-armor -u "$KEY" "$p"
  done
fi
repo-add ${KEY:+--sign --key "$KEY"} --remove "$PKGDEST/wolos.db.tar.zst" "$PKGDEST"/*.pkg.tar.zst
echo "$(date -u +%Y%m%d)-$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo local)" > "$RD/revision"
echo "repo: $PKGDEST (revision $(cat "$RD/revision"))"
