# Hardware

Decision record: ADR 0016 (Arch base; firmware from Arch packages).

One catalogue (`crates/core/src/hardware/catalog.rs`) decides, per device, the Linux driver, kernel modules, Arch firmware/driver packages and modprobe options. Boot, `wol hw` and helpers read the same table; no shell scripts pick drivers.

## Detection (`wol_core::hardware::read`, Host trait, 2 s per command, never a shell)

| Source | Gives |
|---|---|
| `/sys/bus/pci/devices/*/{vendor,device,class}`, `driver` link | PCI ids, class, bound driver |
| `lspci -mm` (optional) | PCI device names |
| `/sys/bus/usb/devices/*` (`idVendor`, `idProduct`, `bDevice*`, interfaces `bInterface*` + `driver`) | USB ids, interface classes, drivers (readable without root) |
| `/sys/bus/acpi/devices/*/modalias` | ACPI `_HID`/`_CID` (I2C HID `PNP0C50`, AMD I2C) |
| `/sys/class/backlight` | backlights |
| `/proc/modules`, `/usr/lib/modules/<release>/modules.{dep,builtin}` | loaded and available modules (built-ins count as loaded) |
| `pacman -Qq` | installed packages |
| `/proc/cpuinfo`, `cpu0/cpufreq/scaling_driver` | CPU vendor and model, cpufreq driver |

Matching: bus, vendor, device range, class prefix. Narrowest matching range wins, then a named vendor over any vendor, then the longest class prefix; ties go to the earlier row. Module names compare with `-` and `_` alike.

## States (`shared::hardware`, snapshot file)

| State | When | Hint |
|---|---|---|
| ready | driver bound, firmware and packages present | - |
| needs-driver | module not loaded, or its package missing | `wol hw fix` / `wol hw fix --yes` |
| needs-firmware | firmware package missing | `wol hw fix --yes` |
| unsupported | module not in this kernel, or row marked | one-line note |

## CLI

| Command | Does |
|---|---|
| `wol hw`, `wol hw list [--json]` | every device: kind, name, driver, state (table on a tty, JSON otherwise) |
| `wol hw status [--json]` | problems only and the fix |
| `wol hw fix [--yes]` | `modprobe` via wol-admind `hw-load` (catalogue modules only); packages via `pkg-install` only with `--yes` |
| `wol hw boot [--online]` | boot service only (root); `--online` on first boot: `pacman -S --needed --noconfirm` the missing catalogue packages first |
| `wol hw report` | the models of this computer's devices for the public hardware list, JSON, no personal data ("Hardware reports" below) |
| `wol sys hw` | same devices as `wol hw list --json` (System API topic, cached 30 s) |

## Boot (`wolos-hw.service`, every boot: a disk moves between machines)

udev autoloads most drivers before; its loads count as loaded.

| Step | Action |
|---|---|
| packages | `--online` only: install what the catalogue lists as missing (NVIDIA), then re-read the hardware |
| options | `/etc/modprobe.d/wolos-hw.conf`: `options` lines (`nvidia_drm modeset=1 fbdev=1`), written before any load |
| boot list | `/etc/modules-load.d/wolos-hw.conf`: catalogue modules of the supported devices present |
| modules | `modprobe` each missing catalogue module now |
| snapshot | written for `wol hw` and Settings |

Written files start with a WolOS header. Unchanged files are not rewritten. Always exits 0. Microcode (`intel-ucode`/`amd-ucode`) loads from the initramfs.

## NVIDIA

| GPU (PCI device) | Driver | Packages |
|---|---|---|
| Turing and newer (0x1e00+) | `nvidia` open modules (`nvidia`, `nvidia_modeset`, `nvidia_drm`, `nvidia_uvm`) | `nvidia-open`, `nvidia-utils` (installed online at first boot if missing) |
| older | `nouveau` | `linux-firmware-nvidia` |

Session environment (GBM, GLX vendor, VA-API, PRIME offload): `crates/realtime/src/gpu_env.rs`.

## Hardware reports (`catalog::report`)

A report is what a public hardware matrix needs from one computer: which models it has and how WolOS drives them. `wol_core::hardware::catalog::report(&snapshot, os)` turns a snapshot into JSON (`format: 1`): per device the kind, bus, `vvvv:dddd` id (or ACPI id, CPU vendor), the catalogue's row name, the bound driver, the state and the missing catalogue packages.

| Kept | Left out (why) |
|---|---|
| kind, bus, id, catalogue name, driver, state, missing catalogue packages | `location` (bus paths tell machines apart), the bus's own description (USB strings can carry an owner's name or a serial), hints, notes, any package the catalogue does not name, any id or driver that is not a plain token |

`wol hw report` prints it (always JSON; `wol hw report > report.json` makes the file to send), with `os` from `/etc/os-release` `VERSION_ID` (`crates/controller/src/hw.rs`; test `crates/controller/tests/hw.rs` `report_is_json_for_the_hardware_list_with_the_os_version`). Test of the writer: `crates/core/src/hardware/catalog.rs` (`report_names_models_and_carries_no_personal_data`).

## Tested configurations

No real machine has a published result yet. Each release run boots the image in QEMU/KVM VMs on the forge (`wolos-iso/tests/wolvm/wolvm`); `wolos-iso/tests/metrics.sh` records them in `metrics.json` (`vms`). v0.12.0 (run `rel012`, 2026-11-03): four VMs, all 4 vCPUs, 6 GB, virtio GPU at 1280x800, OVMF, KVM:

| VM | Disk | TPM | Secure Boot | Journey | Result |
|---|---|---|---|---|---|
| live | 40G, boots the ISO | no | off | live-boot | 13/13 |
| sb | 40G, boots the ISO | no | on | live-secureboot | 36/36 |
| mok | 1G, shim test ESP | yes | on | mok-screen | 16/16 |
| inst | 40G, autoinstall | yes | off | install-auto | 3/4: installed, the harness lost the VM at `stop`; the installed disk's first start was not reached |

The site's Hardware page lists the catalogue (from `catalog.rs`) and these VMs.

Tests: `crates/core/src/hardware/tests.rs`, `crates/controller/tests/hw.rs`.
