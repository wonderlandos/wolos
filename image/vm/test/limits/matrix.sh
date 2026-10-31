#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# One throwaway KVM guest per run, memory and CPUs capped, images in DIR, no network device.
# matrix.sh DIR OUT "MODES" "KINDS"; needs DIR/{bzImage,base.raw,files.tar}.
D=$1 OUT=$2; mkdir -p "$OUT"
CPUS=8 MEM=8G
ACCEL="-enable-kvm -cpu host"
[ -e /dev/kvm ] || ACCEL="-accel tcg,thread=multi,tb-size=1024 -cpu max"
for mode in $3; do for kind in $4; do
	n=$kind-$mode; ov=$D/$n.qcow2; rm -f "$ov" "$OUT/$n.data"
	qemu-img create -q -f qcow2 -F raw -b "$D/base.raw" "$ov"
	timeout -k 5 ${TIMEOUT:-420} qemu-system-x86_64 $ACCEL -smp $CPUS -m $MEM -nic none \
		-nographic -monitor none -no-reboot \
		-kernel "$D/bzImage" \
		-append "root=/dev/vda rootflags=subvol=@ rw console=ttyS0 quiet kb=$kind kbmode=$mode $KBX" \
		-drive file="$ov",if=virtio,format=qcow2,cache=unsafe \
		-drive file="$D/files.tar",if=virtio,format=raw,readonly=on \
		-serial file:"$OUT/$n.console" -serial file:"$OUT/$n.data" </dev/null >/dev/null 2>&1
	echo "$n rc=$?"
	rm -f "$ov"
done; done
