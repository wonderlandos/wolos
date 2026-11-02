# Hyprland fork

Base: upstream `v0.56.2` (FreeBSD port `x11-wm/hyprland` 0.56.2). Build: wolos-pkgbuilds `hyprland-wolos/` (PKGBUILD + the `NNNN-*.patch` files beside it).
Patches: `wolos-pkgbuilds/hyprland-wolos/NNNN-*.patch`, applied in order. Config: `desktop/overlay/usr/local/share/wolos/hypr/wolos-perf.conf` (sourced by the desktop conf).
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
| Content-only layer commits (menu bar clock, Dock badge) damage only the changed pixels; quiet backdrop commits keep the cached blur | `blur.rs` (redo on backdrop change) | `CLayerSurface::onCommit` | patch 0007 | not measured (needs GPU run) |
| Focusing a minimised window (Dock click, `focuswindow`, app activation) restores it to the current desktop | - | `CFocusState::fullWindowFocus`, `WolosWindow.hpp` | patch 0008 | - |
| `direct_scanout = 2` also scans out fullscreen video (content-type video) and tearing-hint games | `backend_drm.rs` | `CMonitor::isDSBlocked` | patch 0009 | not measured (needs GPU run) |
| Covered windows get frame callbacks every 33.3 ms or 2 updates once more than 10 windows are open (`misc:throttle_*`) | - | `sendFrameEventsToWorkspace`, `WolosPerf::allowFrameCallback` | patch 0010 | not measured |
| After a mode change, rendering waits at most 250 ms for clients to ack their new size (`misc:reconfigure_ack_timeout_ms`) | - | `renderMonitor`, `SReconfigureBarrier` | patch 0011 | not measured |
| Frame callbacks carry the output's predicted next present time; a render past 95 % of the frame is late (`misc:frame_deadline`) | - | `SPresentPredictor`, `CMonitorFrameScheduler::onSyncFired` | patch 0012 | not measured |
| Zoom (maximize) is a state the client sees: every toplevel with an xdg-decoration object (asked for a client- or server-side frame: Chromium, Electron, Qt, SDL) gets the sticky `maximized` dropped for one configure, so its own maximized state follows zoom; GTK (no decoration object) keeps it, since unmaximized GTK draws shadow margins and rounded corners under the bar | - | `CXDGToplevelResource` configure, xdg-decoration map (`XDGShell.cpp`) | patch 0013 | - |
| A crashed locker leaves the screen black while wol-lock brings a new one (no "lockscreen app died" page), and a new lock takes over an unconfirmed one (`misc:allow_session_lock_restore`, ADR 0012) | - | `shallConsiderLockMissing`, session-lock takeover | patch 0014 | - |
| A dragged floating window (Super+drag, hyprbars title bar, touch) never goes under the menu bar's reserved area and keeps 40 px of its frame on screen at every edge (macOS clamps the frame top to the usable area); the clamp runs after snapping, where a plugin could not (input.mouse.move fires before the drag applies the position) | - | `CDragStateController::mouseMove`, `WolosWindow::clampDragX/Y` (pure, gtest) | patch 0015 | - |
| Active window raised; CSD apps tagged `wol-csd`; placement after layout | - | `window.active`, `window.open`, `window.openLate` hooks | plugin | - |
| Esc and the Super release reach the shell without a process: forwarded to `wol-shell.sock` (`hide transient`, `hide switcher`) only while a `wol-panel-*` layer is mapped, or a ⌘Tab is in progress; keys are never consumed | - | `input.keyboard.key`, `layer.opened`/`layer.closed` hooks in `plugins/wol-hypr` | plugin | not measured (was one `wol` fork per Esc) |
| A floating window leaving full screen is refitted to the current usable area (a resize or Dock change while it was full screen left it outside) | `placement.rs` refit | `window.fullscreen` hook in `plugins/wol-hypr` | plugin | - |

Notes for a GPU run (audit3 R2A-15): in a VM the conf forces `cursor:no_hardware_cursors 1` (no cursor plane on vmwgfx/virtio), so the software cursor is composited over every frame and direct scanout never engages there; expected, measure scanout on real hardware only. The conf sets no `cursor:inactive_timeout`, so patch 0002's ticker is only the idle heartbeat and the pointer never auto-hides on the desktop (games hide their own). Notification banners are held by the shell while the active workspace is in full screen (`hypr::Fullscreen::holds_banners`): a Top layer mapped then would draw over the game and stop its scanout. The volume OSD still shows (1.5 s, like the macOS HUD). Without wol-hypr loaded the shell binds Esc and the Super release itself at start (`hypr::needs_key_fallback`).

Measurement: Linux host (CachyOS), nested Wayland backend, 1280x800, one static alacritty window,
25 s warm-up, 10 s sample of `/proc/<pid>/task/*/schedstat` and main-thread voluntary switches.
Animating = `hyprctl dispatch workspace` toggled every 500 ms.

Invariants:
- Patches stay small and apply with `git apply --check` on `v0.56.2`.
- Policy math lives in `src/helpers/WolosPerf.hpp`, `WolosLock.hpp` and `WolosWindow.hpp`, tested by `tests/helpers/`.
- Bump the base tag only together with the FreeBSD port; rebase patches then.
