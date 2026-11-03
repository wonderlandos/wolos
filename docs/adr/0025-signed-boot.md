# 0025 - Signed boot with this computer's own keys
Status: accepted

## Context
WolOS boots from A/B slots with systemd-boot (ADR 0016). Secure Boot had to be off, and the rollback of ADR 0016 did not work: `default wolos-b.conf` kept a slot that had used all its tries, so a bad update booted forever.

## Decision
- **Loader:** systemd-boot stays. It counts tries, carries SBAT, starts unified images, and needs no config of its own to be signed.
- **Images:** each slot boots one signed unified kernel image, `EFI/wolos/<slot>.efi`: kernel, initramfs, command line, one signature. Names never change, so rEFInd and GRUB can point at them.
- **Rollback:** no `default` in `loader.conf`. Entries share a sort key and sort by generation (`version <gen>-<release>`); an entry with no tries left falls behind the other. `wol update` writes through `esp-sync --new` and clears any default set from the menu.
- **Keys:** every computer makes its own at install (`boot-keys`). No image carries a key and no key is shared between computers. With a TPM2 the private keys rest encrypted to it and reach RAM only while signing.
- **Tiers:** the installer picks the simplest that applies (`wol boot detect`):

| Tier | When | Chain |
|---|---|---|
| `own-keys` | firmware in Setup Mode | this computer's keys enrolled next to Microsoft's; loader and images signed by them |
| `mok` | Secure Boot on, or off with Microsoft's third-party CA in `db` | Fedora's signed, unmodified shim starts our signed loader; one key screen at first start |
| `off` | neither, or the person chose it | starts unsigned; the app says so |
| `firmware` | Secure Boot on, nothing above fits | a firmware setting must change first; the app says which |

- **Protected words** in the signed command line on `own-keys` and `mok`: `lockdown=integrity module.sig_enforce=1`.
- **Root:** the fs-verity seal stays. Its digest moves into the signed command line (`wolos.seal=`), so the signature covers it. dm-verity does not fit a writable btrfs slot.
- **Disk key:** enrolled at install against a signed PCR 11 policy (`systemd-measure`, `ukify --phases=enter-initrd`). The first boot adds the firmware state (PCR 7, and 14 with shim) and removes the install key. An update re-signs and the disk still opens; a changed kernel, command line or initramfs does not unseal. The recovery key stays.
- **Anti-rollback:** the signed `wolos.epoch=` must not be below the floor in `/var/lib/wolos/secureboot/floor`, which moves up only when an older generation is gone.

## Consequences
- Updates sign on the computer, so its key is on it. That is the price of needing no vendor key; the TPM and the encrypted volume guard it.
- Module signatures: `db` keys reach only the kernel's `.platform` keyring, MOK keys reach `.machine`. Out-of-tree modules (NVIDIA) on `own-keys` also need the key screen once.
- `lockdown=integrity` blocks hibernation and raw MSR writes.
- The live ISO is signed with the WolOS live-image key (wolos-build docs/signing-kms.md "Secure Boot live key"): with Secure Boot on, one blue key screen once; off, nothing changes.

## Live ISO with Secure Boot on
The installed system needs no vendor key; a live ISO that boots with Secure Boot on does: one WolOS
key that signs only the live image's second stage and UKI. Shim (unchanged, Microsoft-signed) is the
first stage, so a PC with the usual keys starts it, and MokManager enrols the WolOS certificate once.

- **Layout** (`wolos-iso/release/sb-layout.sh layout`, `WOLOS_SB_LAYOUT=1 wolos-iso/build.sh`): `EFI/BOOT/BOOTX64.EFI`
  is the pinned `shimx64.efi`, `EFI/BOOT/mmx64.efi` is MokManager, systemd-boot is
  `EFI/BOOT/grubx64.efi` (the name shim starts), and the kernel, initramfs and both entries are one
  UKI with two profiles, Install WolOS and Try WolOS, `EFI/Linux/wolos.efi`. Unsigned it boots with
  Secure Boot off.
- **Signing** (`wolos-iso/release/sb-layout.sh sign ISO KEY CERT`, wolos-iso `release.yml` when `WOLOS_SB_KEY` and `WOLOS_SB_CERT`
  are set): systemd-boot and the UKI are signed, `/ENROL_THIS_KEY_FOR_WOLOS.cer` (DER; also `EFI/wolos/wolos-live-2026.cer`, with `EFI/wolos/README.txt`) is on the EFI partition
  for MokManager's "Enroll key from disk". `release.yml` then runs `wolos-iso/tests/test-secureboot.sh`: OVMF in
  secure mode with the PC keys and the certificate in MokList must boot to the live system.
- **First boot with Secure Boot on:** shim cannot verify systemd-boot yet and opens MokManager:
  Enroll key from disk, `/ENROL_THIS_KEY_FOR_WOLOS.cer` (DER; also `EFI/wolos/wolos-live-2026.cer`, with `EFI/wolos/README.txt`), confirm, reboot. Later boots go straight through.
- **The key** is a trust root: only the owner makes it, offline, and it signs nothing but live images.

Making the key (owner, once, on an offline machine with `openssl`):

```sh
umask 077; mkdir wolos-sb && cd wolos-sb
openssl req -new -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes \
  -subj "/CN=WolOS Live Image Secure Boot 2026/O=WolOS/" \
  -addext "extendedKeyUsage=codeSigning" -keyout wolos-sb.key -out wolos-sb.crt
openssl x509 -in wolos-sb.crt -noout -fingerprint -sha256
# Offline backup of the key, encrypted (keep the passphrase apart from it):
openssl pkey -in wolos-sb.key -aes256 -out wolos-sb.key.enc
```

Adding it (from a computer with `gh` signed in as a wonderlandos owner; the key leaves the offline
machine only into the secret):

```sh
gh secret set WOLOS_SB_KEY  -R wonderlandos/wolos < wolos-sb.key
gh secret set WOLOS_SB_CERT -R wonderlandos/wolos < wolos-sb.crt
gh variable set WOLOS_SB_LAYOUT -R wonderlandos/wolos --body 1
shred -u wolos-sb.key   # the plain copy; wolos-sb.key.enc stays offline
```

Box builds (AWS) then build with `WOLOS_SB_LAYOUT=1 wolos-iso/build.sh`; the tag's `release.yml` run signs the
image, boots it with Secure Boot on, and uploads the signed image with its new hash.


