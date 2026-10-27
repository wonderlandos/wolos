# wol-hypr
Thin Hyprland plugin: hooks `openWindow` (macOS placement), `closeWindow`, `activeWindow` (QoS boost via wold).
Policy is Rust: `crates/realtime/include/wol_realtime.h` (C ABI, `librealtime.a`).
Build: `gmake` (needs `pkg-config hyprland`). Load: `hyprctl plugin load $PWD/wol-hypr.so`.
Not built in CI on hosts without Hyprland headers. Hyprland member names track the pinned Hyprland version.
