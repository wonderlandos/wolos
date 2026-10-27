# image

WolOS installer/live USB (amd64, UEFI+BIOS) built from official `FreeBSD-15.1-RELEASE-amd64-memstick.img`.

| command | where | free disk (checked) |
|---|---|---|
| `image/vm/build-in-vm.sh` | Linux host: boots FreeBSD 15.1 BASIC-CLOUDINIT VM, runs everything inside | 40 GB in `$WOLOS_VM` (default `~/WolOS-vm`) |
| `WOLOS_SRC=/usr/src image/build.sh` | FreeBSD 15.1 host, root | 20 GB in `$WOLOS_WORK` (+ kernel 12 GB) |
| `image/vm/up.sh` | Linux host: fetch+boot VM, ssh ready, `/work` (vtbd1, sparse `work.raw`) mounted, deps; idempotent | 20 GB |
| `image/vm/update-disk.sh [--dry-run]` | Linux host: updates the installed `~/WolOS-vm/test/disk.raw` in place into a new boot environment (old BE = fallback; homes, accounts, rc.conf kept). Refuses while a VM uses the disk | ~6 GB (work.raw) |
| `image/vm/run.sh` / `image/vm/ssh.sh` | boot VM (KVM, virtio, OVMF if found, VNC :0, SSH 2222) / run as root | - |

Output: `WolOS-15.1-amd64.img`. Write: `dd if=WolOS-15.1-amd64.img of=/dev/daX bs=1m`.

build.sh steps:
1. fetch memstick `.img.xz` + `CHECKSUM.SHA256-*`, verify sha256 of `.xz` and `.img`.
2. copy, grow to `WOLOS_IMG_SIZE` (12g), mdconfig, resize partition, growfs, mount.
3. stage `wolos.txz`: `pkg -r` of `packages-base.txt` + `desktop/packages.txt`; `desktop/overlay/`; `design/{icons,wallpapers,tokens}`;
   Rust workspace (`build-rust.sh`: wold wol-admind wol wol-login wol-run wol-shell wol-spotlight wol-control-center wol-lock wol-settings wol-loginwindow; wol-authcheck 4555 in libexec); compositor = pkg `hyprland` (or fork, `hyprland/`); `files/` (rc.d/wold, rc.d/wolos_firstboot);
   WolKernel + wolos_qos if `WOLOS_SRC` set. Added to `usr/freebsd-dist/MANIFEST`.
4. `/etc/installerconfig`: asks disk only, ZFS GPT BIOS+UEFI, `zroot/ROOT/default` (boot environments).
5. live root (`WOLOS_LIVE=1`): extract wolos.txz, rc.conf, rw fstab, `/firstboot`.

First boot (`wolos_firstboot`): pciconf GPU -> kld_list i915kms | amdgpu | nvidia-drm; linux_base-rl9 + linux-steam-utils; bsddialog user creation if none.
| `hyprland/build-fork.sh` | optional fork build (`WOLOS_HYPR_FORK=1`, URL/ref env, `patches/`) | 3 GB |

VM (`$WOLOS_VM`, default `~/WolOS-vm`):
- runs detached (`qemu -daemonize`, pid `vm.pid`); stays up after scripts exit. Stop: `kill $(cat ~/WolOS-vm/vm.pid)` or `image/vm/ssh.sh poweroff`.
- start again: `image/vm/up.sh` (or `run.sh`). Shell: `image/vm/ssh.sh [cmd]` (root, key `vmkey`, port 2222).
- provisioning: nuageinit runs on first boot only; `rm ~/WolOS-vm/*.qcow2` + `up.sh` reprovisions from kept `.xz`.
- root login: seed `runcmd` sets `PermitRootLogin prohibit-password` (FreeBSD default is `no`).
- no `qemu-img` on host: up.sh grows root live to `WOLOS_ROOT_SIZE` (20G) via monitor `block_resize` + growfs; src/obj on `/work`.
- GPU: no DRM device in the VM (FreeBSD drm-kmod has no virtio-gpu DRM driver), so Hyprland cannot start; use `Hyprland --verify-config`.
