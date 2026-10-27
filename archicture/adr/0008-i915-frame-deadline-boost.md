# 0008 - i915kms: pinned drm-kmod build
Status: accepted
Context: WolOS ships its own i915kms build so GPU tuning can change without waiting for pkg.
Decision: `kernel/drm/build.sh` builds drm-kmod at the pkg drm-612-kmod tag (dmabuf, ttm, drm, i915kms; amdgpu/radeon stay pkg, same tag so drm.ko ABI matches), plus the patches in `kernel/drm/patches/` (checked after the build by `kernel/drm/verify.sh`). Shipped as `drm-fork.tar` in the offline repo, extracted by `install-pkgs`, `pkg lock drm-612-kmod`. Runtime knobs: `archicture/modules/intel-graphics.md`.
Consequences: rebuild per drm-kmod bump (only with the pkg tag).
