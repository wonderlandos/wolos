# wol-hypr
Thin Hyprland plugin: `window.openLate` (macOS placement: own size cut to the usable area, dialogs centred
on their parent, cascade per app), `window.active` (QoS boost via wold, raise on user-driven focus only),
`render.pre` (usable-area change: display resize, Dock shown/hidden -> refit open windows), `render.stage`
(frame stats), `hyprctl wolstats`.
Policy is Rust: `crates/realtime/include/wol_realtime.h` (C ABI, `librealtime.a`).
Build: `gmake` (needs `pkg-config hyprland`). Load: `hyprctl plugin load $PWD/wol-hypr.so`.
Not built in CI on hosts without Hyprland headers. Hyprland member names track the pinned Hyprland version.
