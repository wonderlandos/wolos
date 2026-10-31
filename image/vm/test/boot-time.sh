#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Boot an installed disk N times in QEMU/KVM and collect what each boot measured about itself:
# systemd-analyze time, blame, critical chain, plot, and the wolos-boot stage lines (ADR 0012).
# A throwaway unit arrives as a systemd credential over SMBIOS, so the sealed slot and the signed
# image stay untouched; results are read back from @var.
# Usage: image/vm/test/boot-time.sh DISK.raw OUTDIR [RUNS]   (needs sudo for the loop mount)
# Env: WOLOS_OVMF_CODE, WOLOS_OVMF_VARS (firmware), WOLOS_QEMU_EXTRA, WOLOS_TIMEOUT (per boot, 240).
set -eu
DISK=$1; OUT=$2; RUNS=${3:-5}
CODE=${WOLOS_OVMF_CODE:-/usr/share/edk2/x64/OVMF_CODE.4m.fd}
VARS=${WOLOS_OVMF_VARS:-/usr/share/edk2/x64/OVMF_VARS.4m.fd}
mkdir -p "$OUT"
b64() { base64 -w0; }
unit() {
  cat <<'E'
[Unit]
Description=Record this boot's timings, then power off
After=multi-user.target
[Service]
Type=simple
ExecStart=/bin/sh -c 'timeout 120 systemctl is-system-running --wait; sleep 5; n=$(ls /var/lib/wolos-measure 2>/dev/null | wc -l); d=/var/lib/wolos-measure/$n; mkdir -p $d; systemd-analyze > $d/time; systemd-analyze blame > $d/blame; systemd-analyze critical-chain > $d/chain; systemd-analyze plot > $d/plot.svg; journalctl -b --no-pager | grep -a wolos-boot > $d/stages; systemctl --failed --no-legend > $d/failed; sync; systemctl poweroff'
E
}
UNIT=$(unit | b64)
DROP=$(printf '[Unit]\nWants=wolos-measure.service\n' | b64)
cp "$VARS" "$OUT/vars.fd"
i=0
while [ "$i" -lt "$RUNS" ]; do
  s=$(date +%s.%N)
  # shellcheck disable=SC2086
  timeout "${WOLOS_TIMEOUT:-240}" qemu-system-x86_64 -enable-kvm -cpu host -smp 4 -m 4G -machine q35,smm=on \
    -global driver=cfi.pflash01,property=secure,value=on \
    -drive if=pflash,format=raw,readonly=on,file="$CODE" -drive if=pflash,format=raw,file="$OUT/vars.fd" \
    -drive file="$DISK",if=virtio,format=raw,discard=unmap -nic none -device virtio-vga -display none \
    -smbios "type=11,value=io.systemd.credential.binary:systemd.extra-unit.wolos-measure.service=$UNIT" \
    -smbios "type=11,value=io.systemd.credential.binary:systemd.unit-dropin.multi-user.target=$DROP" \
    -serial file:"$OUT/serial-$i.log" ${WOLOS_QEMU_EXTRA:-} || echo "run $i: timeout"
  e=$(date +%s.%N); echo "run $i wall $(echo "$e - $s" | bc)" | tee -a "$OUT/wall.txt"
  i=$((i + 1))
done
L=$(sudo losetup -Pf --show "$DISK"); M=$(mktemp -d)
sudo mount -o subvol=/@var "${L}p2" "$M"
sudo cp -r "$M/lib/wolos-measure/." "$OUT/"; sudo chown -R "$(id -u)" "$OUT"
sudo rm -rf "$M/lib/wolos-measure"
sudo umount "$M"; sudo losetup -d "$L"
for d in "$OUT"/[0-9]*; do head -1 "$d/time"; done
