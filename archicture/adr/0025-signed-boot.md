# ADR 0025: Signed boot with this computer's own keys

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
- The live ISO is unsigned: install with Secure Boot off or in Setup Mode.


