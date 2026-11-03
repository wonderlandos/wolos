# wol-hypr
Thin compositor plugin: `window.openLate` (macOS placement: own size cut to the usable area, dialogs centred
on their parent, cascade per app), `window.active` (QoS boost via wold, raise on user-driven focus only),
`render.pre` (usable-area change: display resize, Dock shown/hidden -> refit open windows), `render.stage`
(frame stats), `hyprctl wolstats`.
Policy is Rust: `crates/realtime/include/wol_realtime.h` (C ABI, `librealtime.a`), including the
input gates (`wm_gates.rs`: which windows are placed, clamped or skipped; which keys go to the shell
socket, and ⌘Tab's consumed Tab), tested by `cargo test -p realtime`.
The QoS hand-off runs on a helper thread (`wol-hypr-qos`): the compositor thread never waits on wold.
Build: `gmake` (needs `pkg-config hyprland`). Load: `hyprctl plugin load $PWD/wol-hypr.so`.
Not built in CI on hosts without the compositor's headers. Its member names track the pinned compositor version.
