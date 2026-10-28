#!/bin/sh
# Boot the newest WolOS ISO (live desktop) in QEMU/KVM: OVMF, virtio-vga-gl (Hyprland runs), a window.
# Usage: image/vm/run.sh [ISO]   Env: WOLOS_VM (default ~/WolOS-vm), WOLOS_MEM (6G), WOLOS_DISPLAY (gtk,gl=on).
set -eu
VM=${WOLOS_VM:-$HOME/WolOS-vm}; T=$VM/test
OVMF=${WOLOS_OVMF:-/usr/share/edk2/x64}
command -v qemu-system-x86_64 >/dev/null || { echo "$0: qemu-system-x86_64 missing (pacman -S qemu-desktop)" >&2; exit 1; }
[ -f "$OVMF/OVMF_CODE.4m.fd" ] || { echo "$0: no OVMF in $OVMF (pacman -S edk2-ovmf)" >&2; exit 1; }
mkdir -p "$T"
ISO=${1:-$(ls -t "$VM"/out/wolos-*.iso 2>/dev/null | head -1)}
[ -f "$ISO" ] || { echo "run.sh: no ISO in $VM/out (image/vm/build.sh)" >&2; exit 1; }
[ -f "$T/vars-live.fd" ] || cp "$OVMF/OVMF_VARS.4m.fd" "$T/vars-live.fd"
exec qemu-system-x86_64 -enable-kvm -cpu host -smp "$(nproc)" -m "${WOLOS_MEM:-6G}" -machine q35 \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF/OVMF_CODE.4m.fd" -drive if=pflash,format=raw,file="$T/vars-live.fd" \
  -cdrom "$ISO" -device virtio-vga-gl -display "${WOLOS_DISPLAY:-gtk,gl=on}" \
  -nic user,model=virtio-net-pci -device qemu-xhci -device usb-tablet -audiodev pipewire,id=a0 -device intel-hda -device hda-duplex,audiodev=a0
