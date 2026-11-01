#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Secure Boot layout of the live ISO's EFI partition (ADR 0025), edited in place with mtools.
#   sb-layout.sh layout ISO REPO_DIR   shim first, systemd-boot second, one UKI (image/build.sh,
#                                      WOLOS_SB_LAYOUT=1)
#   sb-layout.sh sign ISO KEY CERT     sign systemd-boot and the UKI, add the certificate (iso.yml,
#                                      once WOLOS_SB_KEY / WOLOS_SB_CERT exist)
# layout: EFI/BOOT/BOOTX64.EFI becomes the pinned shim (Fedora's build, signed by Microsoft) with
# MokManager (mmx64.efi) next to it; systemd-boot moves to EFI/BOOT/grubx64.efi, the second stage
# that shim starts. The kernel, the initramfs and the two loader entries become one UKI with two
# profiles, Install WolOS and Try WolOS, in EFI/Linux/wolos.efi. Unsigned it boots with Secure
# Boot off (shim then starts anything). sign: with the WolOS Secure Boot key, systemd-boot and the
# UKI carry its signature and /wolos-secureboot.cer is the certificate MokManager enrolls ("Enroll
# key from disk") on the first boot with Secure Boot on.
set -euo pipefail
die() { echo "sb-layout.sh: $*" >&2; exit 1; }
SHIM_SHA=351e131d3c4a636704e9cae7d1296baf8a05beca9922cfebbfa48933c9bf1ad3 # image/repo/wolos-shim
MM_SHA=ed4442fa88cccba3a9e6a32d21a8f4c56bf5aab184f1fb9aadde8d3a9d04dc27
UKIFY=$(command -v ukify || echo /usr/lib/systemd/ukify)
export MTOOLS_SKIP_CHECK=1

# The EFI partition is the hybrid ISO's second partition: mtools reads it at its byte offset.
esp() {
  local start
  start=$(sfdisk -J "$1" | python3 -c '
import json, sys
p = json.load(sys.stdin)["partitiontable"]["partitions"]
print(next(x["start"] for x in p if x["type"].lower() in ("ef", "c12a7328-f81f-11d2-ba4b-00a0c93ec93b")))')
  echo "$1@@$((start * 512))"
}

layout() {
  local iso=$1 rd=$2 e t shim k i
  e=$(esp "$iso"); t=$(mktemp -d); trap 'rm -rf "$t"' RETURN
  shim=$(ls "$rd"/x86_64/wolos-shim-*.pkg.tar.zst 2>/dev/null | head -1)
  [ -n "$shim" ] || die "no wolos-shim package in $rd/x86_64"
  bsdtar -xf "$shim" -C "$t" usr/share/wolos/shim
  printf '%s  %s\n' "$SHIM_SHA" "$t/usr/share/wolos/shim/shimx64.efi" "$MM_SHA" "$t/usr/share/wolos/shim/mmx64.efi" |
    sha256sum -c --quiet || die "shim does not match its pin"
  mcopy -n -i "$e" ::/EFI/BOOT/BOOTx64.EFI "$t/systemd-boot.efi"
  mcopy -n -i "$e" ::/loader/entries/01-wolos.conf ::/loader/entries/02-wolos-try.conf "$t/"
  k=$(sed -n 's/^linux *//p' "$t/01-wolos.conf"); i=$(sed -n 's/^initrd *//p' "$t/01-wolos.conf")
  mcopy -n -i "$e" "::$k" "$t/vmlinuz"; mcopy -n -i "$e" "::$i" "$t/initrd"
  opt() { sed -n 's/^options *//p' "$t/$1"; }
  printf 'NAME="WolOS"\nPRETTY_NAME="WolOS"\nID=wolos\nID_LIKE=arch\n' > "$t/os-release"
  # Profile 0 (Install) takes the base command line; profile 1 (Try) has its own.
  "$UKIFY" build --profile=$'ID=install\nTITLE=Install WolOS' --output="$t/p0.efi" >/dev/null
  "$UKIFY" build --profile=$'ID=try\nTITLE=Try WolOS' --cmdline="$(opt 02-wolos-try.conf)" --output="$t/p1.efi" >/dev/null
  "$UKIFY" build --linux="$t/vmlinuz" --initrd="$t/initrd" --cmdline="$(opt 01-wolos.conf)" \
    --os-release="@$t/os-release" --join-profile="$t/p0.efi" --join-profile="$t/p1.efi" \
    --output="$t/wolos.efi" >/dev/null
  # The UKI holds kernel and initramfs: their loose copies and the entries leave the partition.
  mdel -i "$e" "::$k" "::$i" ::/loader/entries/01-wolos.conf ::/loader/entries/02-wolos-try.conf
  # No default entry name: profile 0 (Install) sorts first, as 01-wolos.conf did.
  mcopy -n -i "$e" ::/loader/loader.conf "$t/loader.conf"
  sed -i '/^default /d' "$t/loader.conf"
  mcopy -o -i "$e" "$t/loader.conf" ::/loader/loader.conf
  mmd -D s -i "$e" ::/EFI/Linux 2>/dev/null || :
  mcopy -o -i "$e" "$t/wolos.efi" ::/EFI/Linux/wolos.efi
  mcopy -o -i "$e" "$t/systemd-boot.efi" ::/EFI/BOOT/grubx64.efi
  mcopy -o -i "$e" "$t/usr/share/wolos/shim/shimx64.efi" ::/EFI/BOOT/BOOTX64.EFI
  mcopy -o -i "$e" "$t/usr/share/wolos/shim/mmx64.efi" ::/EFI/BOOT/mmx64.efi
  echo "sb-layout: shim -> systemd-boot -> EFI/Linux/wolos.efi (2 profiles) in $iso"
}

sign() {
  local iso=$1 key=$2 cert=$3 e t f
  e=$(esp "$iso"); t=$(mktemp -d); trap 'rm -rf "$t"' RETURN
  mcopy -n -i "$e" ::/EFI/Linux/wolos.efi "$t/wolos.efi" 2>/dev/null || die "no Secure Boot layout in $iso (sb-layout.sh layout)"
  mcopy -n -i "$e" ::/EFI/BOOT/grubx64.efi "$t/grubx64.efi"
  for f in wolos.efi grubx64.efi; do
    sbsign --key "$key" --cert "$cert" --output "$t/$f.signed" "$t/$f" 2>/dev/null
    sbverify --cert "$cert" "$t/$f.signed" >/dev/null || die "$f: signature does not verify"
  done
  openssl x509 -in "$cert" -outform DER -out "$t/wolos-secureboot.cer"
  mcopy -o -i "$e" "$t/wolos.efi.signed" ::/EFI/Linux/wolos.efi
  mcopy -o -i "$e" "$t/grubx64.efi.signed" ::/EFI/BOOT/grubx64.efi
  mcopy -o -i "$e" "$t/wolos-secureboot.cer" ::/wolos-secureboot.cer
  echo "sb-layout: signed systemd-boot and the UKI, /wolos-secureboot.cer added in $iso"
}

case ${1:-} in
  layout) [ $# = 3 ] || die "usage: $0 layout ISO REPO_DIR"; layout "$2" "$3" ;;
  sign) [ $# = 4 ] || die "usage: $0 sign ISO KEY CERT"; sign "$2" "$3" "$4" ;;
  *) die "usage: $0 layout ISO REPO_DIR | sign ISO KEY CERT" ;;
esac
