# Performance

Measured live, release build, Linux host, nested compositor fork, 0.56.2 (patches + plugins) on a 1280x720 headless output, started the product way: `wol-session` -> `start-hyprland` -> exec-once `wol-shell`.
When and where: October 2026, recorded with release 0.11.2 (CHANGELOG), on a CachyOS development host, not on the installed WolOS image; the compositor ran nested, so no number below comes from a real DRM output. Re-measure on the image before quoting one as a WolOS figure; the table keeps these until then.
Probes: `design/tools/perf/build.sh` builds `dmg` (screencopy region hash per frame), `vk`/`vp` (virtual keyboard/pointer, `STAMP=1` prints send time). Run the scripts inside the session (`WAYLAND_DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE` set, probes on PATH).

| Metric | Target | Measured | Reproduce |
|---|---|---|---|
| Compositor idle CPU | <= 2 ms/s | 0.06 ms/s | `idle.py 60 Hyprland wol-shell` |
| Compositor idle wakeups | <= 2/s | 1.14/s | same |
| Compositor RSS | - | 73 MB | same |
| wol-shell idle CPU (own) | <= 1 ms/s | 0.25-0.28 ms/s | same |
| wol-shell helpers (children) | - | ~1.0 ms/s (Linux nmcli/wpctl, 1/min) | same, `kids` column |
| wol-shell idle wakeups | <= 2/s | 1.72-1.78/s | same |
| wol-shell RSS | - | ~110 MB | same |
| Super+Tab to first frame | < 100 ms | 31.1 median / 32.5 p95 | `latency.py` (30 runs) |
| Control Center click to frame | < 100 ms | 27.0 / 36.8 | same |
| Overview (Ctrl+Up) | < 100 ms | 31.6 / 32.3 | same |
| Volume OSD | < 100 ms | 36.2 / 37.0 | same |
| Search keystroke to frame | < 100 ms | 15.4 / 24.3 | same |
| Search keystroke compute | < 16 ms | 0.58 / 0.94 | `WOL_SHELL_TIMING=1` log |
| Control Center x200 RSS growth | < 5 MB | -0.9 MB | `leak.py` |
| Switcher x200 RSS growth | < 5 MB | +0.5 MB | same |
| Search x200 | no leftover process | 200/200 exited | same |
| wol-shell start to wallpaper+dock+menu bar mapped | - | ~237 ms | `session_start.py LOG -- wol-session` |
| Session login to full desktop | - | ~2.54 s nested (see notes) | same |

Fixes behind these numbers (before -> after):
- Status refresh: 1 s always -> 1 s only while a panel watches, else 10 s with 60 s TTLs. Helpers 14 -> 1 ms/s; wakeups 3-10 -> 1.8/s.
- Volume keys: OSD shows after an on-demand audio read (`wol shell show osd`), not from polling.
- Control Center: spawned app per open (52/109 ms, +110 MB over 200 opens) -> resident shell panel, built once, watcher stopped on hide.
- `blocking` pool threads woke at 2 Hz forever -> exit when idle (ADR 0006, `tests/idle_threads.rs`).
- `wolos-perf.conf` was installed but never sourced -> sourced from the desktop `hyprland.conf`.
- `wol shell ...` spawns from the shell write the socket directly (no CLI fork).

Notes:
- Latency = first frame whose region hash differs after the input stamp; includes compositor frame pacing (60 Hz headless).
- Session start: the nested compositor socket is up at ~0.1 s, but exec-once fires on `Event start`, emitted on the first rendered monitor frame (`Renderer.cpp`). The harness creates its headless output from exec-once, so ~2 s goes to the compositor's fallback output. On hardware a DRM monitor renders at once; real-boot number still to measure on the installed image.
- N/A on this host: hypridle (not installed), wold (needs root paths under /var/run). perf/ptrace unavailable; profiling used LD_PRELOAD timer shims and per-thread context switches.
- Helper cost (nmcli, wpctl) was measured on a CachyOS host, not on the installed image; re-measure on the image.

## Warm Search

Search (Super+Space) used to start a GTK process per press: 0.4-0.9 s to the first frame, and the letters typed in that time went to the window underneath (QA-LN-3). `wolos-search.service` (`files/usr/lib/systemd/user/`, wanted by `wolos-session.target`, restarted on failure, never waited for by the login) keeps one `wol-search --warm` with no window; a press only activates it over D-Bus (`ui::run_panel_app`, `Activation`). Without the unit a press still starts Search the old way.

Measured 2026-11-04, release build, Arch container (GTK 4.24) on a Broadway display, not on the installed image; re-measure there before quoting it as a WolOS figure (3 runs: `wol-search --warm` under `dbus-run-session`, then one `wol-search` per press, RSS read from the process status):

| Metric | Budget | Measured |
|---|---|---|
| Warm process, idle before the first press, RSS | < 40 MB | 35.9-36.5 MB (anonymous 7.6 MB, the rest is shared library pages) |
| After the first press and close, RSS | - | 48-51 MB (anonymous 9.7 MB; it does not shrink back, GTK keeps its caches) |
| Press to first map, warm | < 150 ms | 8.3-8.9 ms |
| Second launch (the activating `wol-search`) | - | 28-30 ms, exits 0 |
| Ceiling | - | `MemoryMax=96M` in the unit: a leak ends this process, not the session |

The budget binds the fresh idle process (the journey `qa100-ln-launcher-toggle` reads it right after login). After use the resident figure is mostly shared pages that `wol-shell` also maps; the anonymous part is under 10 MB.
