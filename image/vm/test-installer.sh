#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Installer end to end (ADR 0023): real disk images, the real install in a VM, each disk booted
# afterwards, what the person kept checked from outside. `install_scenario` walks the same steps
# as the app. Boot counting proves the boot: the entry is blessed only when the system came up.
# Usage: image/vm/test-installer.sh remaster BASE.iso BIN_DIR | image/vm/test-installer.sh run SCENARIO...
#        image/vm/test-installer.sh layouts BIN_DIR NAME...   (real partitioning on loop devices, no VM)
# Scenarios: erase alongside-windows alongside-linux manual manual-ext4 password tpm medium
# Env: WOLOS_E2E (work dir), WOLOS_OVMF, WOLOS_E2E_INSTALL_S, WOLOS_E2E_BOOT_S (longer without /dev/kvm).
# Needs: qemu, edk2-ovmf, swtpm, sudo, squashfs-tools, xorriso.
set -u
REPO=$(cd "$(dirname "$0")/../.." && pwd); H=$REPO/image/vm/test
W=${WOLOS_E2E:-$HOME/wolos-e2e}; OVMF=${WOLOS_OVMF:-/usr/share/edk2/x64}
ISO=$W/wolos-e2e.iso
GB=$((1000 * 1000 * 1000))
mkdir -p "$W"

remaster() { # base.iso bin_dir: this tree's installer on the base image
  local base=$1 bin=$2 r=$W/root
  sudo rm -rf "$r"; xorriso -osirrox on -indev "$base" -extract /wolos/x86_64/airootfs.sfs "$W/base.sfs" 2>/dev/null
  sudo unsquashfs -q -f -d "$r" "$W/base.sfs" >/dev/null || exit 1
  sudo install -m 755 "$bin/wol" "$bin/wol-installer" "$bin/install_scenario" "$r/usr/bin/"
  sudo cp -r "$REPO"/image/files/usr/lib/wolos/. "$r/usr/lib/wolos/"
  sudo chmod 755 "$r"/usr/lib/wolos/*
  sudo cp "$REPO/desktop/overlay/usr/share/wolos/hypr/installer.conf" "$r/usr/share/wolos/hypr/"
  sudo cp -r "$REPO"/image/arch/airootfs/etc/systemd/system/. "$r/etc/systemd/system/"
  sudo cp "$H/installer/wolos-e2e" "$r/usr/lib/wolos/wolos-e2e"
  sudo cp "$H/installer/wolos-e2e.service" "$r/etc/systemd/system/"
  sudo ln -sf ../wolos-e2e.service "$r/etc/systemd/system/multi-user.target.wants/wolos-e2e.service"
  sudo sed -i '$a m live seat' "$r/etc/sysusers.d/wolos-live.conf"
  echo 'live ALL=(root) NOPASSWD: /usr/lib/wolos/install-run, /usr/lib/wolos/install-check, /usr/bin/systemctl reboot, /usr/bin/systemctl poweroff' | sudo tee "$r/etc/sudoers.d/wolos-live" >/dev/null
  sudo pacman --noconfirm --needed --root "$r" --dbpath "$r/var/lib/pacman" -S xfsprogs ntfsprogs >/dev/null 2>&1 ||
    sudo pacman --noconfirm --needed --root "$r" --dbpath "$r/usr/lib/pacman" -S xfsprogs ntfsprogs >/dev/null 2>&1 || :
  sudo rm -f "$W/new.sfs"
  sudo mksquashfs "$r" "$W/new.sfs" -comp zstd -Xcompression-level 3 -quiet -noappend || exit 1
  sudo chown "$(id -u)" "$W/new.sfs"
  rm -f "$ISO"
  xorriso -indev "$base" -outdev "$ISO" -boot_image any replay \
    -map "$W/new.sfs" /wolos/x86_64/airootfs.sfs -end 2>/dev/null
  bsdtar -xOf "$ISO" wolos/boot/x86_64/vmlinuz-linux-wolos > "$W/kernel"
  bsdtar -xOf "$ISO" wolos/boot/x86_64/initramfs-linux-wolos.img > "$W/initrd"
  ls -la "$ISO"
}

# Runs: `with_loop FILE cmd...` with $LOOP set and the partitions showing.
with_loop() {
  local backing=$1; shift
  LOOP=$(sudo losetup -Pf --show "$backing") || exit 1
  sudo udevadm settle
  "$@"; local rc=$?
  sudo losetup -d "$LOOP"
  return $rc
}

part_hash() { # disk.raw number -> hash of the partition's bytes
  local line start size
  line=$(sfdisk -d "$1" | grep -E "^$1$2 ")
  start=$(sed -E 's/.*start= *([0-9]+).*/\1/' <<<"$line"); size=$(sed -E 's/.*size= *([0-9]+).*/\1/' <<<"$line")
  dd if="$1" bs=1M iflag=skip_bytes,count_bytes skip=$((start * 512)) count=$((size * 512)) status=none | sha256sum | cut -d' ' -f1
}

mk_windows_inner() {
  sudo mkfs.vfat -n SYSTEM "${LOOP}p1" >/dev/null
  sudo mkntfs -Q -L Windows "${LOOP}p3" >/dev/null
  local m=$D.mnt; mkdir -p "$m"
  sudo mount "${LOOP}p1" "$m"; sudo mkdir -p "$m/EFI/Microsoft/Boot"; echo fake | sudo tee "$m/EFI/Microsoft/Boot/bootmgfw.efi" >/dev/null; sudo umount "$m"
  sudo mount -t ntfs3 "${LOOP}p3" "$m" 2>/dev/null || sudo ntfs-3g "${LOOP}p3" "$m"
  sudo mkdir -p "$m/Windows/System32" "$m/Users/Alice"
  sudo dd if=/dev/urandom of="$m/Users/Alice/keep.bin" bs=1M count=256 status=none
  sudo sha256sum "$m/Users/Alice/keep.bin" | cut -d' ' -f1 > "$D.keep"
  sudo umount "$m"
}

mk_linux_inner() {
  sudo mkfs.vfat -F32 -n ESP "${LOOP}p1" >/dev/null
  sudo mkfs.ext4 -q -L linuxroot "${LOOP}p2"
  sudo mkfs.ext4 -q -L linuxhome "${LOOP}p3"
  sudo mkswap -q "${LOOP}p4"
  local m=$D.mnt; mkdir -p "$m"
  sudo mount "${LOOP}p2" "$m"
  sudo mkdir -p "$m/etc"; printf 'NAME="Fedora Linux"\nPRETTY_NAME="Fedora Linux 41"\n' | sudo tee "$m/etc/os-release" >/dev/null
  sudo umount "$m"
  sudo mount "${LOOP}p3" "$m"
  sudo mkdir -p "$m/alice"; sudo dd if=/dev/urandom of="$m/alice/keep.bin" bs=1M count=64 status=none
  sudo sha256sum "$m/alice/keep.bin" | cut -d' ' -f1 > "$D.keep"
  sudo umount "$m"
}

mk_disk() { # kind path
  D=$2; rm -f "$D"; truncate -s 64G "$D"
  case $1 in
    blank) ;;
    windows)
      sfdisk -q "$D" <<'E'
label: gpt
size=100MiB, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B
size=16MiB, type=E3C9E316-0B5C-4DB8-817D-F92DF00215AE
type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7
E
      with_loop "$D" mk_windows_inner ;;
    linux)
      sfdisk -q "$D" <<'E'
label: gpt
size=1GiB, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B
size=24GiB, type=4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709, name=linuxroot
size=30GiB, type=933AC7E1-2EB4-4F13-B844-0E14E2AEF915, name=linuxhome
type=0657FD6D-A4AB-43C4-84E5-0933C84B4F4F
E
      with_loop "$D" mk_linux_inner ;;
  esac
}

vm() { # dir extra-args...
  local d=$1; shift
  local accel="-enable-kvm -cpu host"
  [ -e /dev/kvm ] || accel="-accel tcg,thread=multi -cpu max"
  qemu-system-x86_64 $accel -smp 4 -m 6G -machine q35 \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF/OVMF_CODE.4m.fd" -drive if=pflash,format=raw,file="$d/vars.fd" \
    -drive file="$d/disk.raw",if=virtio,format=raw,discard=unmap \
    -nic none -device virtio-vga -display none -qmp "unix:$d/qmp.sock,server,nowait" ${TPM_ARGS:-} "$@"
}

tpm_start() { # dir: a TPM2 that lives through install and boot
  mkdir -p "$1/tpm"
  swtpm socket --tpm2 --tpmstate "dir=$1/tpm" --ctrl "type=unixio,path=$1/tpm.sock" --daemon --terminate
  TPM_ARGS="-chardev socket,id=c,path=$1/tpm.sock -tpmdev emulator,id=t,chardev=c -device tpm-tis,tpmdev=t"
}

label() { blkid -o value -s LABEL "$ISO"; }

install_in_vm() { # dir scenario
  local d=$1 s=$2 start=$SECONDS
  timeout "${WOLOS_E2E_INSTALL_S:-1500}" bash -c "$(declare -f vm); OVMF=$OVMF TPM_ARGS='${TPM_ARGS:-}'; vm $d -cdrom $ISO -kernel $W/kernel -initrd $W/initrd \
    -append 'archisobasedir=wolos archisolabel=$(label) wolos.e2e=$s wolos.try console=ttyS0' -serial file:$d/install.log"
  echo "$s: install took $((SECONDS - start)) s" | tee "$d/took"
}

boot_in_vm() { # dir scenario: up, a screenshot, a clean stop
  local d=$1 s=$2
  vm "$d" -serial "file:$d/boot.log" & local q=$!
  if [ "$s" = password ]; then
    for t in 1 2 3 4; do sleep 15; python3 "$H/qmp.py" "$d" shot "$d/unlock$t.png"; python3 "$H/qmp.py" "$d" keys $'white rabbit\n'; done
  fi
  sleep "${WOLOS_E2E_BOOT_S:-150}"
  python3 "$H/qmp.py" "$d" shot "$d/booted.png" || :
  kill "$q"; wait "$q" 2>/dev/null
}

blessed() { # dir: the boot entry lost its counter
  local m=$1/mnt rc=1 p; mkdir -p "$m"
  check_esp() {
    for p in "${LOOP}"p*; do
      sudo mount -o ro "$p" "$m" 2>/dev/null || continue
      if [ -d "$m/loader/entries" ]; then
        ls "$m/loader/entries" | tee "$1/entries"
        [ -e "$m/loader/entries/wolos-a.conf" ] && rc=0
      fi
      sudo umount "$m"
    done
  }
  with_loop "$1/disk.raw" check_esp "$1"
  return $rc
}

say() { if eval "$2"; then echo "E2E PASS $S: $1"; else echo "E2E FAIL $S: $1"; fi; }

untouched() { # the partitions this scenario must never write
  case $S in
    alongside-windows) echo 1 2 ;;
    alongside-linux) echo 1 2 4 ;;
    manual*) echo 4 ;;
  esac
}

kept_hashes() { # dir
  local n; for n in $(untouched); do part_hash "$1/disk.raw" "$n"; done
}

file_hash() { # dir partition fs-tool path-inside
  local d=$1 n=$2 mnt=$3 f=$4 m=$1/m
  read_file() {
    mkdir -p "$m"
    sudo $mnt "${LOOP}p$n" "$m" && sudo sha256sum "$m/$f" | cut -d' ' -f1 > "$d/keep.after"
    sudo umount "$m"
  }
  with_loop "$d/disk.raw" read_file
}

kept_checks() { # dir hashes-before: what the person kept is still there
  local d=$1
  [ -z "$(untouched)" ] || say "partitions nobody chose byte-identical" "[ \"$2\" = \"\$(kept_hashes '$d')\" ]"
  case $S in
    alongside-windows) file_hash "$d" 3 "ntfs-3g -o ro" Users/Alice/keep.bin ;;
    alongside-linux | manual*) file_hash "$d" 3 "mount -o ro" alice/keep.bin ;;
    *) return ;;
  esac
  say "the person's file intact" "cmp -s '$d/disk.raw.keep' '$d/keep.after'"
}

scenario() { # name
  S=$1; local d=$W/$S plan=$S before=
  rm -rf "$d"; mkdir -p "$d"; cp "$OVMF/OVMF_VARS.4m.fd" "$d/vars.fd"
  TPM_ARGS=
  case $S in
    erase|password|tpm) mk_disk blank "$d/disk.raw" ;;
    alongside-windows) mk_disk windows "$d/disk.raw"; plan=alongside ;;
    alongside-linux) mk_disk linux "$d/disk.raw"; plan=alongside ;;
    manual|manual-ext4) mk_disk linux "$d/disk.raw" ;;
  esac
  [ "$S" != tpm ] || tpm_start "$d"
  before=$(kept_hashes "$d")
  install_in_vm "$d" "$plan"
  grep -a 'summary:' "$d/install.log" | sed 's/.*summary: /  /' | sort -u
  say "installer said done" "grep -aq 'wolos-install: done' '$d/install.log'"
  kept_checks "$d" "$before"
  boot_in_vm "$d" "$S"
  say "booted (entry blessed)" "blessed '$d'"
  [ "$S" != manual-ext4 ] || say "one copy, no second slot" "! grep -q wolos-b '$d/entries'"
}

layouts() { # bin_dir names...: the real partitioning on loop devices, no VM and no boot
  local bin=$1; shift
  for S in "$@"; do
    local d=$W/layout-$S plan=$S before
    rm -rf "$d"; mkdir -p "$d"
    case $S in
      alongside-windows) mk_disk windows "$d/disk.raw"; plan=alongside ;;
      alongside-linux) mk_disk linux "$d/disk.raw"; plan=alongside ;;
      *) mk_disk linux "$d/disk.raw" ;;
    esac
    before=$(kept_hashes "$d")
    apply_layout() {
      sudo "$bin/wol" install probe "$LOOP" > "$d/probe.json" || return 1
      "$bin/install_scenario" "$plan" "$d/probe.json" "$d/plan" || return 1
      sudo "$bin/wol" install layout "$d/plan/plan.json" "$d/key" > "$d/layout.env"
    }
    if ! with_loop "$d/disk.raw" apply_layout; then echo "E2E FAIL $S: layout not applied"; continue; fi
    echo "E2E PASS $S: layout applied"
    grep -v '^extra_fstab=$' "$d/layout.env"
    sfdisk -d "$d/disk.raw"
    kept_checks "$d" "$before"
  done
}

medium() { # the installer's own medium is refused
  S=medium; local d=$W/$S; rm -rf "$d"; mkdir -p "$d"; cp "$OVMF/OVMF_VARS.4m.fd" "$d/vars.fd"
  mk_disk blank "$d/disk.raw"
  timeout 300 bash -c "$(declare -f vm); OVMF=$OVMF; vm $d -drive file=$ISO,if=virtio,format=raw,readonly=off,snapshot=on -kernel $W/kernel -initrd $W/initrd \
    -append 'archisobasedir=wolos archisolabel=$(label) wolos.e2e=medium wolos.try console=ttyS0' -serial file:$d/install.log"
  say "boot medium refused" "grep -aq 'medium-refused' '$d/install.log'"
}

case ${1:-} in
  remaster) shift; remaster "$@" ;;
  layouts) shift; layouts "$@" ;;
  run) shift
    for s in "$@"; do
      if [ "$s" = medium ]; then medium; else scenario "$s"; fi 2>&1 | tee "$W/$s.result" &
    done
    wait
    grep -h '^E2E\|install took' "$W"/*.result ;;
  *) echo "usage: $0 remaster BASE.iso BIN_DIR | run SCENARIO..." >&2; exit 2 ;;
esac
