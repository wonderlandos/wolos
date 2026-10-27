# Performance

Measured live, release build, Linux host, nested Hyprland 0.56.2 fork (patches + plugins) on a 1280x720 headless output, started the product way: `wol-session` -> `start-hyprland` -> exec-once `wol-shell`.
Probes: `design/tools/perf/build.sh` builds `dmg` (screencopy region hash per frame), `vk`/`vp` (virtual keyboard/pointer, `STAMP=1` prints send time). Run the scripts inside the session (`WAYLAND_DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE` set, probes on PATH).

| Metric | Target | Measured | Reproduce |
|---|---|---|---|
| Hyprland idle CPU | <= 2 ms/s | 0.06 ms/s | `idle.py 60 Hyprland wol-shell` |
| Hyprland idle wakeups | <= 2/s | 1.14/s | same |
| Hyprland RSS | - | 73 MB | same |
| wol-shell idle CPU (own) | <= 1 ms/s | 0.25-0.28 ms/s | same |
| wol-shell helpers (children) | - | ~1.0 ms/s (Linux nmcli/wpctl, 1/min) | same, `kids` column |
| wol-shell idle wakeups | <= 2/s | 1.72-1.78/s | same |
| wol-shell RSS | - | ~110 MB | same |
| Super+Tab to first frame | < 100 ms | 31.1 median / 32.5 p95 | `latency.py` (30 runs) |
| Control Center click to frame | < 100 ms | 27.0 / 36.8 | same |
| Mission Control (Super+Up) | < 100 ms | 31.6 / 32.3 | same |
| Volume OSD | < 100 ms | 36.2 / 37.0 | same |
| Spotlight keystroke to frame | < 100 ms | 15.4 / 24.3 | same |
| Spotlight keystroke compute | < 16 ms | 0.58 / 0.94 | `WOL_SHELL_TIMING=1` log |
| Control Center x200 RSS growth | < 5 MB | -0.9 MB | `leak.py` |
| Switcher x200 RSS growth | < 5 MB | +0.5 MB | same |
| Spotlight x200 | no leftover process | 200/200 exited | same |
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
- Session start: the nested Hyprland socket is up at ~0.1 s, but exec-once fires on `Event start`, emitted on the first rendered monitor frame (`Renderer.cpp`). The harness creates its headless output from exec-once, so ~2 s goes to Hyprland's fallback output. On hardware a DRM monitor renders at once; real-boot number still to measure on FreeBSD.
- N/A on this host: hypridle (not installed), wold (needs root paths under /var/run). perf/ptrace unavailable; profiling used LD_PRELOAD timer shims and per-thread context switches.
- Linux helper cost (nmcli, wpctl) differs from FreeBSD (ifconfig, mixer); re-measure on FreeBSD.
