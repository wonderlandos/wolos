# 0017 - One base system: WolKernel and userland released together
Status: accepted. Builds on 0016.
Context: on BSD the kernel and base userland come from one tree with one version, so headers, tools and kernel always agree. A plain Arch system assembles them from unrelated packages.
Decision:
- `wolos-base` = WolKernel + its UAPI headers (`linux-wolos-api-headers` replaces `linux-api-headers`) + the base userland, built from this repo at one version and published together in `[wolos]`. `wolos-base` version = WolOS release; `uname -v` and `/etc/os-release` agree.
- Base userland prefers non-GNU, memory-safe tools: uutils-coreutils/findutils/diffutils, sudo-rs, bsdtar (libarchive), zsh, systemd-timesyncd. Kept GNU where the product needs it: glibc (Steam, Proton, NVIDIA), gcc runtime libs, bash only as `/usr/bin/sh` fallback for Arch scripts (pacman hooks, PKGBUILD).
- `wol` tools (`wol sys`, `wol hw`, `wol update`, `wol security`) are the base admin interface, documented as man pages next to the base, like BSD's `freebsd-update`/`bsdinstall`.
- Kernel removals (ADR 0016 WolKernel) land in the same base release as the matching header and userland changes; a base release that boots but fails `wol doctor` is not published.
- Stays compatible with the common Linux ecosystem: any binary, Flatpak, AppImage, container or DKMS module that runs on stock Arch runs on WolOS. Removals cover only interfaces nothing uses today; if unsure, keep it.
- pacman cannot replace base packages from Arch: `[wolos]` provides/conflicts them, and the release snapshot is tested as one unit.
Consequences: `wolos-base` is one meta-package with a manifest of every base file; `wol base verify` checks it (like `freebsd-update IDS`). Arch packages outside the base follow the pinned snapshot.
