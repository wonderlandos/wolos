# Hyprland fork

Base: upstream `v0.56.2` (FreeBSD port `x11-wm/hyprland` 0.56.2). Build: `image/hyprland/build-fork.sh`.
Patches: `image/hyprland/patches/NNNN-*.patch`, applied in order. Config: `desktop/overlay/usr/local/share/wolos/hypr/wolos-perf.conf` (sourced by the desktop conf).
Frontmost QoS and placement: `plugins/wol-hypr` + `crates/realtime` C ABI (`wol_realtime.h`), not a patch.

| Optimization | IaraOS source | Hyprland mechanism | Status | Measured |
|---|---|---|---|---|
| GPU compositing (GLES) | `gpu.rs`, `render.rs` | `CHyprOpenGLImpl` | built in | - |
| linux-dmabuf zero copy | `render.rs` | `linux-dmabuf` protocol | built in | - |
| Damage + buffer age | `render.rs`, `cache.rs` | `CDamageRing`, `debug:damage_tracking = 2` | config | - |
| Event-driven frames | `main.rs` frame clock | `debug:vfr = true` | config | idle 0.12 ms CPU/s |
| Animation tick only while animating | `main.rs` 16 ms timer | `shouldTickForNext()` | built in | - |
| Animation tick at frame rate, not 1 kHz | `cache::next_frame_ms` (16 ms) | `CHyprAnimationManager::scheduleTick` | patch 0001 | animating: 67-71 -> 8-10 ms CPU/s, 906 -> 63 wakeups/s |
| Idle heartbeat instead of fixed polls | `main.rs` 250 ms idle timer | cursor-hide ticker (`Renderer.cpp`, was 2 Hz) | patch 0002 | idle: 3 -> 1 wakeups/s |
| Cached blur of static backdrop | `blur.rs` (redo on backdrop change) | `decoration:blur:new_optimizations`, `xray` | config | - |
| Dual Kawase glass | `glass.rs`, `shaders/kawase_*` | `decoration:blur:passes = 2` | config | - |
| Close snapshot in GPU texture | `anim.rs`, `render.rs` | `makeSnapshotFB` (FBO, no readback) | built in | - |
| Settled fallback snapshots (crashed apps) | `cache.rs` 500 ms / 2 s | snapshot at unmap only | not ported | - |
| Direct scanout | `backend_drm.rs` | `render:direct_scanout = 2` | config | not measured (nested) |
| HW cursor plane | `backend_drm.rs` | `cursor:no_hardware_cursors = 2` | config | - |
| Frame ring (ms, damage px) for `wol-realtime hypr stats` | - | `render.stage` hook + hyprctl `wolstats` in `plugins/wol-hypr` | plugin | - |
| Frontmost -> QoS boost | `qos.rs` | `activeWindow` hook in `plugins/wol-hypr` | plugin | - |
| No `.conf` deprecation banner (0.56 pin keeps `.conf`) | - | `CCompositor` startup notifications | patch 0003 | - |
| Client minimise is real (CSD minimise light enabled) | - | xdg `wm_capabilities` + `set_minimized` -> `special:hidden` | patch 0004 | - |
| Locked start / restored locker never shows the desktop; crash restart relaunches the locker (ADR 0012) | - | `SSessionLock::takeover`, `start-hyprland` `$WOLOS_CRASH_LOCK_CMD` | patch 0005 | - |
| ⌘C reaches the app as Ctrl+C: `sendshortcut` finds the key in any layout (⇧⌘Z, Cyrillic), taps only the asked modifiers, restores the real ones (`WolosKeys.hpp`, gtest) | - | `DispatcherTranslator` `sendshortcut`, `Actions::pass` | patch 0006 | - |
| Active window raised; CSD apps tagged `wol-csd`; placement after layout | - | `window.active`, `window.open`, `window.openLate` hooks | plugin | - |

Measurement: Linux host, nested Wayland backend, 1280x800, one static alacritty window,
25 s warm-up, 10 s sample of `/proc/<pid>/task/*/schedstat` and main-thread voluntary switches.
Animating = `hyprctl dispatch workspace` toggled every 500 ms.

Invariants:
- Patches stay small and apply with `git apply --check` on `v0.56.2`.
- Policy math lives in `src/helpers/WolosPerf.hpp` and `src/helpers/WolosLock.hpp`, tested by `tests/helpers/`.
- Bump the base tag only together with the FreeBSD port; rebase patches then.
