#!/bin/sh
# Boot the WolOS build/test VM headless (KVM, virtio, UEFI if OVMF found).
# SSH: image/vm/ssh.sh   VNC: 127.0.0.1:5900 (WOLOS_VM_VNC=1)
# Env: WOLOS_VM (state dir, default ~/WolOS-vm), WOLOS_VM_DISK (boot disk), WOLOS_VM_SEED (cloud-init ISO),
#      The system disk has bootindex=0 so an extra bootable disk (WolOS) never boots instead.
#      WOLOS_VM_MEM (6G), WOLOS_VM_CPUS (nproc), WOLOS_VM_EXTRA (extra qemu args, e.g. a disk for update-disk.sh).
set -eu
VM=${WOLOS_VM:-$HOME/WolOS-vm}
DISK=${WOLOS_VM_DISK:-$VM/FreeBSD-15.1-RELEASE-amd64-BASIC-CLOUDINIT-ufs.qcow2}
[ -f "$DISK" ] || { echo "no disk $DISK (image/vm/build-in-vm.sh fetches it)" >&2; exit 1; }
[ -e /dev/kvm ] || { echo "/dev/kvm missing" >&2; exit 1; }
FW=
for f in /usr/share/edk2/x64/OVMF_CODE.4m.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2-ovmf/x64/OVMF_CODE.fd /usr/share/qemu/ovmf-x86_64-code.bin; do
  v=$(echo "$f" | sed 's/CODE/VARS/'); [ -f "$f" ] && [ -f "$v" ] || continue
  [ -f "$VM/ovmf_vars.fd" ] || cp "$v" "$VM/ovmf_vars.fd"   # OVMF needs a writable VARS pflash
  FW="-drive if=pflash,format=raw,readonly=on,file=$f -drive if=pflash,format=raw,file=$VM/ovmf_vars.fd"; break
done
SEED=${WOLOS_VM_SEED:-$VM/seed.iso}; [ -f "$SEED" ] && SEED="-drive file=$SEED,media=cdrom,readonly=on" || SEED=
WORKD=; [ -f "$VM/work.raw" ] && WORKD="-drive file=$VM/work.raw,if=virtio,format=raw,discard=unmap"
DISP="-display none"; [ "${WOLOS_VM_VNC:-1}" = 1 ] && DISP="-display none -vnc 127.0.0.1:0"
# shellcheck disable=SC2086
exec qemu-system-x86_64 -enable-kvm -cpu host -smp "${WOLOS_VM_CPUS:-$(nproc)}" -m "${WOLOS_VM_MEM:-6G}" \
  -machine q35 $FW \
  -drive file="$DISK",if=none,id=sys,format=qcow2,discard=unmap -device virtio-blk-pci,drive=sys,bootindex=0 $WORKD $SEED \
  -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:2222-:22 \
  -device virtio-rng-pci -vga std $DISP \
  -serial file:"$VM/serial.log" -monitor unix:"$VM/mon.sock",server,nowait \
  -daemonize -pidfile "$VM/vm.pid" ${WOLOS_VM_EXTRA:-}
