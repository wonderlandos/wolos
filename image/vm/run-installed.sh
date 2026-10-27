#!/bin/sh
# Boot the installed test disk kept by WOLOS_KEEP_DISK=1 image/vm/test-install.sh (UEFI, user-mode NIC).
# Env: WOLOS_TEST_DISK / WOLOS_TEST_VARS boot a throwaway copy (default disk.raw / vars.fd).
# Usage: image/vm/run-installed.sh [extra qemu args...]   e.g. -device virtio-vga -display egl-headless
# Console: serial via image/vm/test/serial.py $WOLOS_VM/test; screen via image/vm/test/qmp.py (VNC :1).
set -eu
VM=${WOLOS_VM:-$HOME/WolOS-vm}; T=$VM/test; OVMF=${WOLOS_OVMF:-/usr/share/edk2/x64}
D=${WOLOS_TEST_DISK:-disk.raw}; V=${WOLOS_TEST_VARS:-vars.fd}
cd "$T"; [ -f "$D" ] || { echo "run-installed: no $T/$D (WOLOS_KEEP_DISK=1 image/vm/test-install.sh)"; exit 1; }
[ -f "$V" ] || cp vars.fd "$V" 2>/dev/null || cp "$OVMF/OVMF_VARS.4m.fd" "$V"
VGA="-vga std"; case " $* " in *" -device "*vga*|*" -vga "*) VGA= ;; esac
DISP="-display none"; case " $* " in *" -display "*) DISP= ;; esac
# shellcheck disable=SC2086
exec qemu-system-x86_64 -enable-kvm -cpu host -smp 4 -m ${WOLOS_TEST_MEM:-3G} -machine q35 \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF/OVMF_CODE.4m.fd" -drive if=pflash,format=raw,file="$V" \
  -drive file="$D",if=none,id=d0,format=raw,discard=unmap -device virtio-blk-pci,drive=d0 \
  -nic user,model=virtio-net-pci $VGA $DISP -vnc 127.0.0.1:1 \
  -chardev socket,id=s0,path=serial.sock,server=on,wait=off,logfile=serial.log,logappend=on -serial chardev:s0 \
  -qmp unix:qmp.sock,server,nowait -daemonize -pidfile vm.pid "$@"
