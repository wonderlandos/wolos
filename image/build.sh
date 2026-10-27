#!/bin/sh
# Build the WolOS installer/live USB image (amd64, UEFI+BIOS) from the official memstick.
# Run as root on FreeBSD 15.1 (or inside image/vm/build-in-vm.sh).
# Env: WOLOS_WORK (default /work), WOLOS_IMG_SIZE (default 12g), WOLOS_LIVE=1 (also install desktop
#      into the live root), WOLOS_REL (15.1-RELEASE), WOLOS_MIRROR (download.freebsd.org).
# Rust: host needs `pkg install rust git pkgconf` + compositor build deps (see build-rust.sh).
# Kernel: set WOLOS_SRC=<tree from kernel/fetch-src.sh> to ship WolKernel + wolos_qos.
# Hyprland: the WolOS fork (image/hyprland/build-fork.sh + patches) replaces pkg hyprland after the package
#   install (install-pkgs), then pkg hyprland is locked. WOLOS_HYPR_FORK=0 ships stock pkg hyprland.
# Disk: ~20 GB free in $WOLOS_WORK (xz 0.9 GB + raw 1.5 GB + image 12 GB + staging/pkg cache ~5 GB).
set -eu
IMG=$(cd "$(dirname "$0")" && pwd); REPO=$(dirname "$IMG")
WORK=${WOLOS_WORK:-/work}; REL=${WOLOS_REL:-15.1-RELEASE}; SIZE=${WOLOS_IMG_SIZE:-12g}
MIRROR=${WOLOS_MIRROR:-https://download.freebsd.org}
BASEURL=$MIRROR/releases/amd64/amd64/ISO-IMAGES/${REL%-RELEASE}
SRCIMG=FreeBSD-$REL-amd64-memstick.img
OUT=$WORK/WolOS-${REL%-RELEASE}-amd64.img
NEED_MB=${WOLOS_NEED_MB:-20000}
die() { echo "build.sh: $*" >&2; exit 1; }

[ "$(uname -s)" = FreeBSD ] || die "needs FreeBSD (mdconfig). On Linux: image/vm/build-in-vm.sh"
[ "$(id -u)" = 0 ] || die "run as root"
mkdir -p "$WORK"
free=$(df -Pm "$WORK" | awk 'NR==2{print $4}')
[ "$free" -ge "$NEED_MB" ] || { echo "build.sh: need $NEED_MB MB free in $WORK, have $free MB" >&2; exit 2; }
[ -f "$REPO/desktop/packages.txt" ] || die "missing desktop/packages.txt"

# WOLOS_STAGE_ONLY=1 (image/vm/update-disk.sh): only step 3, leaving $WORK/stage + $WORK/pkgrepo for an
# in-place update of an installed disk; no memstick, no image.
STAGE=$WORK/stage; SO=${WOLOS_STAGE_ONLY:-0}
cd "$WORK"
if [ "$SO" != 1 ]; then
# 1. Fetch + verify.
fetch -m "$BASEURL/CHECKSUM.SHA256-FreeBSD-$REL-amd64"
if [ ! -f "$SRCIMG" ]; then
  fetch -m "$BASEURL/$SRCIMG.xz"
  want=$(awk -v f="($SRCIMG.xz)" '$2==f{print $4}' "CHECKSUM.SHA256-FreeBSD-$REL-amd64")
  [ -n "$want" ] && [ "$(sha256 -q "$SRCIMG.xz")" = "$want" ] || die "checksum mismatch: $SRCIMG.xz"
  xz -d "$SRCIMG.xz"   # disk: keep only the raw memstick
fi
want=$(awk -v f="($SRCIMG)" '$2==f{print $4}' "CHECKSUM.SHA256-FreeBSD-$REL-amd64")
[ "$(sha256 -q "$SRCIMG")" = "$want" ] || die "checksum mismatch: $SRCIMG"

# 2. Grow a copy and mount its UFS root.
cp "$SRCIMG" "$OUT"; truncate -s "$SIZE" "$OUT"
MD=$(mdconfig -a -t vnode -f "$OUT"); R=$WORK/root
cleanup() { umount "$R/dev" 2>/dev/null || :; umount "$R" 2>/dev/null || :; mdconfig -d -u "$MD" 2>/dev/null || :; }
trap cleanup EXIT INT TERM
gpart recover "$MD" >/dev/null 2>&1 || :
# Resize every partition on the path to the freebsd-ufs root (GPT or MBR+BSD label).
UFS=$(gpart show -p "$MD" | awk '$4=="freebsd-ufs"{print $3; exit}')
if [ -z "$UFS" ]; then
  SL=$(gpart show -p "$MD" | awk '$4=="freebsd"{print $3; exit}')
  [ -n "$SL" ] || die "no UFS/freebsd partition in $SRCIMG"
  gpart resize -i "${SL##*s}" "$MD"; gpart recover "$SL" >/dev/null 2>&1 || :
  UFS=$(gpart show -p "$SL" | awk '$4=="freebsd-ufs"{print $3; exit}')
  gpart resize -i 1 "$SL"
else
  gpart resize -i "${UFS##*p}" "$MD"
fi
growfs -y "/dev/$UFS"
mkdir -p "$R"; mount "/dev/$UFS" "$R"
fi

# 3. Packages -> offline repo (wolospkg.txz, installed in the target chroot so pkg scripts see a real
#    /etc/passwd); overlay + design + builds -> wolos.txz.
[ ! -d "$STAGE" ] || { chflags -R noschg "$STAGE"; rm -rf "$STAGE"; }; mkdir -p "$STAGE"
pkgs=$(cat "$IMG/packages-base.txt" "$REPO/desktop/packages.txt" | sed 's/#.*//' | tr -s ' \t' '\n' | grep . | sort -u)
PR=$WORK/pkgrepo; PRC=$WORK/pkgrepo.conf
rm -rf "$PR"; mkdir -p "$PR/var/cache/wolos-repo" "$PRC"
cp /etc/pkg/FreeBSD.conf "$PRC/"   # ports + kmods only: never the host's pkgbase repos
# shellcheck disable=SC2086
env ASSUME_ALWAYS_YES=yes pkg -R "$PRC" -o ABI=FreeBSD:15:amd64 -o OSVERSION=1501000 \
  fetch -y -d -o "$PR/var/cache/wolos-repo" $pkgs
pkg repo "$PR/var/cache/wolos-repo" >/dev/null
echo "$pkgs" > "$PR/var/cache/wolos-repo/wolos-packages.txt"
[ -d "$REPO/desktop/overlay" ] && (cd "$REPO/desktop/overlay" && tar -cf - .) | tar -C "$STAGE" -xpf -
[ -d "$REPO/design/icons" ] && sh "$REPO/design/icons/install.sh" "$STAGE/usr/local/share/icons/WolOS"
if [ -d "$REPO/design/wallpapers" ]; then mkdir -p "$STAGE/usr/local/share/wolos/wallpapers"; cp -R "$REPO/design/wallpapers/" "$STAGE/usr/local/share/wolos/wallpapers/"; fi
if [ -d "$REPO/design/tokens" ]; then mkdir -p "$STAGE/usr/local/share/wolos/tokens"; cp -R "$REPO/design/tokens/" "$STAGE/usr/local/share/wolos/tokens/"; fi
if [ -d "$REPO/design/icons/store" ]; then mkdir -p "$STAGE/usr/local/share/wolos/store-icons"; cp "$REPO"/design/icons/store/*.svg "$STAGE/usr/local/share/wolos/store-icons/"; fi
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$WORK/target/wolos}   # shared by build-rust.sh + build-plugins.sh
"$IMG/build-rust.sh" "$STAGE"
"$IMG/fetch-theme.sh" "$STAGE"
# Optional Hyprland fork over pkg hyprland.
# The fork rides in the offline repo as a tarball: extracting it in the stage would be overwritten by
# pkg hyprland during install. Also installed into the build host so plugins compile against its headers.
if [ "${WOLOS_HYPR_FORK:-1}" = 1 ]; then
  HF=$WORK/hyprfork; rm -rf "$HF"
  "$IMG/hyprland/build-fork.sh" "$HF"
  tar -C "$HF" -cf - . | tar -C / -xpf -
  tar -C "$HF" --uid 0 --gid 0 --uname root --gname wheel -cf "$PR/var/cache/wolos-repo/hyprland-fork.tar" .
  rm -rf "$HF"
fi
# hyprbars (traffic lights) + wol-hypr -> /usr/local/lib/hyprland/plugins (paths in hyprland.conf).
"$IMG/hyprland/build-plugins.sh" "$STAGE"
# WolKernel + wolos_qos into stage (overrides kernel.txz on install) when a source tree exists.
if [ -n "${WOLOS_SRC:-}" ]; then INSTALL=1 DESTDIR="$STAGE" "$REPO/kernel/build.sh" "$WOLOS_SRC"; fi
# WolOS i915kms (kernel/drm, ADR 0008): rides in the offline repo like the Hyprland fork, extracted
# over pkg drm-612-kmod by install-pkgs. WOLOS_DRM_FORK=0 ships the stock pkg modules.
if [ -n "${WOLOS_SRC:-}" ] && [ "${WOLOS_DRM_FORK:-1}" = 1 ]; then
  DF=$WORK/drmfork; rm -rf "$DF"
  "$REPO/kernel/drm/build.sh" "$WOLOS_SRC" "$DF"
  tar -C "$DF" --uid 0 --gid 0 --uname root --gname wheel -cf "$PR/var/cache/wolos-repo/drm-fork.tar" .
  rm -rf "$DF"
fi
(cd "$IMG/files" && tar -cf - .) | tar -C "$STAGE" -xpf -
cp "$IMG/hw/verify-gpu.sh" "$STAGE/usr/local/libexec/wolos/verify-gpu.sh"   # native-boot GPU proof
chmod 555 "$STAGE/etc/rc.d/wolos_firstboot" "$STAGE/etc/rc.d/wolos_hw" "$STAGE/etc/rc.d/wolos_gpu" "$STAGE/etc/rc.d/wolos_wifi" "$STAGE/etc/rc.d/wolos_live" "$STAGE/etc/rc.d/wolos_quiet" "$STAGE/usr/local/etc/rc.d/wold" "$STAGE/usr/local/libexec/wolos/"*
chmod 440 "$STAGE/usr/local/etc/sudoers.d/wolos"   # sudoers mode (visudo -c)
mkdir -p "$STAGE/boot/images"
# rEFInd and similar boot menus: the WolOS icon next to \EFI\WolOS\loader.efi (libexec/wolos/install-esp).
mkdir -p "$STAGE/usr/local/share/wolos/boot"
[ -f "$REPO/design/brand/os_wolos.png" ] && cp "$REPO/design/brand/os_wolos.png" "$STAGE/usr/local/share/wolos/boot/os_wolos.png"
# Kernel splash (ADR 0012): the WolOS mark on black from the loader to the first WolOS frame.
[ -f "$REPO/design/brand/wolos-splash.png" ] && cp "$REPO/design/brand/wolos-splash.png" "$STAGE/boot/images/wolos.png"
# Illustrations for Setup, the installer and the lock screen (kit::assistant), @1x and @2x.
mkdir -p "$STAGE/usr/local/share/wolos/illustrations"
for a in setup-welcome white-rabbit keyhole pocket-watch cheshire teacup lock-peek; do
  for f in "$REPO/design/illustrations/png/$a"@*.png; do
    if [ -f "$f" ]; then cp "$f" "$STAGE/usr/local/share/wolos/illustrations/"; fi
  done
done
cat > "$STAGE/boot/loader.conf.wolos" <<'L'
autoboot_delay="2"
boot_mute="YES"
module_verbose="0"
splash="/boot/images/wolos.png"
shutdown_splash="/boot/images/wolos.png"
splash_changer_enable="NO"
loader_logo="none"
loader_brand="wolos"
loader_menu_title="Welcome to WolOS"
loader_menu_multi_user_prompt="WolOS"
linux_load="YES"
linux64_load="YES"
fusefs_load="YES"
kern.ipc.shmseg="1024"
kern.ipc.shmmni="1024"
kern.maxproc="100000"
hw.usb.no_boot_wait="1"
# Kernel + rc on screen and serial (headless boxes, VM tests).
boot_multicons="YES"
boot_serial="YES"
wolos_qos_load="YES"
L
[ "$SO" = 1 ] && { echo "stage: $STAGE  pkgrepo: $PR"; exit 0; }
DIST=$R/usr/freebsd-dist
# 15.x memstick is pkgbase: MANIFEST lists base/kernel but the sets are not on it. Fetch + verify.
for d in base.txz kernel.txz; do
  [ -f "$DIST/$d" ] || fetch -o "$DIST/$d" "$MIRROR/releases/amd64/$REL/$d"
  [ "$(sha256 -q "$DIST/$d")" = "$(awk -v f="$d" '$1==f{print $2}' "$DIST/MANIFEST")" ] || die "checksum mismatch: $d"
done
# Repo files come from a user checkout: ship everything as root:wheel (ldconfig ignores non-root dirs).
tar -C "$STAGE" --uid 0 --gid 0 --uname root --gname wheel -cJf "$DIST/wolos.txz" .
# .pkg files are already zstd: store, don't recompress (xz -0 only for the .txz name/format).
tar -C "$PR" -cf - . | xz -0 -T0 > "$DIST/wolospkg.txz"
for d in wolos:"WolOS desktop" wolospkg:"WolOS packages"; do
  n=$(tar -tf "$DIST/${d%%:*}.txz" | wc -l | tr -d ' ')
  printf '%s.txz\t%s\t%s\t%s\t%s\ton\n' "${d%%:*}" "$(sha256 -q "$DIST/${d%%:*}.txz")" "$n" "${d%%:*}" "\"${d#*:}\"" >> "$DIST/MANIFEST"
done
rm -rf "$PR" "$PRC"
[ "${WOLOS_KEEP_STAGE:-0}" = 1 ] || { chflags -R noschg "$STAGE"; rm -rf "$STAGE"; }   # disk: stage is in wolos.txz now

# 4. Installer: unattended ZFS + boot environments, asks only for the disk. WolOS branding:
#    loader brand/title, bsdinstall OSNAME (backtitles, EFI boot label), rc.local welcome, and
#    the C helpers' hardcoded "FreeBSD Installer" backtitle (same-length NUL-padded patch).
cp "$IMG/installerconfig" "$R/etc/installerconfig"
cp "$IMG/files/boot/lua/gfx-wolos.lua" "$R/boot/lua/"
printf 'loader_brand="wolos"\nloader_logo="none"\nloader_menu_title="Welcome to WolOS Installer"\n' >> "$R/boot/loader.conf"
sed -i "" 's/^: ${OSNAME:=FreeBSD}/: ${OSNAME:=WolOS}/' "$R/usr/share/bsdconfig/common.subr"
grep -q 'OSNAME:=WolOS' "$R/usr/share/bsdconfig/common.subr" || die "OSNAME patch failed"
sed -i "" 's/Welcome to FreeBSD/Welcome to WolOS/g' "$R/etc/rc.local"
for b in "$R"/usr/libexec/bsdinstall/*; do
  grep -q 'FreeBSD Installer' "$b" 2>/dev/null || continue
  if file -b "$b" | grep -q ELF; then
    perl -pi -e 's/FreeBSD Installer\0/WolOS Installer\0\0\0/g' "$b"
  else
    sed -i "" 's/FreeBSD Installer/WolOS Installer/g' "$b"
  fi
done
! grep -rl 'FreeBSD Installer' "$R/usr/libexec/bsdinstall" "$R/boot/lua/gfx-wolos.lua" || die "FreeBSD Installer branding left"

# 5. Live root (optional): same desktop on the USB stick.
if [ "${WOLOS_LIVE:-1}" = 1 ]; then
  tar -C "$R" -xpf "$DIST/wolos.txz"; tar -C "$R" -xpf "$DIST/wolospkg.txz"
  mount -t devfs devfs "$R/dev"; chroot "$R" /usr/local/libexec/wolos/install-pkgs; umount "$R/dev"
  cat "$R/boot/loader.conf.wolos" >> "$R/boot/loader.conf"
  "$IMG/rcconf.sh" "$R"
  sed -i "" 's/[[:<:]]ro[[:>:]]/rw/' "$R/etc/fstab"   # live root writable (USB)
  touch "$R/firstboot"
  # Live desktop (ADR 0009): the ISO root is read-only, so accounts and config are baked here.
  # Default boot = WolOS desktop as `live` (autologin); rc.local (bsdinstall) only with
  # `set wolos.install=1` at the loader prompt; "Install WolOS" in the dock runs the same installer.
  pw -R "$R" usershow _wollogin >/dev/null 2>&1 || pw -R "$R" useradd _wollogin -u 911 -c "WolOS login window" -d /var/empty -s /usr/sbin/nologin -G video
  pw -R "$R" usershow live >/dev/null 2>&1 || pw -R "$R" useradd live -u 1000 -c "WolOS Live" -d /home/live -s /usr/local/bin/zsh -G wheel,video,operator -w none
  # Install WolOS (ADR 0013): the live user runs the installer (and its read-only disk check,
  # which reads the Secure Boot variable) as root, no password, no terminal.
  mkdir -p "$R/usr/local/etc/sudoers.d"
  printf 'live ALL=(root) NOPASSWD: /usr/local/libexec/wolos/install-run, /usr/local/libexec/wolos/install-check\n' > "$R/usr/local/etc/sudoers.d/wolos-live"
  chmod 440 "$R/usr/local/etc/sudoers.d/wolos-live"
  printf '%s\n' '{"autologin":"live"}' > "$R/usr/local/etc/wolos/login.json"
  sysrc -f "$R/etc/rc.conf" wolos_live_enable=YES >/dev/null
  sed -i "" -E 's|^(ttyv0[[:space:]].*[[:space:]])on(ifexists)?([[:space:]])|\1off\3|' "$R/etc/ttys"
  printf '#!/bin/sh\n[ -f /etc/wolos-live ] && [ "$(kenv -q wolos.install)" != 1 ] && exit 0\n' > "$R/etc/rc.local.new"
  sed 1d "$R/etc/rc.local" >> "$R/etc/rc.local.new"; mv "$R/etc/rc.local.new" "$R/etc/rc.local"
  sed -i "" 's|"dock": \[|"dock": [\
    { "id": "org.wolos.install", "name": "Install WolOS", "icon": "system-software-install", "exec": ["/usr/local/libexec/wolos/wol-install"] },|' "$R/usr/local/etc/wolos/shell.json"
  grep -q org.wolos.install "$R/usr/local/etc/wolos/shell.json" || die "live dock patch failed"
  touch "$R/etc/wolos-live"
fi
echo "WolOS" > "$R/etc/wolos-release"
sync; cleanup; trap - EXIT
echo "image: $OUT  (dd if=$OUT of=/dev/daX bs=1m conv=sync)"
