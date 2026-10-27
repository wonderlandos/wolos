#!/bin/sh
# Update the installed WolOS disk ($WOLOS_VM/test/disk.raw, the one run-gpu.sh boots) to this checkout
# WITHOUT reinstalling: a new boot environment gets the new build, the old one stays bootable as fallback
# (loader menu "Boot Environments" or Settings > Restore). Users, homes, accounts and rc.conf are kept.
# Usage: image/vm/update-disk.sh [--dry-run]   Env: WOLOS_UPD_DISK (default $WOLOS_VM/test/disk.raw),
#   WOLOS_UPD_KERNEL=0 (skip WolKernel), WOLOS_HYPR_FORK / WOLOS_DRM_FORK as build.sh.
# Disk is attached to the build VM in place (no copy). Host disk: work.raw grows ~6 GB (stage + packages).
set -eu
cd "$(dirname "$0")/../.."
VM=${WOLOS_VM:-$HOME/WolOS-vm}; export WOLOS_VM=$VM
DISK=${WOLOS_UPD_DISK:-$VM/test/disk.raw}
BE=wolos-update-$(date +%Y%m%d-%H%M)
SER=WOLUPD; SSH=image/vm/ssh.sh
DRY=0; [ "${1:-}" = --dry-run ] && DRY=1
die() { echo "update-disk: $*" >&2; exit 2; }
[ -f "$DISK" ] || die "no $DISK"
# Never attach a disk that is in use: any qemu with it on the command line, or a live test/GPU VM pid file.
inuse() {
  for p in /proc/[0-9]*; do
    case $(tr '\0' '\n' < "$p/cmdline" 2>/dev/null | head -1) in
    *qemu-system*) tr '\0' '\n' < "$p/cmdline" | grep -qF "$(basename "$DISK")" && return 0 ;;
    esac
  done
  for f in "$(dirname "$DISK")/vm.pid" "$VM/test/vm.pid"; do
    [ -f "$f" ] && kill -0 "$(cat "$f")" 2>/dev/null && return 0
  done
  ! command -v fuser >/dev/null || [ -z "$(fuser "$DISK" 2>/dev/null)" ] || return 0
  return 1
}
inuse && die "$DISK is in use by a running VM: power WolOS off first"
HOST_MIN=${WOLOS_HOST_MIN_MB:-6000}
f=$(df -Pm "$VM" | awk 'NR==2{print $4}'); [ "$f" -ge "$HOST_MIN" ] || die "host has $f MB free in $VM, need $HOST_MIN MB"
KSRC=; [ "${WOLOS_UPD_KERNEL:-1}" = 1 ] && KSRC=/work/src
cat <<P
update-disk plan ($([ $DRY = 1 ] && echo dry run || echo live)):
  disk        $DISK (attached in place, serial $SER; no copy)
  build VM    power off if running, boot with the disk, sync repo, WOLOS_STAGE_ONLY=1 image/build.sh
  kernel      $([ -n "$KSRC" ] && echo "WolKernel from $KSRC" || echo "unchanged (WOLOS_UPD_KERNEL=0)")
  pool        found by its ZFS label GUID on that disk; zpool import -f -R /mnt/upd -N
  new BE      $BE  (bectl create -e <active>; old BE kept)
  installs    stage (Rust binaries, shell, desktop/overlay, image/files, design, plugins, kernel) +
              packages-base.txt + desktop/packages.txt via install-pkgs from the offline repo
  untouched   /home, /usr/home, /root, /var, master.passwd/passwd/group, rc.conf(.local), fstab, hosts,
              boot/loader.conf, login.json, users' dotfiles
  then        bectl activate $BE, zpool export, build VM power off
P
[ "$DRY" = 1 ] && exit 0
# One VM at a time (15 GB host): the build VM restarts with the disk attached, then powers off.
$SSH 'shutdown -p now' >/dev/null 2>&1 || :
p=$(cat "$VM/vm.pid" 2>/dev/null || :); while [ -n "$p" ] && kill -0 "$p" 2>/dev/null; do sleep 3; done
inuse && die "$DISK became in use"
export WOLOS_VM_EXTRA="-drive file=$DISK,if=none,id=upd,format=raw,discard=unmap -device virtio-blk-pci,drive=upd,serial=$SER,addr=0x10"
stop() { $SSH 'shutdown -p now' >/dev/null 2>&1 || :; }
trap stop EXIT
image/vm/up.sh
unset WOLOS_VM_EXTRA
image/vm/sync.sh
[ -n "$KSRC" ] && $SSH "cd /work/repo && sh kernel/fetch-src.sh $KSRC"
$SSH "cd /work/repo && env CARGO_TARGET_DIR=/work/target WOLOS_VENDOR=/work/vendor WOLOS_STAGE_ONLY=1 WOLOS_NEED_MB=8000 \
  ${KSRC:+WOLOS_SRC=$KSRC} WOLOS_HYPR_FORK=${WOLOS_HYPR_FORK:-1} WOLOS_DRM_FORK=${WOLOS_DRM_FORK:-1} WOLOS_WORK=/work/img sh image/build.sh"
$SSH "env WOLOS_WORK=/work/img sh /work/repo/image/vm/update-be.sh $SER $BE"
$SSH 'cat /work/img/update-replaced.txt' > "$VM/test/update-replaced.txt" || :
$SSH 'cd /work/img && chflags -R noschg stage 2>/dev/null; rm -rf stage pkgrepo pkgrepo.conf' || :
echo "update-disk: done. Boots $BE next; files replaced: $VM/test/update-replaced.txt"
echo "rollback (in WolOS): sudo bectl activate <previous BE> && reboot   (bectl list shows it; or pick it in the loader menu)"
