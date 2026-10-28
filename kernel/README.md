# kernel

**WolKernel 1.0 (7.3)**: WolKernel release 1.0 on Linux 7.3. The product version comes first, the Linux base is always shown in parentheses.
`VERSION` is the single source (`wolkernel=`, `linux=`): both PKGBUILDs read it (pkgver `1.0+7.3`, epoch 1) and `uname -v` shows it.

WolKernel (package `linux-wolos`, ADR 0016/0017) = the `wolos` branch of github.com/wonderlandos/linux (upstream stable + the WolKernel patches, see `README.wolos.md` there) at the commit in `linux-wolos/ref`, over Arch's x86_64 config.
Config: the fork's `kernel/configs/wolos.config`, then `conf/wolos.config` merged on top. `conf/wolos.config` is the working copy: changes land here first and are then copied to the fork (CI reports drift).
Extra patches from `$WOLOS_OVERLAY/kernel/linux/patches` apply only when that variable is set. Fork tags: `wolkernel-1.0-7.3`.

| path | what |
|---|---|
| `conf/wolos.config` | Kconfig fragment, one comment per group; `scripts/kconfig/merge_config.sh <arch config> conf/wolos.config && make olddefconfig` |
| `VERSION` | WolKernel and Linux base versions (bump `wolkernel=` per release, `linux=` when `ref` moves to a new line) |
| `abi-removed.txt` | userspace interfaces removed versus Arch's kernel, one per line (`wol doctor` reads it) |
| `linux-wolos-api-headers/` | UAPI headers from the same tree; replaces `linux-api-headers` |

What users notice versus Arch's `linux`:
- Anything that runs on stock Arch runs here: 32-bit apps and Steam, Wine/Proton, Flatpak/AppImage/Snap, Docker/Podman, KVM/VirtualBox, browser sandboxes, DKMS modules (NVIDIA, ZFS, v4l2loopback).
- Removed: the x32 ABI (no x32 software exists in Arch, Flatpak or container images). Datacenter-only drivers are not built (InfiniBand, Fibre Channel, storage targets, legacy RAID cards, telecom links, cluster filesystems).
- Desktop tuning: 1000 Hz full preemption, sched_ext with the WolOS scheduler (stock EEVDF if it stops), hugepages on request only, MGLRU, zram/zswap with zstd, BBR.
- Module ABI is stable within a release line (CONFIG_MODVERSIONS): vendor modules built once keep loading until the next line.
- `uname -r` ends in `-wolos`; `uname -v` reads `#1 WolKernel 1.0 (7.3) SMP PREEMPT_DYNAMIC <date>`.
- `/sys/kernel/wolos/` and `<linux/wolos.h>` exist when the extra patches are applied; nothing depends on them to boot.

Measured (7.3.0, `make olddefconfig`): Arch 6332 modules / 3247 built-in options; WolKernel 6154 / 3164 (-178 modules, -83 built-in).


## Build

| path | what |
|---|---|
| `linux-wolos/ref` | pinned fork commit (bump it to move the kernel) |
| `linux-wolos/PKGBUILD` | `makepkg -s`: `linux-wolos` + `linux-wolos-headers` (DKMS, e.g. `nvidia-open-dkms`) |
| `linux-wolos/prepare.sh` | extra patches in name order (no fuzz; exit 3 if one does not apply), fork fragment then ours, `make olddefconfig` |
| `linux-wolos/config.x86_64` | Arch `linux` config at the snapshot (`7.2.11.arch1-1`) |

Published only in `[wolos]` (`image/repo/build.sh`); the ISO never ships Arch `linux`. Other kernels installed with pacman get their own boot entry; `linux-wolos` stays the default.
