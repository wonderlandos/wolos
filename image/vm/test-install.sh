#!/bin/sh
# Offline UEFI install test of $WOLOS_VM/out/WolOS-15.1-amd64.img.xz in a throwaway QEMU VM (-nic none).
# Usage: image/vm/test-install.sh [--build]   (--build: run image/vm/build-in-vm.sh with WOLOS_LIVE=0 first)
# Env: WOLOS_KEEP_DISK=1 keeps the installed disk as $WOLOS_VM/test/disk.raw (zpool-trimmed) for image/vm/run-installed.sh.
# WOLOS_KEEP_INSTALL=1 keeps the scratch install.raw (throwaway flows; disk.raw untouched).
# Scratch disk is install.raw: an existing disk.raw is only replaced with WOLOS_KEEP_DISK=1.
# Powers off the build VM while testing (host RAM: two VMs got OOM-killed and corrupted its UFS root),
# brings it back after. Sparse disks in $WOLOS_VM/test, deleted at the end. Keeps:
# A bystander disk (other.raw, 17 GB, random head) stays attached throughout: it must come out
# byte-identical. Boot-chain results: lines "BOOT PASS|FAIL <what>" at the end of checks.txt.
#   checks.txt, install-pkgs.log, serial-install.log, serial-boot.log, prog-N.png (installer screenshots).
# Needs: qemu-system-x86_64, OVMF (/usr/share/edk2/x64), xz, python3 (+ Pillow for screenshots).
set -u
REPO=$(cd "$(dirname "$0")/../.." && pwd)
VM=${WOLOS_VM:-$HOME/WolOS-vm}; T=$VM/test; H=$REPO/image/vm/test
OVMF=${WOLOS_OVMF:-/usr/share/edk2/x64}
IMG=$VM/out/WolOS-15.1-amd64.img.xz
mkdir -p "$T"; cd "$T" || exit 1
qmp() { python3 "$H/qmp.py" "$T" "$@"; }
ser() { python3 "$H/serial.py" "$T" "$@"; }
plain() { sed 's/\x1b\[[0-9;?]*[a-zA-Z]//g' serial.log | tr -c '[:print:]\n' ' '; }
vm() {   # $1 = with installer medium (1/0)
  inst=; [ "$1" = 1 ] && inst="-drive file=inst.img,if=none,id=d1,format=raw -device virtio-blk-pci,drive=d1,bootindex=1"
  # shellcheck disable=SC2086
  qemu-system-x86_64 -enable-kvm -cpu host -smp 4 -m 3G -machine q35 \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF/OVMF_CODE.4m.fd" -drive if=pflash,format=raw,file=vars.fd \
    -drive file=install.raw,if=none,id=d0,format=raw,discard=unmap -device virtio-blk-pci,drive=d0,bootindex=2 $inst \
    -drive file=other.raw,if=none,id=d2,format=raw -device virtio-blk-pci,drive=d2 \
    -nic none -vga std -display none -vnc 127.0.0.1:1 \
    -chardev socket,id=s0,path=serial.sock,server=on,wait=off,logfile=serial.log,logappend=on -serial chardev:s0 \
    -qmp unix:qmp.sock,server,nowait -daemonize -pidfile vm.pid
}
stop_vm() { kill "$(cat vm.pid)" 2>/dev/null; sleep 2; }

if [ "${1:-}" = --build ]; then
  (cd "$REPO" && WOLOS_LIVE=0 image/vm/build-in-vm.sh) || { echo "test-install: build failed"; exit 1; }
fi
[ -f "$IMG" ] || { echo "test-install: no $IMG"; exit 1; }
"$REPO/image/vm/ssh.sh" 'shutdown -p now' >/dev/null 2>&1 || :
p=$(cat "$VM/vm.pid" 2>/dev/null); while [ -n "$p" ] && kill -0 "$p" 2>/dev/null; do sleep 5; done
echo "test-install: build VM off"

rm -f serial.log inst.img install.raw prog-*.png checks.txt install-pkgs.log
xz -dc "$IMG" | cp --sparse=always /dev/stdin inst.img
truncate -s 20G install.raw; rm -f other.raw; truncate -s 17G other.raw
head -c 64M /dev/urandom | dd of=other.raw conv=notrunc status=none   # vtbd2 at install, vtbd1 after
other_before=$(sha256sum other.raw | cut -d' ' -f1)
cp "$OVMF/OVMF_VARS.4m.fd" vars.fd
vm 1
for _ in $(seq 120); do plain | grep -q "terminal type" && break; sleep 5; done; sleep 20
qmp key ret; sleep 3; qmp key ret       # disk menu (only disk) -> confirm erase
( for n in 1 2 3 4 5 6 7 8; do sleep 45; qmp shot "prog-$n.png" 2>/dev/null || break; done ) &
# Done when the medium boots its loader a second time (installer reboots).
for _ in $(seq 240); do [ "$(plain | grep -c 'Welcome to WolOS Installer')" -ge 2 ] && break; sleep 10; done
stop_vm; mv serial.log serial-install.log; rm -f inst.img
vm 0
for _ in $(seq 120); do plain | grep -q "login:" && break; sleep 5; done; sleep 30
WAIT=3 ser root >/dev/null
# One command per line: the serial tty truncates long lines.
WAIT=8 ser "uname -v" "zpool list -H -o name,health,size" "kldstat | grep -E 'wolos|linux'" "pkg info | wc -l" \
  "ls -ld /usr/local /usr/local/lib" "ls /var/cache/wolos-repo" \
  "grep -cE 'script failed' /var/log/wolos-install-pkgs.log" "grep -E 'rc=|devfs|WOLOS' /var/log/wolos-install-pkgs.log" \
  "grep -c Installing /var/log/wolos-install-pkgs.log" "grep wolos_firstboot /var/log/messages" \
  "pgrep -lf 'wold|wol-login'" "pkg info -l hyprland | grep start-hyprland" \
  "ls -l /usr/local/bin/start-hyprland /usr/local/bin/Hyprland" "XDG_RUNTIME_DIR=/tmp Hyprland --version | head -3" \
  "pkg lock -l" "cat /etc/os-release" "grep -c DEBUG /var/log/bsdinstall_log" > checks.txt
# Intel GPU stack (no Intel GPU here: files only; image/hw/verify-gpu.sh proves it on hardware).
WAIT=8 ser "ls /boot/modules" "ls /boot/modules/i915_adlp_dmc_bin.ko /boot/modules/i915_tgl_huc_bin.ko" \
  "cat /boot/loader.conf.d/wolos-i915.conf" "kenv | grep i915" \
  "pkg info -e libva-intel-media-driver && echo iHD-ok" "ls /usr/local/lib/dri/iHD_drv_video.so" \
  "ls /usr/local/lib/dri/iris_dri.so" "ls /usr/local/share/vulkan/icd.d/intel_icd.x86_64.json" \
  "pkg info -e vulkan-tools mesa-demos libva-utils glmark2 vkmark && echo tools-ok" \
  "ls -l /usr/local/libexec/wolos/verify-gpu.sh" \
  "strings /boot/modules/i915kms.ko | grep -c wol_fg_pid" "ls -l /boot/modules/i915kms.ko /boot/modules/drm.ko" \
  "wol-realtime gpu caps" "kldload -n cpuctl; wol-realtime power show; echo power-rc=\$?" \
  "pkg info -e vkpeak && echo vkpeak-ok" >> checks.txt
# Boot chain: firmware entry "WolOS" first, \EFI\WolOS on the installed disk's ESP (vtbd0p1).
WAIT=8 ser "efibootmgr -v | grep -i wolos" "efibootmgr | grep BootOrder" \
  "mount -t msdosfs /dev/vtbd0p1 /mnt" "ls /mnt/EFI/WolOS" "ls /mnt/EFI/BOOT" "ls /mnt/EFI" "umount /mnt" \
  "grep -c . /boot/lua/local.lua" "grep -E 'module_verbose|boot_mute' /boot/loader.conf" > boot.txt
cat boot.txt >> checks.txt
WAIT=20 ser "cat /var/log/wolos-install-pkgs.log" > install-pkgs.log
[ "${WOLOS_KEEP_DISK:-0}${WOLOS_KEEP_INSTALL:-0}" != 00 ] && WAIT=60 ser "zpool trim -w zroot" "shutdown -p now" >/dev/null
{ echo "image sha256: $(cat "$IMG.sha256")"; ls -l "$IMG"; } >> checks.txt
stop_vm; mv serial.log serial-boot.log
other_after=$(sha256sum other.raw | cut -d' ' -f1); rm -f other.raw
b() { if eval "$2"; then echo "BOOT PASS $1"; else echo "BOOT FAIL $1"; fi; }
{ b "bystander disk byte-identical" '[ "$other_before" = "$other_after" ]'
  b "firmware entry WolOS" "grep -q 'WolOS.*EFI.WolOS.loader.efi' boot.txt"
  wol=$(grep -o 'Boot[0-9A-Fa-f]\{4\}\*\{0,1\} *WolOS' boot.txt | head -1 | cut -c5-8)
  first=$(grep BootOrder boot.txt | sed 's/.*: *//' | cut -d, -f1 | tr -d ' \r')
  b "WolOS first in BootOrder ($first = $wol)" '[ -n "$wol" ] && [ "$first" = "$wol" ]'
  b "EFI/WolOS/loader.efi + os_wolos.png" "grep -q loader.efi boot.txt && grep -q os_wolos.png boot.txt"
  b "no loader menu or countdown" "! grep -qE 'Autoboot in|Welcome to WolOS|Hit \[Enter\]' serial-boot.log"
  b "no installer on installed boot" "! grep -qiE 'bsdinstall|WolOS Installer' serial-boot.log"
} >> checks.txt
grep '^BOOT ' checks.txt
if [ "${WOLOS_KEEP_DISK:-0}" = 1 ]; then mv install.raw disk.raw; elif [ "${WOLOS_KEEP_INSTALL:-0}" != 1 ]; then rm -f install.raw; fi
echo "test-install: done ($T/checks.txt)"
"$REPO/image/vm/up.sh" >/dev/null 2>&1 && echo "test-install: build VM up"
