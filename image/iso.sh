#!/bin/sh
# Pack a built WolOS installer image (image/build.sh output) as a hybrid UEFI/BIOS ISO, the way FreeBSD
# release builds disc1 (release/amd64/mkisoimages.sh). Ventoy boots FreeBSD ISOs, not raw .img files.
# Run as root on FreeBSD 15.1 with a src tree (kernel/fetch-src.sh).
# Usage: image/iso.sh IMG OUT.iso [SRC]   (SRC default /work/src). Writes OUT.iso + OUT.iso.sha256.
# Limits: ISO 9660 files must be < 4 GiB (wolospkg.txz is checked).
set -eu
IMG=$1; OUT=$2; SRC=${3:-/work/src}; LABEL=WOLOS
die() { echo "iso.sh: $*" >&2; exit 1; }
[ "$(uname -s)" = FreeBSD ] && [ "$(id -u)" = 0 ] || die "run as root on FreeBSD"
[ -f "$SRC/release/amd64/mkisoimages.sh" ] || die "no $SRC/release/amd64/mkisoimages.sh"
MD=$(mdconfig -a -t vnode -f "$IMG"); M=$(mktemp -d)
restore() { [ -f "$M/etc/fstab.img" ] && mv "$M/etc/fstab.img" "$M/etc/fstab"; [ -f "$M/etc/rc.conf.img" ] && mv "$M/etc/rc.conf.img" "$M/etc/rc.conf"; :; }
cleanup() { restore 2>/dev/null; umount "$M" 2>/dev/null || :; mdconfig -d -u "$MD" 2>/dev/null || :; rmdir "$M" 2>/dev/null || :; }
trap cleanup EXIT INT TERM
UFS=$(gpart show -p "$MD" | awk '$4=="freebsd-ufs"{print $3; exit}')
if [ -z "$UFS" ]; then   # memstick: MBR slice + BSD label (as in build.sh)
  SL=$(gpart show -p "$MD" | awk '$4=="freebsd"{print $3; exit}')
  [ -z "$SL" ] || UFS=$(gpart show -p "$SL" | awk '$4=="freebsd-ufs"{print $3; exit}')
fi
[ -n "$UFS" ] || die "no freebsd-ufs partition in $IMG"
# Packed in place (no staging copy: host disk is tight); the image's fstab/rc.conf are restored after.
mount "/dev/$UFS" "$M"
big=$(find "$M" -type f -size +4194303k)
[ -z "$big" ] || die "files >= 4 GiB cannot go on ISO 9660: $big"
# disc1 layout: read-only cd9660 root, rc makes /tmp and /var memory disks.
cp -p "$M/etc/fstab" "$M/etc/fstab.img"; cp -p "$M/etc/rc.conf" "$M/etc/rc.conf.img" 2>/dev/null || touch "$M/etc/rc.conf.img"
echo "/dev/iso9660/$LABEL / cd9660 ro 0 0" > "$M/etc/fstab"
grep -q '^root_rw_mount=' "$M/etc/rc.conf" 2>/dev/null || echo 'root_rw_mount="NO"' >> "$M/etc/rc.conf"
rm -f "$OUT"
sh "$SRC/release/amd64/mkisoimages.sh" -b "$LABEL" "$OUT" "$M"
sha256 -r "$OUT" > "$OUT.sha256"
cat "$OUT.sha256"
