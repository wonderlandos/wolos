#!/bin/sh
# Boot the installed WolOS disk with the host Intel iGPU passed through (laptop panel shows WolOS).
# Host must be booted with: intel_iommu=on iommu=pt vfio-pci.ids=<igpu id> modprobe.blacklist=i915,xe
# (rEFInd entry "WolOS GPU passthrough"). Keyboard via evdev, USB mice and (if isolated) Wi-Fi passed through.
# Runs as root (vfio + /dev/input). When WolOS powers off, the host reboots back to its normal entry.
# Env: WOLOS_VM (default ~klee/WolOS-vm), WOLOS_GPU (default 0000:00:02.0), WOLOS_GPU_MEM (default 6G).
set -eu
VM=${WOLOS_VM:-/home/klee/WolOS-vm}; T=$VM/test; OVMF=${WOLOS_OVMF:-/usr/share/edk2/x64}
GPU=${WOLOS_GPU:-0000:00:02.0}
[ -f "$T/disk.raw" ] || { echo "run-gpu: no $T/disk.raw (WOLOS_KEEP_DISK=1 image/vm/test-install.sh)"; exit 1; }
modprobe vfio-pci   # not in the initramfs: vfio-pci.ids= binds the iGPU when it loads
drv=$(basename "$(readlink -f /sys/bus/pci/devices/$GPU/driver)")
[ "$drv" = vfio-pci ] || { echo "run-gpu: $GPU is bound to $drv, not vfio-pci (boot the passthrough entry)"; exit 1; }
[ -f "$T/vars-gpu.fd" ] || cp "$OVMF/OVMF_VARS.4m.fd" "$T/vars-gpu.fd"

# Keyboard: evdev with grab (both Ctrl keys hand it back to the host). USB mice: real USB passthrough,
# which QEMU re-attaches when the mouse re-enumerates (evdev lost it: the mouse froze mid-session).
# Touchpad (i2c, absolute) is not passed: evdev forwards only relative motion.
input=
for d in /dev/input/by-path/*-event-kbd; do
  [ -e "$d" ] || continue
  input="$input -object input-linux,id=in$(echo "$d" | cksum | cut -d' ' -f1),evdev=$d,grab_all=on,repeat=on"
done
usb=
for u in /sys/bus/usb/devices/*; do
  [ -f "$u/idVendor" ] || continue
  grep -qx 02 "$u"/*:*/bInterfaceProtocol 2>/dev/null && grep -qx 03 "$u"/*:*/bInterfaceClass 2>/dev/null || continue
  echo on > "$u/power/control" 2>/dev/null || :   # no autosuspend disconnects
  usb="$usb -device usb-host,bus=xhci.0,vendorid=0x$(cat "$u/idVendor"),productid=0x$(cat "$u/idProduct")"
done

# Wi-Fi: passed through when it is alone in its IOMMU group (Linux does not use it while WolOS runs).
WIFI=${WOLOS_WIFI:-0000:00:14.3}
wifi=
if [ -e "/sys/bus/pci/devices/$WIFI/iommu_group" ] &&
   [ "$(ls "/sys/bus/pci/devices/$WIFI/iommu_group/devices" | wc -l)" -eq 1 ]; then
  [ -e "/sys/bus/pci/devices/$WIFI/driver" ] && echo "$WIFI" > "/sys/bus/pci/devices/$WIFI/driver/unbind"
  echo vfio-pci > "/sys/bus/pci/devices/$WIFI/driver_override"
  echo "$WIFI" > /sys/bus/pci/drivers_probe
  [ "$(basename "$(readlink -f "/sys/bus/pci/devices/$WIFI/driver")")" = vfio-pci ] &&
    wifi="-device vfio-pci,host=$WIFI,addr=06.0"
fi
echo "run-gpu: wifi=${wifi:-none} usb=${usb:-none}"

# i440fx (pc): x-igd-lpc needs a free 1f.0 to clone the host LPC bridge (i915 reads the PCH id there).
# The iGPU must take guest slot 02.0 before any other device (i915 and x-igd-lpc expect it there).
GPUDEV=${WOLOS_GPU_DEV:-vfio-pci,host=$GPU,addr=02.0,x-igd-opregion=on,x-igd-lpc=on}
start=$(date +%s)
# shellcheck disable=SC2086
qemu-system-x86_64 -enable-kvm -cpu host -smp "$(nproc)" -m "${WOLOS_GPU_MEM:-6G}" -machine pc \
  -vga none -display none -device "$GPUDEV" \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF/OVMF_CODE.4m.fd" -drive if=pflash,format=raw,file="$T/vars-gpu.fd" \
  -drive file="$T/disk.raw",if=none,id=d0,format=raw,discard=unmap -device virtio-blk-pci,drive=d0,addr=05.0 \
  -netdev user,id=n0 -device virtio-net-pci,netdev=n0,addr=04.0 \
  -device qemu-xhci,id=xhci,addr=07.0 $usb $wifi \
  -serial file:"$T/serial-gpu.log" $input || true
# A qemu that dies at once leaves the host as it is (no reboot loop into a dead VM).
[ $(( $(date +%s) - start )) -lt 30 ] && { echo "run-gpu: qemu exited early; staying on the host"; exit 1; }
systemctl reboot
