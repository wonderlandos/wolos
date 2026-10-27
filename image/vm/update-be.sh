#!/bin/sh
# In the build VM (FreeBSD, root): update an installed WolOS disk in a NEW boot environment.
# Called by image/vm/update-disk.sh after `WOLOS_STAGE_ONLY=1 image/build.sh` left /work/stage + /work/pkgrepo.
# Usage: update-be.sh SERIAL BE   (SERIAL = virtio-blk serial of the attached disk; BE = new boot environment)
# Never: the build VM's own disks, other pools, /home, /usr/home, accounts, rc.conf, user dotfiles.
set -eu
SERIAL=${1:?serial}; BE=${2:?be}
WORK=${WOLOS_WORK:-/work}; STAGE=$WORK/stage; PR=$WORK/pkgrepo
ALT=/mnt/upd; M=$ALT.be
die() { echo "update-be: $*" >&2; exit 1; }
[ -d "$STAGE/usr/local/bin" ] && [ -f "$PR/var/cache/wolos-repo/wolos-packages.txt" ] || die "no $STAGE / $PR (build.sh WOLOS_STAGE_ONLY=1)"
# The attached disk by its serial, never by vtbdN order (vtbd0/1 are the build VM's root and /work).
D=
for g in $(sysctl -n kern.disks); do
  [ "$(diskinfo -s "$g" 2>/dev/null)" = "$SERIAL" ] && D=$g
done
[ -n "$D" ] || die "no disk with serial $SERIAL"
P=$(gpart show -p "$D" | awk '$4=="freebsd-zfs"{print $3; exit}')
[ -n "$P" ] || die "$D has no freebsd-zfs partition"
GUID=$(zdb -l "/dev/$P" | awk '$1=="pool_guid:"{print $2; exit}')
POOL=$(zdb -l "/dev/$P" | awk '$1=="name:"{gsub(/\x27/,"",$2); print $2; exit}')
[ -n "$GUID" ] && [ -n "$POOL" ] || die "no ZFS label on /dev/$P"
# Import by GUID under an altroot, nothing mounted. A same-named pool already imported means the build VM's
# own: refuse rather than rename the user's pool (bootfs and loader paths name it).
zpool list -H -o name "$POOL" >/dev/null 2>&1 && die "a pool named $POOL is already imported here"
mkdir -p "$ALT" "$M"
zpool import -f -R "$ALT" -N -d "/dev/$P" "$GUID"
done_=0
cleanup() {
  umount "$M/dev" 2>/dev/null || :
  bectl -r "$POOL/ROOT" umount -f "$BE" 2>/dev/null || :
  zpool export "$POOL" 2>/dev/null || zpool export -f "$POOL" 2>/dev/null || :
  [ "$done_" = 1 ] || echo "update-be: FAILED; pool exported, old boot environment still active" >&2
}
trap cleanup EXIT INT TERM
B="bectl -r $POOL/ROOT"
ACTIVE=$(zpool get -H -o value bootfs "$POOL"); ACTIVE=${ACTIVE##*/}
[ -n "$ACTIVE" ] && [ "$ACTIVE" != - ] || die "pool $POOL has no bootfs"
$B list -H | awk '{print $1}' | grep -qx "$BE" && die "boot environment $BE exists"
$B create -e "$ACTIVE" "$BE"
$B mount "$BE" "$M"
# The BE is only ROOT/<be>: /home, /usr/home, /var/log, ... are separate datasets and stay unmounted.
for d in home usr/home; do
  [ -z "$(ls -A "$M/$d" 2>/dev/null)" ] || [ -L "$M/$d" ] || die "$M/$d has files: home is inside the BE, refusing"
done
# WolOS-owned files only. Accounts, rc.conf, fstab, hosts, users' homes and dotfiles never come from the stage.
EX=$WORK/update-exclude; : > "$EX"
for x in ./home ./usr/home ./root ./etc/master.passwd ./etc/passwd ./etc/group ./etc/pwd.db ./etc/spwd.db \
  ./etc/rc.conf ./etc/rc.conf.local ./etc/fstab ./etc/hosts ./etc/hostid ./etc/resolv.conf ./etc/wpa_supplicant.conf \
  ./boot/loader.conf ./usr/local/etc/wolos/login.json ./firstboot ./var; do echo "$x" >> "$EX"; done
LIST=$WORK/update-replaced.txt
(cd "$STAGE" && find . \( -type f -o -type l \) | grep -vxFf "$EX" | grep -vE '^\./(home|usr/home|root|var)/' | sort) > "$LIST"
chflags -R noschg "$M/boot/kernel" 2>/dev/null || :
tar -C "$STAGE" -X "$EX" --uid 0 --gid 0 -cf - . | tar -C "$M" -xpf -
# Packages: the offline repo + install-pkgs in the BE (pkg upgrades changed ones, adds new ones, forks re-applied).
mkdir -p "$M/var/cache"; rm -rf "$M/var/cache/wolos-repo"
tar -C "$PR/var/cache" -cf - wolos-repo | tar -C "$M/var/cache" -xpf -
mount -t devfs devfs "$M/dev"
pkg -c "$M" unlock -y hyprland drm-612-kmod >/dev/null 2>&1 || :
chroot "$M" /usr/local/libexec/wolos/install-pkgs
umount "$M/dev"
[ -d "$M/boot/kernel" ] && chroot "$M" kldxref /boot/kernel /boot/modules 2>/dev/null || :
echo "WolOS" > "$M/etc/wolos-release"
$B umount "$BE"
$B activate "$BE"
done_=1
echo "update-be: replaced $(wc -l < "$LIST" | tr -d ' ') WolOS files (list: $LIST); active $BE, fallback $ACTIVE"
