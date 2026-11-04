# Boot chain and install safety (ADR 0016)

How a WolOS computer is laid out on disk and starts, and what keeps an install or an update
from leaving it unable to start.

## Disk layout
GPT: partition 1 = ESP (1 GiB, FAT32, label `WOLOS`, mounted at `/efi`), partition 2 = btrfs (label `WolOS`).
Subvolumes: `@wolos-a` (installed system), `@home`, `@var`; the first update creates `@wolos-b`.
`/` is the slot the boot entry names (`rootflags=subvol=@wolos-a`); fstab mounts only `/home`, `/var`, `/efi`.
The pacman database lives in the slot (`DBPath = /usr/lib/pacman/`), so each slot knows its own packages.

## Boot entries
systemd-boot, `timeout 0` (3 with another system on the disk), no `default`: entries share a sort key and sort by generation, so the newest good slot boots (ADR 0025).
`/usr/lib/wolos/esp-sync` (pacman hook `95-wolos-esp`, `boot-setup`, updates with `--new`) builds one image per kernel in `EFI/wolos/` and writes:

| Entry | Image | Notes |
|---|---|---|
| `wolos-<slot>+3.conf` | `wolos-<slot>.efi` | three tries; `version <generation>-<release>` |
| `wolos-<slot>-<pkgbase>.conf` | `wolos-<slot>-<pkgbase>.efi` | any other kernel installed with pacman |

Words come from `/etc/kernel/cmdline`, the slot from `/etc/wolos/slot`; `wolos.epoch=`, the tier's words and `wolos.seal=` are added inside the image.
A boot is good when `wolos-boot-ok.service` sees wold (login window) up, before `boot-complete.target`; `systemd-bless-boot` then drops the counter and `wol boot bless` moves the epoch floor.
Three failed boots: the entry sorts last and the other slot boots.

## /etc across slots (ADR 0044)
`/etc` is an overlay over the running slot's own `/etc`, with one upper in `/var/lib/wolos/etc/upper` shared by both slots. Its fstab line is `wol_core::update::ETC_FSTAB` (`index=off,metacopy=off,...`): the installer writes it and every slot build rewrites it, after `files/usr/lib/wolos/etc-settle` has copied the data of metacopy-only files up, so a switch to the other slot (or the fall-back after failed boots) mounts `/etc` instead of stopping in emergency mode.

## Quiet boot
Kernel `quiet splash loglevel=3 audit=0 rd.udev.log_level=3 systemd.show_status=auto vt.global_cursor_default=0`; plymouth theme `wolos` (the mark on black) in a systemd initramfs.

## Console and VT
ADR 0012. tty1 is the only console: no getty on any VT (`NAutoVTs=0`, `ReserveVT=0`), and wol-login (a wold job, root, `CAP_SYS_TTY_CONFIG`) blanks tty1 and sets the Linux `VT_LOCKSWITCH` (`0x560B`) before every compositor, so Ctrl+Alt+Fn and `chvt` do nothing at the login window, in a session, on the lock screen and after it is unlocked.
When the login window cannot start, wol-login sets `VT_UNLOCKSWITCH` (`0x560C`) and draws a notice on tty1 after every exit: what is wrong, the exit status, when it tries again (backoff 1 s to 5 min), `journalctl -b -u wold`, and Ctrl+Alt+Delete to restart.
The journey `tests/wolvm/journeys/vt-lock.yaml` in wolos-iso proves each of these on an installed disk.

## Secure Boot
ADR 0025. `install-boot` runs `boot-setup ROOT ESP` after the seal; it prints the tier and writes plain words to `/run/wolos/boot-note` (the Install app shows that file).

| Tier | When | First stage |
|---|---|---|
| `own-keys` | Setup Mode | our loader, signed with this computer's key, enrolled next to Microsoft's |
| `mok` | third-party CA in `db` | `wolos-shim`, then our signed loader as `grubx64.efi`; one key screen, word `boot` |
| `off` | neither, or `WOLOS_SECURE=no` | unsigned |
| `firmware` | Secure Boot on, nothing above fits | a firmware setting must change |

Keys: `/var/lib/wolos/secureboot` (`boot-keys create|open|close`). `boot-setup / /efi --enroll` turns protection on later. `wol boot status` prints the JSON Settings reads. The firmware menu entry and tamper proofs are in `wolos-iso/tests/test/`.

## Other boot managers
rEFInd finds `EFI/systemd/systemd-bootx64.efi` and `EFI/wolos/*.efi` on its own. Pick the first: it keeps try counting and rollback.
GRUB of another system: copy `/usr/share/wolos/boot/41_wolos` to its `/etc/grub.d/` and run its config update.

## Install safety
| Step | Script | Guarantee |
|---|---|---|
| list | `install-disks` | never the live medium, CDs, loop, zram or read-only disks |
| check | `install-check <disk>` | read-only; `refuse not-listed/too-small/in-use`; `os Windows/Linux/macOS/FreeBSD` from GPT types and filesystems; `secureboot on` |
| write | `install-run <disk>` | all checks before `systemd-repart --empty=force`; only the picked disk and its partitions are named; sleep and lid held off |
| plan | `install-run --plan FILE` (ADR 0023) | the disk must still match the plan's fingerprint; Alongside and Manual go through `wol install layout`, which names only the partitions in the plan and reads the table back after every step |
| verify | `install-run` | entry, kernel and initramfs on the ESP and fstab before `done` |

Unattended: `wolos.autoinstall=<disk>` on the live command line (`wolos-autoinstall.service`), then power off.
Tests: `crates/controller/tests/image_install.rs` (fake tools). VM: `wolos-iso/tests/test-install.sh` (bystander disk, `BOOT PASS|FAIL`). `wolos-iso/tests/test-installer.sh`: Windows-like and Linux-like disks, real partitioning on loop devices (`layouts`) and whole installs in a VM (`run`).

## Key enrollment

`wol boot enroll` queues this computer's key with `mokutil --import` and sets the key screen to wait.
`boot-setup` runs it during install and `wolos-mok-enroll.service` repeats it at each start until the
key is enrolled. The password comes from `wol boot password`. The image carries the first stage in
`wolos-shim`: a system without it never reports `mok`.
