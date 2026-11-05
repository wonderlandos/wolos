# wol-hypr
Thin compositor plugin: `window.openLate` (WolOS placement: own size cut to the usable area, dialogs centred
on their parent, cascade per app), `window.active` (QoS boost via wold, raise on user-driven focus only),
`render.pre` (usable-area change: display resize, Dock shown/hidden -> refit open windows), `render.stage`
(frame stats), `hyprctl wolstats`. Snapping (docs/modules/windows.md, `crates/realtime/src/snap.rs`): the drag controller's `motion` and `ended` events give the edge/corner zone and the drop preview (`snap-preview` line to the shell), a rest on a window's zoom button opens the layouts chooser (`snap-layouts`), `hyprctl wolsnap left|right|up|down` and `hyprctl wolsnap zone <place>` (also `hl.plugin.wol.snap("left")`) move the focused window; `plugin:wol:edge_snap` switches the drag and chooser off.
Policy is Rust: `crates/realtime/include/wol_realtime.h` (C ABI, `librealtime.a`), including the
input gates (`wm_gates.rs`: which windows are placed, clamped or skipped; which keys go to the shell
socket, and ⌘Tab's consumed Tab), tested by `cargo test -p realtime`.
The QoS hand-off runs on a helper thread (`wol-hypr-qos`): the compositor thread never waits on wold.
Animation curves and durations are not in the plugin: the motion table is `crates/realtime/src/anim.rs` (rendered into hyprland.lua by `anim::lua()`), and C code reads a curve by name with `wol_anim_curve` / `wol_anim_curve_name`.
Build: `gmake` (needs `pkg-config hyprland`). Load: `hyprctl plugin load $PWD/wol-hypr.so`.
Not built in CI on hosts without the compositor's headers. Its member names track the pinned compositor version.

Risk: the build uses `-fno-access-control`, and snapping writes the drag controller's private grab state (`m_dragHotspot`, `m_beginDragXY`, `m_beginDragPositionXY`, `m_beginDragSizeXY` in `unsnapForDrag`) so a snapped window dragged away keeps the pointer on its bar. Re-check those members against the compositor's headers at every version bump; the plugin is built against the pinned version only.
