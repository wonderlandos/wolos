#!/bin/sh
# Boot the installed disk ($WOLOS_VM/test/disk.raw from WOLOS_KEEP_DISK=1 image/vm/test-install.sh) in
# QEMU/KVM with OVMF and virtio-vga-gl. Serial console in $T/serial-installed.log.
# Env: WOLOS_VM (default ~/WolOS-vm), WOLOS_MEM (6G), WOLOS_DISPLAY (gtk,gl=on; egl-headless for CI).
set -eu
VM=${WOLOS_VM:-$HOME/WolOS-vm}; T=$VM/test
OVMF=${WOLOS_OVMF:-/usr/share/edk2/x64}
command -v qemu-system-x86_64 >/dev/null || { echo "$0: qemu-system-x86_64 missing (pacman -S qemu-desktop)" >&2; exit 1; }
[ -f "$OVMF/OVMF_CODE.4m.fd" ] || { echo "$0: no OVMF in $OVMF (pacman -S edk2-ovmf)" >&2; exit 1; }
mkdir -p "$T"
[ -f "$T/disk.raw" ] || { echo "run-installed.sh: no $T/disk.raw (WOLOS_KEEP_DISK=1 image/vm/test-install.sh)" >&2; exit 1; }
[ -f "$T/vars.fd" ] || cp "$OVMF/OVMF_VARS.4m.fd" "$T/vars.fd"
echo $$ > "$T/vm.pid"
exec qemu-system-x86_64 -enable-kvm -cpu host -smp "$(nproc)" -m "${WOLOS_MEM:-6G}" -machine q35 \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF/OVMF_CODE.4m.fd" -drive if=pflash,format=raw,file="$T/vars.fd" \
  -drive file="$T/disk.raw",if=virtio,format=raw,discard=unmap \
  -device virtio-vga-gl -display "${WOLOS_DISPLAY:-gtk,gl=on}" \
  -nic user,model=virtio-net-pci -device qemu-xhci -device usb-tablet \
  -serial file:"$T/serial-installed.log" -qmp unix:"$T/qmp.sock",server,nowait
