# image

WolOS live + installer ISO on Arch Linux packages pinned to one Arch Linux Archive date (ADR 0016, 0017).

| Path | What |
|---|---|
| `arch/` | mkarchiso profile (releng-based): `snapshot`, `pacman.conf` (`[wolos]` first, then core/extra/multilib at the snapshot), `packages.x86_64`, systemd-boot entries (live, Install WolOS), live-only `airootfs` |
| `arch/wolos-update-slot` | wrapper over `wol update apply`: update into the other slot, boot it next |
| `repo/packages` | one line per package: name, PKGBUILD dir, local deps, watched paths; the build order |
| `repo/build.sh` | signed `[wolos]` repo in that order: `linux-wolos` (`kernel/linux-wolos`), `hyprland-wolos` (0.56.2 + `hyprland/patches`), `wolos-system`, `wolos-base`; `WOLOS_REPO_ONLY` rebuilds a subset, `WOLOS_CHROOT` uses a devtools chroot for self-contained recipes |
| `repo/changed.sh` | changed paths on stdin -> packages to rebuild (each plus its users) |
| `repo/wolos-system/` | Rust binaries (`build-rust.sh`), Hyprland plugins, `desktop/overlay`, `files/`, design |
| `repo/wolos-base/` | one versioned base: kernel, headers, uutils coreutils, sudo-rs, bsdtar, zsh, systemd; `base.manifest` |
| `files/` | systemd units + preset, sysusers, tmpfiles, sysctl.d, udev, plymouth theme, installer (`usr/lib/wolos`) |
| `build.sh` | `[wolos]` repo if missing -> staged profile -> `mkarchiso` (or `podman` `archlinux:base-devel`) -> ISO + release manifest |
| `release-manifest.sh` | `release.json`: `{version, snapshot, wolos_repo, kernel, image, sha256, size}` |
| `sign-release.sh` | `release.json.sig`: raw Ed25519 with the package key |
| `vm/` | QEMU/KVM + OVMF + virtio-gpu: `build.sh`, `run.sh` (live), `test-install.sh` (unattended), `run-installed.sh`, `run-gpu.sh` (host iGPU) |

Build (host with archiso, ~20 GB free in `~/WolOS-vm`):

```sh
WOLOS_SIGN_KEY=<gpg id> image/vm/build.sh        # or WOLOS_UNSIGNED=1 for a local test repo
WOLOS_KEEP_DISK=1 image/vm/test-install.sh       # -> ~/WolOS-vm/test/disk.raw, checks.txt
image/vm/run-installed.sh
```

Move a release to a newer snapshot: change `arch/snapshot`, the dates in both `pacman.conf` files and `repo/wolos-mirrorlist/` (file + pkgver), rebuild, run `test-install.sh`.
Boot chain and installer: `archicture/modules/boot.md`.

## CI and updates

- `.github/workflows/packages.yml` (push to `main`): rebuilds the changed packages in an archlinux container and publishes the repo as the assets of the `wolos-<snapshot>` release.
- `[wolos-cachy]` (ADR 0019): `repo/mirror-cachy.sh` freezes x86-64-v3 builds at the release (`WOLOS_CACHY_PLAN=1` reports the size without downloading); `iso.yml` refreshes it on a tag as the `wolos-cachy-<snapshot>` release.
- `.github/workflows/iso.yml` (tag `v*` or manual): `mkarchiso` in a privileged container; the tag's release, marked latest, carries the ISO, `.sha256`, `release.json` and `release.json.sig`.
- Users: `wolos-update-check.timer` fetches `releases/latest/download/release.json` (+ `.sig`); Software Update or `wolos-update-slot` runs `wol update apply` into the other slot at that snapshot, and the running slot stays the restore point.

## Userland and GNU

- Done in `wolos-base`: uutils-coreutils (as `coreutils`), sudo-rs (as `sudo`), bsdtar. The image lists no `gcc`, `texinfo`, `man-pages` or `base-devel` (tested in `image_gaming_audio.rs`); `base` does not pull them.
- Kept on purpose: glibc and gcc-libs (Steam, Proton, NVIDIA), bash (pacman hooks), GNU findutils, diffutils, tar (no uutils package at the snapshot; pacman/dkms scripts need them), `man-db` + `less` (small, used by `wol --help`).
- Not done: busybox/toybox. Arch packages depend on the GNU names, a second provider would add conflicts for no size gain.
- Recommendation: revisit when `uutils-findutils` and `uutils-diffutils` reach `extra`; add them to `wolos-base` `provides`/`conflicts` then.

## Tuning

`files/usr/lib/sysctl.d/70-wolos-performance.conf`, `systemd/*.conf.d/70-wolos-limits.conf`, `systemd/*.conf.d/70-wolos-malloc.conf` (glibc thread cache: 64 chunks per bin instead of 7), `etc/security/limits.d`, and the PipeWire/WirePlumber drop-ins under `files/usr/share` hold the game and audio defaults (values from CachyOS-Settings and Bazzite). PipeWire's minimum quantum is 64 frames so VMs keep working. No `PROTON_*` defaults are set globally: each is game-specific, so `wol games` sets them per title.
