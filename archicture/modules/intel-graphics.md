# Intel graphics (i915, drm-612-kmod 6.12)

Target: ADL-P GT2 Iris Xe `8086:46a8` (`include/drm/intel/i915_pciids.h:677`, `INTEL_ADLP_IDS`). Loaded by `wolos_gpu` (`i915kms`).
Source refs: `freebsd/drm-kmod` branch `6.12-lts`, under `drivers/gpu/drm/i915/`. Proof on hardware: `image/hw/verify-gpu.sh`.

## Firmware (offline, `/boot/modules/*.ko`, packages in `desktop/packages.txt`)

| Blob i915 requests (46a8) | Order | Source | Package |
|---|---|---|---|
| `i915/adlp_dmc.bin`, fallback `adlp_dmc_ver2_16.bin` | DMC | `display/intel_dmc.c:128-131` | `gpu-firmware-intel-kmod-alderlake` |
| `i915/adlp_guc_70.bin`, `_70.1.1`, `_69.0.3` | GuC | `gt/uc/intel_uc_fw.c:93-95` | alderlake |
| `i915/tgl_huc.bin`, `tgl_huc_7.9.3.bin` | HuC | `gt/uc/intel_uc_fw.c:116-117` | `gpu-firmware-intel-kmod-tigerlake` |

Also shipped: rocketlake (RKL DMC), meteorlake (MTL DMC/GuC/HuC/GSC). ADL-S/RPL use adls DMC + tgl GuC/HuC. Names confirmed by `strings i915kms.ko`.
LinuxKPI `request_firmware("i915/x.bin")` autoloads `i915_x_bin.ko`: no network, no `/lib/firmware`.

## Tunables (`image/files/boot/loader.conf.d/wolos-i915.conf`)

Every tunable below is `*_param_named_unsafe` upstream. Rule: pin driver defaults, never force on.

| Setting | Value | Source | Expected effect | Risk |
|---|---|---|---|---|
| `i915_enable_guc` | `0` | `i915_params.h:55-64` (FreeBSD: -1 -> 0, commit b2c91e6ebc), `gt/uc/intel_uc.c:45` | execlists submission, no GuC/HuC load; DMC still loads | `1`/`2`/`3` freezes reboot/poweroff on FreeBSD; HuC-only media encode (VDENC BRC) unavailable |
| `i915_enable_fbc` | `-1` | `display/intel_display_params.c:109` | per-chip: FBC on (Gen9+), less memory traffic on static screens | none (default) |
| `i915_enable_psr` | `-1` | `display/intel_display_params.c:113` | PSR1/PSR2 per VBT on eDP panel: panel self-refresh when idle | panel-specific flicker; workaround `0` |
| `i915_enable_dc` | `-1` | `display/intel_display_params.c:52` | deepest DC (DC6 on ADL-P) when DMC loaded | none (default) |
| fastboot | n/a | param removed in 6.12 (`sysctl compat.linuxkpi` lists none) | fastset always on: no modeset if EFI mode = panel mode | `kern.vt.fb.default_mode` != panel native forces one modeset |
| LinuxKPI work queues | n/a | `sysctl -d compat.linuxkpi` on 15.1: only `task_struct_reserve` (3072) | nothing to tune | - |

Not set (debug/test-only per descriptions): `mmio_debug`, `memtest`, `force_reset_modeset_test`, `load_detect_test`, `enable_hangcheck=0`, `guc_log_level`.

## User space

| Need | Package / file | Session |
|---|---|---|
| GL / GLES | `mesa-dri` `iris_dri.so` | - |
| Vulkan (DXVK, Proton) | `mesa-dri` `libvulkan_intel.so` (anv), `vulkan-loader` | - |
| VA-API | `libva`, `libva-intel-media-driver` (iHD) | `LIBVA_DRIVER_NAME=iHD` |
| Compositor GPU | `crates/realtime` `gpu env` | `AQ_DRM_DEVICES` (composite GPU first), sourced by `wol-session` |

## Driver fork (ADR 0008, `kernel/drm`)

| sysctl (runtime, 0644) | Default | Effect |
|---|---|---|
| `compat.linuxkpi.i915_wol_deadline_boost` | `1` | request still running when its fence gets the vblank deadline -> `intel_rps_boost` |
| `compat.linuxkpi.i915_wol_waitboost` | `2` | 0 never, 1 every waiter (upstream), 2 foreground pid only |
| `compat.linuxkpi.i915_wol_fg_pid` | `0` | written by wold on each frontmost switch; `<= 0` = boost all |

Verified in the build VM (no Intel GPU): builds against WOLOS 15.1, `kldload` registers the three sysctls, `fg_pid` is writable. Gain: A/B in `verify-gpu.sh` on ADL-P, not yet run.

## Compute and power limits

| Need | FreeBSD 15.1 | WolOS |
|---|---|---|
| OpenCL / Level Zero on Intel GPU | no `intel-compute-runtime`/IGC port, Mesa built without rusticl (`pkg search`, 2026-10-26) | none; GPU compute = Vulkan compute on anv |
| Compute peak + GPU memory bandwidth | `vkpeak` | shipped; `verify-gpu.sh` runs `vkpeak 0` |
| Package power limits PL1/PL2 | MSR 0x610 via `cpuctl` | `wol-realtime power show` (JSON), `power set --pl1 W --pl2 W` (root; refuses a firmware-locked MSR, keeps windows/clamp) |
| Effective limit | min(MSR 0x610, MCHBAR mirror) | MCHBAR is firmware-owned, not written |

No default change: PL1/PL2 stay firmware values until the `verify-gpu.sh` A/B (firmware PL1 vs 15 W, score + watts) is run on ADL-P.
