# 0044 - The /etc overlay across slot switches
Status: accepted (written for 0.12.1 from the FIX-slot change; proven in a VM: slot A, then B on kernel 7.3.1, then A again, and three failed starts of B boot A). Builds on 0016 (Arch base, A/B slots) and 0035 (layers on a slot switch).

## Context
`/etc` is an overlay: the lower is the running slot's own `/etc`, the upper is `/var/lib/wolos/etc/upper`, shared by both slots because `/var` is. After an update the new slot has a different lower, the old slot's `/etc` copy. The kernel's overlayfs option `index=on` records the origin of the upper (the lower it was made over) and checks it at every mount. After a switch the check fails (`ESTALE`), the mount of `/etc` fails and the computer starts in emergency mode. `metacopy=on` has a second edge: a file whose copy-up holds only metadata still reads its data from the old lower, which is another slot's file after a switch. The same slot build ran pacman and mkinitcpio on a copy that was not a mount point, and both stopped with "could not determine root mount point"; a locally built package installed with `pacman -U` was not part of the copy at all.

## Decision
- One option set, one source: `wol_core::update::ETC_FSTAB` (`crates/core/src/update.rs`) is the `/etc` line of fstab, with `index=off,metacopy=off,redirect_dir=follow,xino=off,nfs_export=off`. The installer writes it (`files/usr/lib/wolos/install-run`) and every slot build rewrites it in the copy, so a slot built by an older updater is the only one that can still carry the old line, and it still boots after a switch back.
- `files/usr/lib/wolos/etc-settle` (`wol_core::update::ETC_SETTLE`) runs before each slot build: it copies the data of every metacopy-only copy-up in the upper up through the live overlay, so the new mount, which has `metacopy=off`, loses no change to a file. It is best effort: a file it cannot copy is named on stderr and the build goes on.
- The slot copy is bind-mounted onto itself around pacman and mkinitcpio, and unmounted before the seal; a rollback unmounts first. Local `-U` packages are reflinked into the copy and removed from it before the seal.

## Consequences
- A switch in either direction, and the fall-back after three failed starts, boots with the same `/etc` the person had. Edits made in the upper survive.
- `index=off` gives up the kernel's check that the lower is the one the upper was made over; that is exactly the check a switch trips, because the lower differs per slot by design.
- The step order of a slot build changed (fstab, etc-settle, bind, pacman, mkinitcpio, unbind, seal); the tests in `crates/controller/tests/update_slot.rs`, `crates/controller/tests/pkg.rs` and `crates/core/src/update/pkg.rs` pin it.
- `etc-settle` needs `getfattr` (attr); without it nothing is copied and the build goes on, and a metacopy-only file would then show the old lower's data after a switch.
- A new overlay option needs a line in `ETC_FSTAB` and a VM switch test (A, B, A), not a change in the installer alone.
