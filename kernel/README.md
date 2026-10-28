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
- Removed (`abi-removed.txt`): the x32 ABI and the vsyscall page (only pre-2011 static binaries use it; boot with `vsyscall=xonly` to bring it back), plus pre-2010 PC hardware (PCMCIA/CardBus, floppy controller, parallel port, gameport, 1990s PCI NICs, pre-ACPI MP tables), the VESA/EFI framebuffers (simpledrm covers boot), cluster sockets (RDS, TIPC, RxRPC/AFS) and raw-flash filesystems. Datacenter-only drivers are not built (InfiniBand, Fibre Channel, storage targets, legacy RAID cards, telecom links, cluster filesystems).
- Desktop tuning: 1000 Hz full preemption, sched_ext with the WolOS scheduler (stock EEVDF if it stops), hugepages on request only, MGLRU, BBR, NTSYNC, at most 512 CPUs.
- Image side: zram swap (zstd, half of RAM up to 8 GB; zswap built but off), `sysctl.d/70-wolos-performance.conf` (swappiness 100 for zram, small dirty-page steps, no NMI watchdog), `tmpfiles.d/70-wolos-performance.conf` (MGLRU min_ttl 1 s, THP defrag on request), `udev/rules.d/60-wolos-ioscheduler.rules` (none / mq-deadline / bfq).
- Module ABI is stable within a release line (CONFIG_MODVERSIONS): vendor modules built once keep loading until the next line.
- `uname -r` ends in `-wolos`; `uname -v` reads `#1 WolKernel 1.0 (7.3) SMP PREEMPT_DYNAMIC <date>`.
- `/sys/kernel/wolos/` and `<linux/wolos.h>` exist when the extra patches are applied; nothing depends on them to boot.

Measured (7.3.0, `make olddefconfig`, before the 1.0 trim): Arch 6332 modules / 3247 built-in options; WolKernel 6154 / 3164 (-178 modules, -83 built-in).
WolKernel 1.0 (7.3): not re-measured; the CI build prints both counts.


## Build

| path | what |
|---|---|
| `linux-wolos/ref` | pinned fork commit (bump it to move the kernel) |
| `linux-wolos/PKGBUILD` | `makepkg -s`: `linux-wolos` + `linux-wolos-headers` (DKMS, e.g. `nvidia-open-dkms`) |
| `linux-wolos/prepare.sh` | extra patches in name order (no fuzz; exit 3 if one does not apply), fork fragment then ours, `make olddefconfig` |
| `linux-wolos/config.x86_64` | Arch `linux` config at the snapshot (`7.2.11.arch1-1`) |
| `.github/workflows/kernel.yml` | CI: `generic` and `v3` packages in an Arch container (ccache cached); a `wolkernel-*` tag publishes a release |

Build options (environment for `makepkg`, see the PKGBUILD header):

| variable | effect |
|---|---|
| `_march=generic\|v3\|v4\|native` | CPU level; `v3`/`v4` build `linux-wolos-v3`/`-v4` (generic stays the default package for every x86-64 PC) |
| `_cc_o3=y` | `-O3` instead of `-O2` |
| `_use_llvm_lto=thin\|full` | Clang/LLVM build with ThinLTO or full LTO |
| `_localmodcfg=y` | developer build of only the modules in `~/.config/modprobed.db` (or the loaded ones) |

ccache is used when installed; `MAKEFLAGS` defaults to `-j$(nproc)`.

Published only in `[wolos]` (`image/repo/build.sh`); the ISO never ships Arch `linux`. Other kernels installed with pacman get their own boot entry; `linux-wolos` stays the default.
