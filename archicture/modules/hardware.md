# Hardware

One catalogue (`crates/core/src/hardware/catalog.rs`) decides, per device, the driver, firmware, packages and loader/rc lines. Boot, `wol hw` and helpers read the same table; no shell scripts pick drivers.

## Detection (`wol_core::hardware::read`, Host trait, 2 s per command, never a shell)

| Source | Gives |
|---|---|
| `pciconf -lv` | PCI vendor/device/class, attached driver |
| `devinfo -v` | ACPI ids (I2C HID `PNP0C50`, AMD I2C), parent/child units |
| `usbconfig dump_device_desc dump_curr_config_desc show_ifdrv` | USB ids, interface classes, drivers (root only; others reuse the boot snapshot) |
| `kldstat -v` | loaded modules, including in-kernel ones (`pci/re`) |
| `pkg query %n`, `/boot/{kernel,modules,firmware}` | installed packages, modules and firmware on disk |
| `hw.model`, `dev.cpu.0.freq` | CPU, cpufreq present |

Matching: bus, vendor, device range, class prefix. Narrowest matching range wins, then a named vendor over any vendor, then the longest class prefix; ties go to the earlier row.

## States (`shared::hardware`, `/var/run/wolos-hw.json`)

| State | When | Hint |
|---|---|---|
| ready | driver attached, firmware and packages present | - |
| needs-driver | module not loaded, or its package missing | `wol hw fix` / `wol hw fix --yes` |
| needs-firmware | firmware package missing | `wol hw fix --yes` |
| unsupported | no FreeBSD 15.1 driver (module not on disk, row marked) | one-line note |

## CLI

| Command | Does |
|---|---|
| `wol hw`, `wol hw list [--json]` | every device: kind, name, driver, state (table on a tty, JSON otherwise) |
| `wol hw status [--json]` | problems only and the fix |
| `wol hw fix [--yes]` | `kldload -n` via wol-admind `hw-load` (catalogue modules only); packages via `pkg-install` only with `--yes` |
| `wol hw boot [--online]` | rc.d only (root); `--online` from wolos_firstboot: `pkg install` the missing catalogue packages first |
| `wol sys hw` | same devices as `wol hw list --json` (System API topic, cached 30 s) |

## Boot (`rc.d/wolos_hw`, REQUIRE FILESYSTEMS wolos_live, BEFORE netif wolos_wifi wolos_gpu)

devmatch runs earlier (before FILESYSTEMS); its loads count as loaded. `wol hw boot`, every boot (a disk moves between machines):

| Step | Action |
|---|---|
| packages | `--online` only: `pkg install -y` what the catalogue lists as missing (NVIDIA), then re-read the hardware |
| kenv | dotted loader tunables of devices whose modules load now (`hw.nvidiadrm.modeset=1`): a module reads them only at load |
| modules | `kldload -n` each catalogue module for the devices present (GPU KMS, Wi-Fi, coretemp/amdtemp, cuse, iichid...) |
| loader | `/boot/loader.conf.d/wolos-hw.conf`: early microcode, `if_re` from realtek-re-kmod, nvidia-drm modeset; next boot |
| rc | `/etc/rc.conf.d/powerd`: `powerd_enable` YES only with a software cpufreq driver (not HWP, not a VM) |
| snapshot | `/var/run/wolos-hw.json`, read by `wol hw` for non-root USB data |

Written files start with a WolOS header; a file without it is the user's and is never touched. Unchanged files are not rewritten. Always exits 0.

Other defaults: `rc.conf.d/webcamd` (devd starts webcamd on attach, `/dev/video*` for group video), `loader.conf.d/wolos-desktop.conf` (`cuse`), `sysctl.conf.local` (preempt_thresh 224, usermount, no kbd debug keys), `dhclient-exit-hooks` (lease bound: ntpd refreshes the leap file and re-resolves its pool).

## Packages (offline, `desktop/packages.txt`)

| Package | Size | For |
|---|---|---|
| `gpu-firmware-kmod` | ~150 MB | all Intel/AMD/Radeon GPUs; drm-kmod depends on it already |
| `cpu-microcode-intel`, `-amd` | 31.5 MB, 0.6 MB | early microcode |
| `iwmbt-firmware`, `rtlbt-firmware`, `bluez-firmware` | 14.7, 1.7, 0.1 MB | Bluetooth |
| `realtek-re-kmod` | 1.4 MB | RTL8125/8126 (base re(4) lacks them) |
| `webcamd`, `pipewire-spa-oss` | 5.6, 1.9 MB | UVC cameras, PipeWire on sound(4) |

Not shipped: NVIDIA (`nvidia-drm-kmod` + `nvidia-kmod` 201 MB + `nvidia-driver` 318 MB, ~520 MB; detected, installed online), `rtw8814a`; no firmware for Wi-Fi cards without a 15.1 driver (wifi.md).

## NVIDIA (catalogue rows, first boot online)

| GPU (PCI device) | Package | State |
|---|---|---|
| Turing and newer (0x1e00+) | `nvidia-drm-kmod` (595) | needs-driver until installed, then ready |
| Maxwell/Pascal/Volta (0x1340-0x1dff) | `nvidia-drm-kmod-580` (last branch for them) | same |
| Kepler and older (< 0x1340) | - | unsupported: legacy 470 has no modesetting (Wayland) |

Modules `nvidia-modeset` + `nvidia-drm` with `hw.nvidiadrm.modeset=1` (kenv now, loader.conf next boot). wolos_firstboot runs `wol hw boot --online`, so the first greeter already runs on the driver. Session environment (GBM, GLX vendor, VA-API, PRIME offload): `crates/realtime/src/gpu_env.rs`.

## FreeBSD 15.1 gaps (checked on the test VM, 2026-10-27)

`if_mt7921` is built by WolKernel (802.11a/b/g); ath10k/ath11k/ath12k, MT7925 and Broadcom Wi-Fi have no working driver and report unsupported with the reason (wifi.md). No iwm firmware modules. Intel LNL/BMG (xe), Intel SOF audio, AMD ACP audio: unsupported.

Tests: `crates/core/src/hardware/tests.rs`, `crates/controller/tests/{hw,image_hw}.rs`.
