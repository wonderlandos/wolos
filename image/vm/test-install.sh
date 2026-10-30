#!/bin/sh
# Unattended install test of the newest WolOS ISO in a throwaway QEMU/KVM VM (UEFI, no network):
# the ISO's kernel boots with wolos.autoinstall=/dev/vda (installs and powers off), then the
# installed disk boots on its own. A bystander disk (other.raw, random head) stays attached and must
# come out byte-identical. Results: lines "BOOT PASS|FAIL <what>" in $T/checks.txt.
# Usage: image/vm/test-install.sh [ISO]   (--build: image/vm/build.sh first)
# Env: WOLOS_VM (~/WolOS-vm), WOLOS_KEEP_DISK=1 keeps the installed disk as $T/disk.raw for
#      run-installed.sh (otherwise the scratch install.raw is deleted).
# Needs: qemu-system-x86_64, edk2-ovmf, bsdtar, blkid, sfdisk; python3 + Pillow for the screenshot.
set -u
REPO=$(cd "$(dirname "$0")/../.." && pwd); H=$REPO/image/vm/test
VM=${WOLOS_VM:-$HOME/WolOS-vm}; T=$VM/test
OVMF=${WOLOS_OVMF:-/usr/share/edk2/x64}
command -v qemu-system-x86_64 >/dev/null || { echo "$0: qemu-system-x86_64 missing (pacman -S qemu-desktop)" >&2; exit 1; }
[ -f "$OVMF/OVMF_CODE.4m.fd" ] || { echo "$0: no OVMF in $OVMF (pacman -S edk2-ovmf)" >&2; exit 1; }
mkdir -p "$T"
# One VM at a time: another VM on the same disk and sockets starves the install (15 GB hosts).
pgrep -x qemu-system-x86 >/dev/null && { echo "test-install: another QEMU VM is running" >&2; exit 1; }
if [ "${1:-}" = --build ]; then "$REPO/image/vm/build.sh" || exit 1; shift; fi
ISO=${1:-$(ls -t "$VM"/out/wolos-*.iso 2>/dev/null | head -1)}
[ -f "$ISO" ] || { echo "test-install: no ISO in $VM/out" >&2; exit 1; }
cd "$T" || exit 1
rm -f serial-install.log serial-boot.log checks.txt install.raw other.raw shot.png kernel initrd
# Direct kernel boot: the ISO's kernel and initramfs with our command line (the ISO menu has no
# unattended entry). archisosearchuuid comes from the ISO's label lookup instead: archisolabel.
bsdtar -xOf "$ISO" wolos/boot/x86_64/vmlinuz-linux-wolos > kernel &&
  bsdtar -xOf "$ISO" wolos/boot/x86_64/initramfs-linux-wolos.img > initrd || { echo "test-install: no kernel in $ISO" >&2; exit 1; }
label=$(blkid -o value -s LABEL "$ISO")
truncate -s 32G install.raw; truncate -s 17G other.raw
head -c 64M /dev/urandom | dd of=other.raw conv=notrunc status=none
other_before=$(sha256sum other.raw | cut -d' ' -f1)
cp "$OVMF/OVMF_VARS.4m.fd" vars.fd
# 2D virtio-vga: QMP screendump cannot read a GL scanout ("no surface"); Hyprland runs on Mesa.
GPU="-device virtio-vga -display none"
vm() { # extra qemu args...
  exec qemu-system-x86_64 -enable-kvm -cpu host -smp 4 -m 4G -machine q35 \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF/OVMF_CODE.4m.fd" -drive if=pflash,format=raw,file=vars.fd \
    -drive file=install.raw,if=virtio,format=raw,discard=unmap -drive file=other.raw,if=virtio,format=raw \
    -nic none $GPU -qmp unix:qmp.sock,server,nowait "$@"
}
# 1. Install: vda = install.raw, vdb = bystander. Powers off when done (timeout 30 min).
vm -cdrom "$ISO" -kernel kernel -initrd initrd \
  -append "archisobasedir=wolos archisolabel=$label wolos.autoinstall=/dev/vda console=ttyS0 quiet" \
  -serial file:serial-install.log &
qpid=$!; ( sleep 1800; kill $qpid 2>/dev/null ) & tpid=$!
wait $qpid; kill $tpid 2>/dev/null
# 2. Boot the installed disk alone; screenshot after a minute (the login window or Setup).
vm -serial file:serial-boot.log &
qpid=$!; sleep 90
rm -f shot.png; python3 "$H/qmp.py" "$T" shot shot.png || :
kill $qpid 2>/dev/null; wait $qpid 2>/dev/null
other_after=$(sha256sum other.raw | cut -d' ' -f1)
sfdisk -d install.raw > parts.txt 2>/dev/null
b() { if eval "$2"; then echo "BOOT PASS $1"; else echo "BOOT FAIL $1"; fi; }
{ grep 'wolos-install:' serial-install.log
  cat parts.txt
  b "installer said done" "grep -q 'wolos-install: done' serial-install.log"
  b "bystander disk byte-identical" '[ "$other_before" = "$other_after" ]'
  b "ESP + root partitions" "grep -qi 'type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B' parts.txt && grep -qi 'type=4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709' parts.txt"
  b "installed disk boots (its login on serial + fresh screenshot)" "grep -aq 'login:' serial-boot.log && [ -s shot.png ]"
} > checks.txt
grep '^BOOT ' checks.txt
rm -f other.raw kernel initrd
if [ "${WOLOS_KEEP_DISK:-0}" = 1 ]; then mv install.raw "$T/disk.raw"; else rm -f install.raw; fi
echo "test-install: done ($T/checks.txt)"
