#!/usr/bin/env bash
# Build the WolOS live + installer ISO (x86_64, UEFI) with mkarchiso from image/arch (ADR 0016).
# Host: Arch (or CachyOS) with archiso; without mkarchiso it runs in an archlinux podman container.
# Steps: [wolos] repo (image/repo/build.sh, skipped when $WOLOS_REPO_DIR has a database) -> staged
# profile in $WORK/profile -> mkarchiso -> ISO + release manifest in $WORK/out.
# Env: WOLOS_WORK (default ~/WolOS-vm/build), WOLOS_REPO_DIR (default $WORK/repo),
#      WOLOS_STAGE_ONLY=1 (stage the profile, no ISO), WOLOS_IMAGE_URL (manifest base URL),
#      WOLOS_PODMAN=1 (force the container). Disk: ~15 GB in $WORK (packages + squashfs + ISO).
set -euo pipefail
IMG=$(cd "$(dirname "$0")" && pwd); REPO=$(dirname "$IMG")
WORK=${WOLOS_WORK:-${WOLOS_VM:-$HOME/WolOS-vm}/build}
RD=${WOLOS_REPO_DIR:-$WORK/repo}
SNAP=$(tr -d '[:space:]' < "$IMG/arch/snapshot")
P=$WORK/profile
die() { echo "build.sh: $*" >&2; exit 1; }
mkdir -p "$WORK"

# 1. The [wolos] repo: linux-wolos, hyprland-wolos, wolos-system.
if [ ! -e "$RD/x86_64/wolos.db" ]; then
  [ "${WOLOS_STAGE_ONLY:-0}" = 1 ] && die "no [wolos] repo in $RD (run image/repo/build.sh)"
  WOLOS_WORK=$WORK WOLOS_REPO_DIR=$RD "$IMG/repo/build.sh"
fi

# 2. Stage the profile: base + desktop packages, [wolos] read from the local repo for this build.
rm -rf "$P"; cp -a "$IMG/arch" "$P"
cat "$IMG/arch/packages.x86_64" "$REPO/desktop/packages.txt" | sed 's/#.*//' | tr -s ' \t' '\n' | grep . > "$P/packages.x86_64"
# No DBPath for the build: pacstrap writes the database to its usual place on the build host, and
# install-run moves it into the slot (/usr/lib/pacman, as airootfs/etc/pacman.conf says).
# [wolos] is this build's own local repo (file://): read as is, so the build host's keyring is not
# touched; the installed system's pacman.conf checks every signature.
awk -v srv="Server = file://$RD/\$arch" '
  /^DBPath/ { next }
  /^\[/ { sect = $0 }
  sect == "[wolos]" && /^SigLevel/ { next }
  sect == "[wolos]" && /^(Server|Include) = / { print "SigLevel = Never"; print srv; next }
  { print }' "$IMG/arch/pacman.conf" > "$P/pacman.conf"
A=$P/airootfs
mkdir -p "$A/etc/wolos" "$A/etc/sysusers.d" "$A/etc/sudoers.d" "$A/etc/tmpfiles.d"
echo "$SNAP" > "$A/etc/wolos/snapshot"
# Live desktop (ADR 0009): `live` logs in automatically; it may run the installer as root, no password.
cat > "$A/etc/sysusers.d/wolos-live.conf" <<'EOF'
u live 1000 "WolOS Live" /home/live /usr/bin/zsh
m live wheel
m live video
m live input
m live seat
EOF
echo 'd /home/live 0700 live live -' > "$A/etc/tmpfiles.d/wolos-live.conf"
echo 'live ALL=(root) NOPASSWD: /usr/lib/wolos/install-run, /usr/lib/wolos/install-check, /usr/bin/systemctl reboot, /usr/bin/systemctl poweroff' > "$A/etc/sudoers.d/wolos-live"
printf '%s\n' '{"autologin":"live"}' > "$A/etc/wolos/login.json"
touch "$A/etc/wolos-live"
# "Install WolOS" first in the live dock (shell.json is a backup file of wolos-system: this copy wins).
sed 's|"dock": \[|"dock": [\
    { "id": "org.wolos.install", "name": "Install WolOS", "icon": "system-software-install", "exec": ["/usr/lib/wolos/wol-install"] },|' \
  "$REPO/desktop/overlay/etc/wolos/shell.json" > "$A/etc/wolos/shell.json"
grep -q org.wolos.install "$A/etc/wolos/shell.json" || die "live dock patch failed"
if [ "${WOLOS_STAGE_ONLY:-0}" = 1 ]; then echo "profile: $P"; exit 0; fi

# 3. mkarchiso (root) on the host, or in a container.
OUT=$WORK/out; rm -rf "$WORK/work"; mkdir -p "$OUT"
if command -v mkarchiso >/dev/null && [ "${WOLOS_PODMAN:-0}" != 1 ]; then
  S=; [ "$(id -u)" = 0 ] || S=sudo
  $S mkarchiso -v -w "$WORK/work" -o "$OUT" "$P"
else
  command -v podman >/dev/null || die "needs mkarchiso (pacman -S archiso) or podman"
  podman run --rm --privileged -v "$WORK:$WORK" -v "$RD:$RD:ro" archlinux:base-devel \
    bash -c "pacman -Sy --noconfirm archiso >/dev/null && mkarchiso -v -w '$WORK/work' -o '$OUT' '$P'"
fi
S=; [ "$(id -u)" = 0 ] || S=sudo
$S rm -rf "$WORK/work"   # disk: the pacstrap root is in the squashfs now

# 4. Release manifest next to the ISO.
ISO=$(ls -t "$OUT"/wolos-*.iso | head -1)
REV=$({ cat "$RD/revision" "$RD/x86_64/revision" 2>/dev/null || :; } | head -1)
[ -n "$REV" ] || REV=$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo local)
"$IMG/release-manifest.sh" "$ISO" "$SNAP" "$REV" "${WOLOS_IMAGE_URL:-https://github.com/wonderlandos/wolos-packages/releases/download/$SNAP}" \
  "" "" "$(cat "${WOLOS_CACHY_DIR:-$WORK/cachy}/revision" 2>/dev/null || :)" > "${ISO%.iso}.json"
echo "image: $ISO  manifest: ${ISO%.iso}.json"
