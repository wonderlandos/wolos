# 0008 - i915kms: pinned drm-kmod build
Status: superseded by 0016 (Arch Linux base; i915 comes from the Linux kernel). Kept as the FreeBSD-era record.
Context: WolOS ships its own i915kms build so GPU tuning can change without waiting for pkg.
Decision: `kernel/drm/build.sh` builds drm-kmod at the pkg drm-612-kmod tag (dmabuf, ttm, drm, i915kms; amdgpu/radeon stay pkg, same tag so drm.ko ABI matches), plus the patches in `kernel/freebsd/drm/`. Shipped as `drm-fork.tar` in the offline repo, extracted by `install-pkgs`, `pkg lock drm-612-kmod`. Runtime knobs: `archicture/modules/intel-graphics.md`.
Consequences: rebuild per drm-kmod bump (only with the pkg tag).
