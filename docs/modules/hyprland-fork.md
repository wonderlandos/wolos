# Compositor fork

Base: upstream `v0.56.2`, the last release with the legacy hyprlang config the shipped confs use (0.57.0 reads Lua; Arch's package moved to it, so the fork recipe is the only source of 0.56.2; the PKGBUILD follows Arch's 0.56.2-4 packaging). Build: wolos-pkgbuilds `hyprland-wolos/` (PKGBUILD + the `NNNN-*.patch` files beside it).
Patches: `wolos-pkgbuilds/hyprland-wolos/NNNN-*.patch`, applied in order. Config: `desktop/overlay/usr/share/wolos/hypr/wolos-perf.conf` (sourced by the desktop conf).
Frontmost QoS and placement: `plugins/wol-hypr` + `crates/realtime` C ABI (`wol_realtime.h`), not a patch.

| Optimization | IaraOS source | Compositor mechanism | Status | Measured |
|---|---|---|---|---|
| GPU compositing (GLES) | `gpu.rs`, `render.rs` | `CHyprOpenGLImpl` | built in | - |
| linux-dmabuf zero copy | `render.rs` | `linux-dmabuf` protocol | built in | - |
| Damage + buffer age | `render.rs`, `cache.rs` | `CDamageRing`, `debug:damage_tracking = 2` | config | - |
| Event-driven frames | `main.rs` frame clock | `debug:vfr = true` | config | idle 0.12 ms CPU/s |
| Animation tick only while animating | `main.rs` 16 ms timer | `shouldTickForNext()` | built in | - |
| Animation tick at twice the frame rate, not 1 kHz (1x beat against vblank and juddered, audit4 S5-7) | `cache::next_frame_ms` (16 ms) | `CHyprAnimationManager::scheduleTick` | patch 0001 | at 1x: animating 67-71 -> 8-10 ms CPU/s, 906 -> 63 wakeups/s; 2x (about 120 wakeups/s) not measured |
| Idle heartbeat instead of fixed polls; pointer motion that unhides the cursor re-arms the ticker | `main.rs` 250 ms idle timer | cursor-hide ticker (`Renderer.cpp`, was 2 Hz) | patch 0002 | idle: 3 -> 1 wakeups/s |
| Cached blur of static backdrop | `blur.rs` (redo on backdrop change) | `decoration:blur:new_optimizations`, `xray` | config | - |
| Dual Kawase glass | `glass.rs`, `shaders/kawase_*` | `decoration:blur:passes = 2` | config | - |
| Close snapshot in GPU texture | `anim.rs`, `render.rs` | `makeSnapshotFB` (FBO, no readback) | built in | - |
| Settled fallback snapshots (crashed apps) | `cache.rs` 500 ms / 2 s | snapshot at unmap only | not ported | - |
| Direct scanout | `backend_drm.rs` | `render:direct_scanout = 2` | config | not measured (nested) |
| HW cursor plane | `backend_drm.rs` | `cursor:no_hardware_cursors = 2` | config | - |
| Frame ring (ms, damage px) for `wol-realtime hypr stats` | - | `render.stage` hook + hyprctl `wolstats` in `plugins/wol-hypr` | plugin | - |
| Frontmost -> QoS boost, handed to a helper thread (the compositor thread never waits on wold, audit4 S5-1) | `qos.rs` | `activeWindow` hook in `plugins/wol-hypr` | plugin | - |
| No `.conf` deprecation banner (0.56 pin keeps `.conf`) | - | `CCompositor` startup notifications | patch 0003 | - |
| Client minimise is real (CSD minimise light enabled) | - | xdg `wm_capabilities` + `set_minimized` -> `special:hidden` | patch 0004 | - |
| Locked start / restored locker never shows the desktop; crash restart relaunches the locker (ADR 0012) | - | `SSessionLock::takeover`, `start-hyprland` `$WOLOS_CRASH_LOCK_CMD` | patch 0005 | - |
| ⌘C reaches the app as Ctrl+C: `sendshortcut` finds the key in any layout (⇧⌘Z, Cyrillic), taps only the asked modifiers, restores the real ones (`WolosKeys.hpp`, gtest) | - | `DispatcherTranslator` `sendshortcut`, `Actions::pass` | patch 0006 | - |
| Content-only layer commits (menu bar clock, Dock badge) damage only the changed pixels; quiet backdrop commits keep the cached blur | `blur.rs` (redo on backdrop change) | `CLayerSurface::onCommit` | patch 0007 | not measured (needs GPU run) |
| Focusing a minimised window (Dock click, `focuswindow`, app activation) restores it to the current desktop | - | `CFocusState::fullWindowFocus`, `WolosWindow.hpp` | patch 0008 | - |
| `direct_scanout = 2` also scans out fullscreen video (content-type video) and tearing-hint games | `backend_drm.rs` | `CMonitor::isDSBlocked` | patch 0009 | not measured (needs GPU run) |
| Covered windows get frame callbacks every 33.3 ms or 2 updates once more than 10 windows are open (`misc:throttle_*`); occluders by opaque region (rounded and CSD windows count), windows allowed over a fullscreen window keep their frames | - | `sendFrameEventsToWorkspace`, `WolosPerf::allowFrameCallback`, `occluderRect`, `coveredByFullscreen` | patch 0010 | not measured |
| After a mode change, rendering waits at most 250 ms for clients to ack their new size (`misc:reconfigure_ack_timeout_ms`) | - | `renderMonitor`, `SReconfigureBarrier` | patch 0011 | not measured |
| Frame callbacks carry the output's predicted next present time, one monotonic floor across outputs; a render past 95 % of the frame is late (`misc:frame_deadline`) | - | `SPresentPredictor`, `monotonicFloor`, `CMonitorFrameScheduler::onSyncFired` | patch 0012 | not measured |
| Zoom (maximize) is a state the client sees: every toplevel with an xdg-decoration object (asked for a client- or server-side frame: Chromium, Electron, Qt, SDL) gets the sticky `maximized` dropped for one configure, so its own maximized state follows zoom; GTK (no decoration object) keeps it, since unmaximized GTK draws shadow margins and rounded corners under the bar | - | `CXDGToplevelResource` configure, xdg-decoration map (`XDGShell.cpp`) | patch 0013 | - |
| A crashed locker leaves the screen black while wol-lock brings a new one (no "lockscreen app died" page), and a new lock takes over an unconfirmed one (`misc:allow_session_lock_restore`, ADR 0012); only `/usr/bin/wol-lock` may replace a held or pending lock (any other same-user client gets `finished`, audit4 S5-15) | - | `shallConsiderLockMissing`, session-lock takeover, `WolosLock::mayRestoreLock` | patch 0014 | - |
| A dragged floating window (Super+drag, hyprbars title bar, touch) never goes under the menu bar's reserved area and keeps 40 px of its frame on screen at every edge (the frame top is clamped to the usable area), against the zone of the output under the pointer (a window can be dragged to an output above); the clamp runs after snapping, where a plugin could not (input.mouse.move fires before the drag applies the position); the hyprbars touch drag uses the same clamp | - | `CDragStateController::mouseMove`, `WolosWindow::clampDragX/Y` (pure, gtest), hyprbars `onTouchMove` | patch 0015 | - |
| Active window raised (`windowState()->raise` + `bringTargetToTop` after the focus change, no hyprctl); CSD apps tagged `wol-csd`; placement after layout, warped into place (no glide from the centre); X11 override-redirect and borderless popups are never placed, cascaded from or refitted, X11 windows that asked for a position are only clamped; a fixed axis stays, the other may be cut | - | `window.active`, `window.open`, `window.openLate` hooks; `crates/realtime/src/wm_gates.rs` through `wol_realtime.h` | plugin | - |
| Esc and the Super release reach the shell without a process: forwarded to `wol-shell.sock` (`hide transient`, `hide switcher`) only while a `wol-panel-*` layer is mapped, or a ⌘Tab is in progress; ⌘Tab's Tab is sent by the plugin too (`show switcher[-prev]`, consumed, so the conf's exec bind is only the fallback) and reaches the shell before the Super release; `send(MSG_NOSIGNAL\|MSG_DONTWAIT)`; other keys are never consumed; layers already mapped at plugin load are seen | - | `input.keyboard.key`, `layer.opened`/`layer.closed` hooks in `plugins/wol-hypr` | plugin | not measured (was one `wol` fork per Esc) |
| A floating window leaving full screen is refitted to the current usable area (a resize or Dock change while it was full screen left it outside) | `placement.rs` refit | `window.fullscreen` hook in `plugins/wol-hypr` | plugin | - |

Notes for a GPU run (audit3 R2A-15): in a VM the conf forces `cursor:no_hardware_cursors 1` (no cursor plane on vmwgfx/virtio), so the software cursor is composited over every frame and direct scanout never engages there; expected, measure scanout on real hardware only. The conf sets no `cursor:inactive_timeout`, so patch 0002's ticker is only the idle heartbeat and the pointer never auto-hides on the desktop (games hide their own). Notification banners are held by the shell while the active workspace is in full screen (`hypr::Fullscreen::holds_banners`): a Top layer mapped then would draw over the game and stop its scanout. The volume OSD still shows (1.5 s, a brief HUD). Without wol-hypr loaded the shell binds Esc and the Super release itself at start (`hypr::needs_key_fallback`).

Measurement: Linux host (CachyOS), nested Wayland backend, 1280x800, one static alacritty window,
25 s warm-up, 10 s sample of `/proc/<pid>/task/*/schedstat` and main-thread voluntary switches.
Animating = `hyprctl dispatch workspace` toggled every 500 ms.

Invariants:
- Patches stay small and apply with `git apply --check` on `v0.56.2`.
- Policy math lives in `src/helpers/WolosPerf.hpp`, `WolosLock.hpp` and `WolosWindow.hpp`, tested by `tests/helpers/`.
- Bump the base tag only together with the Arch snapshot (`pkgver` of `wolos-pkgbuilds/hyprland-wolos/PKGBUILD` follows the version the snapshot ships); rebase the patches then and check them with the series check in `SERIES.md`.
- The base stays on the 0.56 series for v0.12: from 0.57 the compositor reads its configuration as Lua, and porting the shipped confs is the feature-wave item NS-compositor-lua (`.claude/v012/plan-northstar.md`); bump the base tag only with that port.
