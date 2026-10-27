# Boot chain and install safety

## Quiet boot
- `boot/lua/local.lua`: no key held while the machine starts -> `autoboot_delay=-1` (no menu, no countdown).
  Any key held (Space, Esc) -> the menu stays up without a countdown: boot environments, single user, `set wolos.install=1`.
  `wolos_boot_menu="YES"` in loader.conf always shows it.
- `loader.conf.wolos`: `boot_mute`, `module_verbose=0`, WolOS splash. Live and installed: `ttyv0` getty off, `rc.local` (bsdinstall) only with `wolos.install=1`.
- The installer-only image (`WOLOS_LIVE=0`) keeps the bsdinstall menu: it has no desktop.

## Firmware and other boot managers
`libexec/wolos/install-esp <disk>` runs after bsdinstall's `bootconfig`, on the picked disk's own (new) ESP:

| What | Where |
|---|---|
| loader | `\EFI\WolOS\loader.efi` |
| rEFInd icon | `\EFI\WolOS\loader.png`, `\EFI\WolOS\os_wolos.png` (from `design/brand/os_wolos.png`) |
| fallback | `\EFI\BOOT\BOOTX64.EFI`, only if missing or already a FreeBSD/WolOS loader |
| NVRAM | entry `WolOS` first in `BootOrder`; other entries keep their order and are never deleted |

Entries pointing at this ESP (bsdinstall's `\efi\freebsd`) are replaced: the partition is new, so they are ours.
Legacy BIOS: `gptzfsboot` in `freebsd-boot`, unchanged.

GRUB cannot read the ZFS root reliably. Chainload the loader from the ESP (`/etc/grub.d/40_custom`, then `update-grub`):

```
menuentry "WolOS" {
    insmod part_gpt
    insmod fat
    search --no-floppy --set=root --file /EFI/WolOS/loader.efi
    chainloader /EFI/WolOS/loader.efi
}
```

Without that snippet (no os-prober support), the firmware boot menu still lists `WolOS`.

## Secure Boot
`loader.efi` is not signed by a key firmware trusts by default: with Secure Boot on, firmware refuses to start WolOS.
`install-check` reads the `SecureBoot` EFI variable; the Install WolOS app shows `install::SECURE_BOOT_TEXT` (turn it off in firmware settings).
No signed boot path in this beta.

## Install safety
| Step | Script | Guarantee |
|---|---|---|
| list | `install-disks` | never cd/md, never the boot medium (memstick, ISO, Ventoy) |
| check | `install-check <disk>` | read-only; `refuse not-listed/too-small/in-use`; `os Windows/Linux/macOS/FreeBSD` from GPT/MBR types and labels for the warning; `secureboot on` |
| write | `installerconfig` | `ZFSBOOT_DISKS` = the picked disk only; all checks run before `gpart destroy` |
| verify | `install-verify <disk>` | read-only pool import from the picked disk's ZFS partition, `bootfs`, health, `freebsd-boot`, ESP loaders; exported before "done" |

Tests: `crates/controller/tests/image_boot_safety.rs` (fake tools; no tool call names another disk).
VM: `image/vm/test-install.sh` attaches a bystander disk and prints `BOOT PASS|FAIL` lines (hash, firmware entry, ESP, no menu).
