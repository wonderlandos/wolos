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
- The app in front goes first. wold writes its pid to `/sys/kernel/wolos/foreground_pid` on every switch; the kernel then gives it a CPU floor (`foreground_uclamp_min`, 512), exact timers (`foreground_timerslack_ns`, 0: Wine/Proton frame limiters wake on time), the i915 waitboost and the top `scx_wolos` slot. Background threads coalesce wakeups (`background_timerslack_ns`, 50 ms).
- Removed (`abi-removed.txt`): the x32 ABI, legacy PTYs and the vsyscall page (boot with `vsyscall=xonly` for pre-2011 static binaries), plus pre-2010 PC hardware (PCMCIA/CardBus, floppy, parallel port, gameport, 1990s PCI NICs, pre-ACPI MP tables), the VESA/EFI framebuffers (simpledrm covers boot), cluster sockets (RDS, TIPC, RxRPC/AFS) and raw-flash filesystems. Datacenter-only drivers are not built.
- Module ABI is stable within a release line (CONFIG_MODVERSIONS): vendor modules built once keep loading until the next line.
- `uname -r` ends in `-wolos`; `uname -v` reads `#1 WolKernel 1.0 Linux 7.3 SMP PREEMPT_DYNAMIC <date>`.

What is in 1.0 (each has an off switch; table with sources in `README.wolos.md` on the fork):

| area | change |
|---|---|
| scheduler | `scx_wolos` (sched_ext): per-CPU queues, stealing within the shared cache first, QoS slices (1/2/4 ms), frontmost boost from `foreground_pid`; BORE and an interactivity-tuned EEVDF when it is not loaded |
| frontmost app | CPU floor and exact timers for the app in front, timer coalescing behind (WolOS code) |
| locks | futex spins briefly before sleeping (`futex.spin_ns`); `selftests/wolos/futex_pingpong` measures it |
| games | NTSYNC and futex_waitv built in; split-lock mitigation off; i915 frame-deadline boost |
| memory | zram first (lz4/zstd, writeback, `page-cluster` 0), MGLRU working-set protection, no watermark boosting or proactive compaction, THP on request |
| disk | ADIOS I/O scheduler by default; none on NVMe, BFQ on spinning disks (udev) |
| network | BBRv3 with fq by default, MPTCP and WireGuard built in |
| graphics | AMD color management (HDR) properties; parallel CPU bring-up at boot |
| build | `-O3`, x86-64-v3/v4 flavours, Clang ThinLTO, zstd modules, ccache (options below) |

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
