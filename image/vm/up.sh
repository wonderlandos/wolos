#!/bin/sh
# Linux host: fetch + boot official FreeBSD 15.1 BASIC-CLOUDINIT VM, wait for ssh, mount /work, install deps.
# Idempotent; reused by build-in-vm.sh. Env: WOLOS_VM (~/WolOS-vm), WOLOS_NEED_MB (free MB check, 20000),
#   WOLOS_WORK_SIZE (sparse /work disk, 40G), WOLOS_ROOT_SIZE (20G), WOLOS_VM_PKGS (extra pkgs).
# Needs: qemu-system-x86_64, xz, curl, sha256sum, ssh, xorriso|genisoimage|mkisofs. qemu-img optional (grows root).
set -eu
cd "$(dirname "$0")/../.."
VM=${WOLOS_VM:-$HOME/WolOS-vm}; export WOLOS_VM=$VM
REL=15.1-RELEASE
BASE=https://download.freebsd.org/releases/VM-IMAGES/$REL/amd64/Latest
Q=FreeBSD-$REL-amd64-BASIC-CLOUDINIT-ufs.qcow2
NEED_MB=${WOLOS_NEED_MB:-20000}
SSH=image/vm/ssh.sh
die() { echo "up.sh: $*" >&2; exit 1; }
mkdir -p "$VM/out"
running() { [ -f "$VM/vm.pid" ] && kill -0 "$(cat "$VM/vm.pid")" 2>/dev/null; }
if ! running && [ ! -f "$VM/$Q" ]; then
  free=$(df -Pm "$VM" | awk 'NR==2{print $4}')
  [ "$free" -ge "$NEED_MB" ] || { echo "up.sh: need $NEED_MB MB free in $VM, have $free MB" >&2; exit 2; }
fi
for c in qemu-system-x86_64 xz curl sha256sum ssh ssh-keygen; do command -v $c >/dev/null || die "missing $c"; done
ISO=$(command -v xorriso || command -v genisoimage || command -v mkisofs) || die "missing xorriso/genisoimage/mkisofs"

if [ ! -f "$VM/$Q" ]; then
  rm -f "$VM/ovmf_vars.fd"
  [ -f "$VM/$Q.xz" ] || { curl -fL -o "$VM/CHECKSUM.SHA256" "$BASE/CHECKSUM.SHA256"; curl -fL -o "$VM/$Q.xz" "$BASE/$Q.xz"; }
  want=$(awk -v f="($Q.xz)" '$2==f{print $4}' "$VM/CHECKSUM.SHA256")
  [ -n "$want" ] && [ "$(sha256sum "$VM/$Q.xz" | cut -d' ' -f1)" = "$want" ] || die "checksum mismatch $Q.xz"
  xz -dk "$VM/$Q.xz"   # keep .xz: nuageinit runs only on first boot; rm the qcow2 to reprovision
  ! command -v qemu-img >/dev/null || qemu-img resize "$VM/$Q" 20G
fi
[ -f "$VM/vmkey" ] || ssh-keygen -q -t ed25519 -N '' -f "$VM/vmkey"
[ -f "$VM/work.raw" ] || truncate -s "${WOLOS_WORK_SIZE:-40G}" "$VM/work.raw"
if [ ! -f "$VM/seed.iso" ]; then  # NoCloud seed for nuageinit: root key + sshd
  S=$(mktemp -d); printf 'instance-id: wolos-build-2\nlocal-hostname: wolos-build\n' > "$S/meta-data"
  # FreeBSD sshd defaults to PermitRootLogin no: allow key-only root login.
  K=$(cat "$VM/vmkey.pub")
  cat > "$S/user-data" <<EOC
#cloud-config
users:
  - name: root
    ssh_authorized_keys:
      - $K
runcmd:
  - sed -i '' -e 's/^#*PermitRootLogin.*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config
  - service sshd restart
EOC
  case $ISO in
  *xorriso) xorriso -as mkisofs -o "$VM/seed.iso" -V cidata -J -r "$S" ;;
  *) "$ISO" -o "$VM/seed.iso" -V cidata -J -r "$S" ;;
  esac; rm -rf "$S"
fi
running || WOLOS_VM_DISK="$VM/$Q" image/vm/run.sh
i=0; until $SSH true 2>/dev/null; do i=$((i+1)); [ $i -lt 150 ] || die "VM ssh timeout (see $VM/serial.log)"; sleep 2; done
# No qemu-img: grow the root disk live via the QEMU monitor (sparse; idempotent).
if ! command -v qemu-img >/dev/null && command -v python3 >/dev/null; then
  python3 -c 'import socket,sys,time; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); time.sleep(.3); s.send(("block_resize sys %s\n" % sys.argv[2]).encode()); time.sleep(.5)' \
    "$VM/mon.sock" "${WOLOS_ROOT_SIZE:-20G}"
  $SSH 'p=$(gpart show -p vtbd0 | awk "/freebsd-ufs/{print \$3}"); gpart recover vtbd0 >/dev/null; gpart resize -i ${p#vtbd0p} vtbd0 >/dev/null 2>&1 && growfs -y / >/dev/null 2>&1 || :'
fi
$SSH 'set -e
  service growfs onestart >/dev/null 2>&1 || :
  if ! mount | grep -q " /work "; then
    [ -e /dev/vtbd1p1 ] || { gpart create -s gpt vtbd1; gpart add -t freebsd-ufs vtbd1; newfs -U /dev/vtbd1p1; }
    mkdir -p /work; mount /dev/vtbd1p1 /work; fi
  grep -q " /work " /etc/fstab || echo "/dev/vtbd1p1 /work ufs rw 2 2" >> /etc/fstab'
$SSH "env ASSUME_ALWAYS_YES=yes pkg install -y git ${WOLOS_VM_PKGS:-} >/dev/null"
echo "up: $SSH ready, /work mounted"
