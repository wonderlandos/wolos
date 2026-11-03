# 0035 - Live layers on a slot switch: held, not merged, until the new slot is good
Status: proposed (SF3, 2026-11-02, after audit2 CORE-1/CORE-2). Builds on 0016 (A/B root slots).

## Context
On a sealed slot `wol pkg` (the `pacman` wrapper) installs into a live layer: a systemd-sysext
extension under `/var/lib/extensions/wolos-layer-NNNN`, merged over `/usr` at once
(`crates/core/src/update/layers.rs`). `/var` is the shared `@var` subvolume, so every slot sees
every layer. The extension is `ID=_any`: systemd-sysext merges it on whichever slot boots.

A system update folds the layers into the new slot (`wol layers fold`, `crates/controller/src/pkg.rs`)
and names them in the slot's `/etc/wolos/folded-layers`. Until audit2 the fold then deleted
`<new slot>/var/lib/extensions/<layer>`, which is the slot's empty `var` mount point, not `@var`:
the live layers were never cleared and kept merging over every later `/usr` (older package files
over newer ones, duplicate pacman database entries with `DBPath = /usr/lib/pacman`).

Since audit2 (v0.12) `wol layers clear` removes the layers the booted slot already holds, and
`/usr/lib/wolos/boot-ok` runs it once the boot is good (after `wol boot bless`). A new slot that
fails to boot falls back to the old slot with its layers intact. What is left: during the first
boot of the new slot, before boot-ok, the old layers are still merged over its newer `/usr`. If
the release moved a library a layer also carries, the old copy wins for that boot, which may be
the boot that fails.

## Options
1. **Clear after a good boot (v0.12).** Safe fallback; the first boot runs with the stale layers.
2. **Clear before the merge.** A unit `Before=systemd-sysext.service After=local-fs.target`
   runs `wol layers clear`. Clean `/usr` from the first second, but a failed new slot falls back
   to an old slot whose live packages are gone (its files and database entries were in the layers).
3. **Hold before the merge, drop after a good boot.** The same early unit moves the layers the
   booting slot holds to `/var/lib/wolos/layers-held/<slot>/` (same file system: a rename, not a
   copy) instead of deleting them. boot-ok deletes the held set of the slot that booted well. When
   a slot boots whose folded list does not name a held layer (the fallback to the old slot), the
   early unit moves that layer back into `/var/lib/extensions` before the merge.
4. **Key each layer to its slot.** Write `ID=wolos` plus a per-build `SYSEXT_LEVEL` into the
   slot's os-release and the layer's extension-release, so systemd-sysext refuses a layer on any
   other slot. No moves at boot, but os-release is a package file that `pacman -Syu` rewrites in
   the slot, and every same-release update would need a new level.

## Decision
Proposed: option 3. Option 1 stays until it lands.

- New verb `wol layers hold` (root, early unit `wolos-layers-hold.service`, `DefaultDependencies=no`,
  `After=local-fs.target`, `Before=systemd-sysext.service`): moves the layers named in the booting
  slot's folded list from `/var/lib/extensions` to `/var/lib/wolos/layers-held/<slot>/`, and moves
  back the held layers of the other slot that this slot does not name.
- `wol layers clear` (boot-ok) then deletes `/var/lib/wolos/layers-held/<slot>/` instead of the live
  directories.
- Pure decisions in `wol_core::update::layers` (which names move where, from the folded list and the
  two directory listings); the verb only runs them. Names are checked with `layers::is_layer_name`.

## Consequences
- The first boot of a new slot sees exactly the files that slot was built with.
- A fallback to the old slot gets its live packages back without a reinstall.
- One more early unit; its failure must not stop the boot (the layers then merge as in option 1).
- Tests: the move decisions for new slot, fallback and a second update before a reboot; a contract
  test that the unit orders before `systemd-sysext.service`.
