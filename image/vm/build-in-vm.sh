#!/bin/sh
# Linux host: boot official FreeBSD 15.1 (BASIC-CLOUDINIT) as build host, run image/build.sh inside,
# compress it there and copy WolOS-15.1-amd64.img.xz + .sha256 back to $WOLOS_VM/out/.
# Host disk: ~40 GB free in $WOLOS_VM (VM qcow2 ~6 GB; work.raw sparse 40G holds src 3.5 + obj 8 + image build 20 GB;
#   output copy 12 GB). Override check: WOLOS_NEED_MB.
# Needs (see up.sh): qemu-system-x86_64, xz, curl, sha256sum, ssh, xorriso|genisoimage|mkisofs.
set -eu
cd "$(dirname "$0")/../.."
VM=${WOLOS_VM:-$HOME/WolOS-vm}; export WOLOS_VM=$VM
export WOLOS_NEED_MB=${WOLOS_NEED_MB:-40000}
export WOLOS_VM_PKGS="rust pkgconf gmake evdev-proto glslang hyprland gtk4-layer-shell libinput seatd mesa-libs libdrm wayland libxkbcommon"
SSH=image/vm/ssh.sh
die() { echo "build-in-vm: $*" >&2; exit 2; }
# Host disk: work.raw grows during the build and the .xz (~3 GB) is copied back. A full /home pauses the VM
# with an io-error and the build dies silently, so refuse to start under WOLOS_HOST_MIN_MB, and abort the
# in-VM build when free space drops under 1 GB (watchdog below).
HOST_MIN=${WOLOS_HOST_MIN_MB:-6000}
mkdir -p "$VM"
hostfree() { df -Pm "$VM" | awk 'NR==2{print $4}'; }
f=$(hostfree); [ "$f" -ge "$HOST_MIN" ] || die "host has $f MB free in $VM, need $HOST_MIN MB (rm old out/*.img.xz, cargo clean, trim work.raw)"
image/vm/up.sh
image/vm/sync.sh
# Raw image stays in the VM; only .xz + .sha256 come back (host disk).
( while sleep 30; do f=$(hostfree); [ "$f" -ge 1000 ] && continue
    echo "build-in-vm: host disk nearly full ($f MB free in $VM): stopping the in-VM build" >&2
    $SSH 'pkill -f image/build.sh' >/dev/null 2>&1; exit 1; done ) & WATCH=$!
trap 'kill $WATCH 2>/dev/null' EXIT
$SSH "cd /work/repo && sh kernel/fetch-src.sh /work/src && env CARGO_TARGET_DIR=/work/target WOLOS_VENDOR=/work/vendor WOLOS_LIVE=${WOLOS_LIVE:-1} \
  WOLOS_SRC=/work/src WOLOS_OBJ=/work/obj WOLOS_WORK=/work/img sh image/build.sh"
# WOLOS_ISO=1: also a hybrid ISO (Ventoy boots FreeBSD ISOs, not .img), packed before the raw is compressed.
if [ "${WOLOS_ISO:-0}" = 1 ]; then
  $SSH 'sh /work/repo/image/iso.sh /work/img/WolOS-15.1-amd64.img /work/img/WolOS-15.1-amd64.iso /work/src'
  rm -f "$VM/out/WolOS-15.1-amd64.iso"
  sh image/vm/fetch.sh /work/img/WolOS-15.1-amd64.iso "$VM/out/WolOS-15.1-amd64.iso" || die "ISO copy failed"
  $SSH 'rm -f /work/img/WolOS-15.1-amd64.iso'
  echo "iso: $VM/out/WolOS-15.1-amd64.iso"
fi
$SSH 'cd /work/img && rm -f WolOS-15.1-amd64.img.xz && xz -T0 -6 WolOS-15.1-amd64.img && sha256 -r WolOS-15.1-amd64.img.xz > WolOS-15.1-amd64.img.xz.sha256'
rm -f "$VM/out/WolOS-15.1-amd64.img.xz"
sh image/vm/fetch.sh /work/img/WolOS-15.1-amd64.img.xz "$VM/out/WolOS-15.1-amd64.img.xz" || die "copy failed"
# Intermediates (keep downloads + src/obj/target caches).
$SSH 'cd /work/img && rm -rf root stage WolOS-15.1-amd64.img.xz'   # memstick kept: no 1.2 GB refetch per build
echo "out: $VM/out/WolOS-15.1-amd64.img.xz"
