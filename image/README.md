# image

WolOS live + installer ISO on Arch Linux packages pinned to one Arch Linux Archive date (ADR 0016, 0017).

| Path | What |
|---|---|
| `arch/` | mkarchiso profile (releng-based): `snapshot`, `pacman.conf` (`[wolos]` first, then core/extra/multilib at the snapshot), `packages.x86_64`, systemd-boot entries (live, Install WolOS), live-only `airootfs` |
| `arch/wolos-update-slot` | wrapper over `wol update apply`: update into the other slot, boot it next |
| `repo/build.sh` | signed `[wolos]` repo: `linux-wolos` (`kernel/`), `hyprland-wolos` (0.56.2 + `hyprland/patches`), `wolos-system`, `wolos-base` |
| `repo/wolos-system/` | Rust binaries (`build-rust.sh`), Hyprland plugins, `desktop/overlay`, `files/`, design |
| `repo/wolos-base/` | one versioned base: kernel, headers, uutils coreutils, sudo-rs, bsdtar, zsh, systemd; `base.manifest` |
| `files/` | systemd units + preset, sysusers, tmpfiles, sysctl.d, udev, plymouth theme, installer (`usr/lib/wolos`) |
| `build.sh` | `[wolos]` repo if missing -> staged profile -> `mkarchiso` (or `podman` `archlinux:base-devel`) -> ISO + release manifest |
| `release-manifest.sh` | `{snapshot, repo_revision, image, sha256, size}` |
| `vm/` | QEMU/KVM + OVMF + virtio-gpu: `build.sh`, `run.sh` (live), `test-install.sh` (unattended), `run-installed.sh`, `run-gpu.sh` (host iGPU) |

Build (host with archiso, ~20 GB free in `~/WolOS-vm`):

```sh
WOLOS_SIGN_KEY=<gpg id> image/vm/build.sh        # or WOLOS_UNSIGNED=1 for a local test repo
WOLOS_KEEP_DISK=1 image/vm/test-install.sh       # -> ~/WolOS-vm/test/disk.raw, checks.txt
image/vm/run-installed.sh
```

Move a release to a newer snapshot: change `arch/snapshot` and the dates in both `pacman.conf` files, rebuild, run `test-install.sh`.
Boot chain and installer: `archicture/modules/boot.md`.
