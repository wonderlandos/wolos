# VM graphics (WolOS desktop in QEMU)

Decision record: none yet.

> Historical. This result was measured on the FreeBSD base (no virtio GPU driver there). On Linux the VM tests use `virtio-vga` (`wolvm`, see [vm-e2e.md](vm-e2e.md)). The table below is the old FreeBSD record.

Result: no full compositor desktop in a VM on FreeBSD 15.1. Desktop proof needs real hardware (USB boot) or iGPU passthrough (`wolos-iso/tests/run-gpu.sh`).

| Option | Tried | Result | Evidence |
|---|---|---|---|
| a. drm-kmod virtio-gpu | drm-515/61/66/612/latest-kmod for 15.1 | fail | packages ship only amdgpu, dmabuf, drm, i915kms, radeonkms, ttm .ko; no virtio_gpu |
| b. other QEMU devices | std/bochs, qxl, cirrus, virtio-vga | fail | no FreeBSD DRM driver for them, so no /dev/dri; host QEMU: "'virtio-vga' is not a valid device model name" |
| c. Compositor headless / llvmpipe | The compositor nested in headless `sway` (pixman) | fail | aquamarine: "Wayland backend cannot start: Missing protocols" (no linux-dmabuf from a pixman host), then "Cannot open backend: no allocator available" (needs a GBM render node; no software allocator) |

Works without a GPU (app shots only, not the desktop):

| What | How |
|---|---|
| GTK apps | `wolos-iso/tests/test/headless.sh start`, then `shot <app> <png>` (sway `WLR_BACKENDS=headless WLR_RENDERER=pixman`, grim) |
| Linux GL apps | Xwayland in that sway; Linux llvmpipe glxgears via wol-run |

## Linux VMs (VirtualBox, QEMU): cursor and scanout

`hyprland.conf` runs `hyprctl keyword cursor:no_hardware_cursors 1` when `systemd-detect-virt --vm` succeeds: VM GPUs (vmwgfx, virtio) put no pointer on a cursor plane. The pointer is then composited into every frame, so direct scanout of a fullscreen game or video never engages in a VM. This is expected; scanout and `hyprctl wolstats` numbers come from real hardware only (docs/modules/performance.md).

## Passthrough panel (VBT)
OVMF gives the guest no IGD OpRegion and the iGPU has no ROM, so i915 logs `Failed to find VBIOS tables (VBT)` and the eDP panel stays dark.
Fix: host `sudo cat /sys/kernel/debug/dri/0000:00:02.0/i915_vbt` → guest `/boot/firmware/i915/vbt.bin` (fork patch 0002 loads it last).
