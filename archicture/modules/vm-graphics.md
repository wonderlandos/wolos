# VM graphics (WolOS desktop in QEMU)

Result: no full Hyprland desktop in a VM on FreeBSD 15.1. Desktop proof needs real hardware (USB boot) or iGPU passthrough (`image/vm/run-gpu.sh`).

| Option | Tried | Result | Evidence |
|---|---|---|---|
| a. drm-kmod virtio-gpu | drm-515/61/66/612/latest-kmod for 15.1 | fail | packages ship only amdgpu, dmabuf, drm, i915kms, radeonkms, ttm .ko; no virtio_gpu |
| b. other QEMU devices | std/bochs, qxl, cirrus, virtio-vga | fail | no FreeBSD DRM driver for them, so no /dev/dri; host QEMU: "'virtio-vga' is not a valid device model name" |
| c. Hyprland headless / llvmpipe | Hyprland nested in headless sway (pixman) | fail | aquamarine: "Wayland backend cannot start: Missing protocols" (no linux-dmabuf from a pixman host), then "Cannot open backend: no allocator available" (needs a GBM render node; no software allocator) |

Works without a GPU (app shots only, not the desktop):

| What | How |
|---|---|
| GTK apps | `image/vm/test/headless.sh start`, then `shot <app> <png>` (sway `WLR_BACKENDS=headless WLR_RENDERER=pixman`, grim) |
| Linux GL apps | Xwayland in that sway; Linux llvmpipe glxgears via wol-run |

## Passthrough panel (VBT)
OVMF gives the guest no IGD OpRegion and the iGPU has no ROM, so i915 logs `Failed to find VBIOS tables (VBT)` and the eDP panel stays dark.
Fix: host `sudo cat /sys/kernel/debug/dri/0000:00:02.0/i915_vbt` → guest `/boot/firmware/i915/vbt.bin` (fork patch 0002 loads it last).
