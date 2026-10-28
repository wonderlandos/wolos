# Boot chain and install safety (ADR 0016)

## Disk layout
GPT: partition 1 = ESP (1 GiB, FAT32, label `WOLOS`, mounted at `/efi`), partition 2 = btrfs (label `WolOS`).
Subvolumes: `@wolos-a` (installed system), `@home`, `@var`; the first update creates `@wolos-b`.
`/` is the slot the boot entry names (`rootflags=subvol=@wolos-a`); fstab mounts only `/home`, `/var`, `/efi`.
The pacman database lives in the slot (`DBPath = /usr/lib/pacman/`), so each slot knows its own packages.

## Boot entries
systemd-boot, `timeout 0` (hold a key for the menu), `default wolos-a.conf` (the slot booting next).
`/usr/lib/wolos/esp-sync` (pacman hook `95-wolos-esp`, installer, updates with `--new`) copies `/boot/vmlinuz-*` and initramfs images to `/efi/wolos-<slot>/` and writes:

| Entry | Kernel | Notes |
|---|---|---|
| `wolos-<slot>+3.conf` | linux-wolos | default; three tries (boot counting) |
| `wolos-<slot>-<pkgbase>.conf` | any other kernel installed with pacman | own entry, never the default |

Options come from `/etc/kernel/cmdline`; the slot from `/etc/wolos/slot`.
A boot is good when `wolos-boot-ok.service` sees wold (login window) up, before `boot-complete.target`; `systemd-bless-boot` then drops the counter.
Three failed boots: systemd-boot falls back to the other slot.

## Quiet boot
Kernel `quiet splash loglevel=3 rd.udev.log_level=3 systemd.show_status=auto vt.global_cursor_default=0`; plymouth theme `wolos` (the mark on black) in a systemd initramfs.

## Secure Boot
Not signed in this beta. `install-check` reads the `SecureBoot` EFI variable; the Install WolOS app shows `install::SECURE_BOOT_TEXT`.

## Install safety
| Step | Script | Guarantee |
|---|---|---|
| list | `install-disks` | never the live medium, CDs, loop, zram or read-only disks |
| check | `install-check <disk>` | read-only; `refuse not-listed/too-small/in-use`; `os Windows/Linux/macOS/FreeBSD` from GPT types and filesystems; `secureboot on` |
| write | `install-run <disk>` | all checks before `systemd-repart --empty=force`; only the picked disk and its partitions are named; sleep and lid held off |
| verify | `install-run` | entry, kernel and initramfs on the ESP and fstab before `done` |

Unattended: `wolos.autoinstall=<disk>` on the live command line (`wolos-autoinstall.service`), then power off.
Tests: `crates/controller/tests/image_install.rs` (fake tools). VM: `image/vm/test-install.sh` (bystander disk, `BOOT PASS|FAIL`).
