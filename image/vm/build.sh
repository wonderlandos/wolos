#!/bin/sh
# Build the WolOS ISO on this Linux host into $WOLOS_VM/out (image/build.sh: [wolos] repo + mkarchiso,
# or an archlinux podman container). Fails fast when the host is short of disk.
# Env: WOLOS_VM (default ~/WolOS-vm), WOLOS_NEED_MB (default 20000), plus image/build.sh's.
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)
VM=${WOLOS_VM:-$HOME/WolOS-vm}
NEED=${WOLOS_NEED_MB:-20000}
mkdir -p "$VM"
free=$(df -Pm "$VM" | awk 'NR == 2 {print $4}')
if [ "$free" -lt "$NEED" ]; then
  echo "build.sh: host has $free MB free in $VM, needs $NEED MB (trim $VM/build, target/)" >&2
  exit 2
fi
WOLOS_WORK=$VM/build "$REPO/image/build.sh"
mkdir -p "$VM/out"
mv "$VM"/build/out/wolos-*.iso "$VM"/build/out/wolos-*.json "$VM/out/"
ls -l "$VM/out"
