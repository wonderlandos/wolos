# Web-app baseline (Memory Saver foundation)

Concept: what the web-based apps WolOS ships or offers cost today, measured the same way as a native app, with the Memory Saver on and off. Why: every later package that changes the web runtime or the apps' JavaScript is gated on these numbers, and the lead's goal ("web apps feel native") needs a distance to close. How: `tools/bench/webapps/` (`sh run.sh VM`) in an installed VM; the raw result is `docs/bench/webapps-baseline.json`, the per-process split is `docs/bench/webapps-roles.json`.

## Identity of this baseline

| | |
|---|---|
| Run | 2026-11-03 16:10 to 18:40 UTC, one pass, 11 rows x saver on and off, every row `ok` |
| Image | `wolos-2026.11.03-x86_64.iso`, sha256 `fcd5a474d8344d8c4c9c46567f4a2427b05554edd3c57d1bbb939b05b6a42bba`, installed with `wolos.autoinstall`; the OS reports `WolOS 0.12.0` (`wolos-system 0.12.0-1`) |
| VM | wolvm `ms` on the forge: 4 vCPU (host CPU Intel Xeon Platinum 8375C), 8 GiB (7927 MiB seen), KVM, `virtio-vga` (no GPU: every frame number is software rendering), UEFI, TPM, 1280x800 at 74.994 Hz, kernel `7.3.0-5-wolos` (`linux-wolos 1:1.0+7.3-5`), `hyprland-wolos 0.56.2-5`, `mesa 1:26.2.6-1` |
| Memory setup | zram 3963.5 MiB, algorithm zstd, THP `madvise` |
| Settings | 3 warm starts, 120 s idle in front, 120 s idle in the background, 60 s back after the interaction, 20 s prime run first (`settings` in the JSON) |
| Runner | `tools/bench/webapps/webapps.py` at this commit. The baseline ran before `roles()` was added; the roles file comes from a second, shorter run of the added runner (`--modes off --warm 1 --idle 30`) |

Apps as installed: `helium-browser 0.20.2.1-2` and `chromium 156.0.8078.79-1` (engine `chromium`), `code 1.138.0-1` on `electron42 42.11.18-1` (engine `electron`), `signal-desktop 8.33.0-1` (its own bundled `electron`), `steam 1.0.0.87-3` (client interface in `cef`), `firefox 159.0-1` and `thunderbird 158.0.1-1` (engine `gecko`), `gnome-text-editor 51.0-1` (the GTK 4 comparison), the Wolkit `todos` example (release build), and the Rust browser engine's own shell 0.7.0 (`servo`, official build; a candidate, not shipped).

## Inventory

| Row | What it is | Runtime | Launched by | Where WolOS has it |
|---|---|---|---|---|
| `helium` | the default browser | `chromium` browser | desktop entry, `helium` | base image |
| `webapp` | a site as its own window (`wol webapp add`, `crates/core/src/webapps.rs`) | `chromium` with `--app=URL`, own or shared profile | the generated `wolos-webapp-<id>.desktop` | base image (launcher) |
| `steam` | the game client; its whole interface is a page | `cef` (embedded browser) | `wol games steam` | Store `steam`, gaming module |
| `code` | the editor | `electron`, engine from the shared `electron42` package | `code` | dev module, Store `vscode` |
| `signal` | messenger | `electron`, bundled | `signal-desktop` | Store `signal` |
| `chromium` | second browser | `chromium` | `chromium` | Store `chromium` |
| `firefox` | browser; its interface is also drawn by its engine | `gecko` | `firefox` | Store `firefox` |
| `thunderbird` | mail; interface drawn by `gecko` | `gecko` | `thunderbird` | Store `thunderbird` |
| `text-editor`, `wolkit-todos` | comparison: a GTK 4 app, a Wolkit app | none | direct | base image, example |
| `servo` | candidate runtime | `servo` | direct | not shipped |

`tools/bench/webapps/apps.json` is the machine-readable list (argv, window class, packages, origin). Its `not_web` section names the Store apps that are not web-based, with the reason. `crates/controller/tests/bench_webapps.rs` fails when a Store app, a module or the web-app launcher is neither a row nor in `not_web`, so a new app is measured.

## What the Memory Saver does today

Correction to the task's premise: **the saver patches no JavaScript.** Nothing under `crates/core/src/memsaver/`, `crates/shared/src/memsaver.rs`, `crates/api/src/memsaver/` or `crates/api/src/qos.rs` reads, rewrites or injects into an app's scripts (ADR 0026 says the same). It is a cgroup policy: `wold` moves each windowed app's process tree into its own scope, and 30 s after the app stops being in use it gives the memory back once, to zram (`memory.reclaim` plus `process_madvise(MADV_PAGEOUT)`); focus cancels it and pages return by faults. Never touched: the compositor, the shell, audio, the frontmost app, apps playing sound or video, games, other users' processes, exclusions. It is generic: it acts on the GTK and Wolkit rows exactly as on the web rows. JavaScript patches (telemetry, polling, jank) are therefore new work, not an extension of something that exists.

## Baseline numbers

Memory in MiB, times in ms, saver **off** (the unperturbed numbers); the next section compares saver on and off. One pass; see "How far to trust it".

| App | Engine | Procs | RSS launch | PSS launch | PSS 2 min front | PSS 2 min back |
|---|---|---|---|---|---|---|
| `helium` | chromium | 10 | 1120 | 438 | 404 | 404 |
| `webapp` | chromium | 10 | 1083 | 420 | 397 | 397 |
| `steam` | cef | 15 | 1428 | 964 | 904 | 906 |
| `code` | electron | 12 | 1821 | 1118 | 806 | 802 |
| `signal` | electron | 7 | 989 | 590 | 486 | 489 |
| `chromium` | chromium | 10 | 1023 | 471 | 433 | 432 |
| `firefox` | gecko | 14 | 1153 | 538 | 505 | 505 |
| `thunderbird` | gecko | 7 | 719 | 460 | 424 | 425 |
| `text-editor` | GTK 4 | 1 | 172 | 103 | 103 | 105 |
| `wolkit-todos` | Wolkit | 2 | 153 | 86 | 86 | 87 |
| `servo` | servo | 1 | 327 | 270 | 271 | 271 |

| App | Cold window | Warm window | Warm settled | Idle CPU ms/s | Idle wakeups/s | Background CPU ms/s | Scroll key to frame p50 / p95 | Frame interval p50 / p95 | Type key to frame p50 / p95 |
|---|---|---|---|---|---|---|---|---|---|
| `helium` | 3236 | 1800 | 3690 | 4.16 | 43.82 | 2.79 | 37.2 / 51.1 | 26.6 / 29.3 | 21.6 / 38.3 |
| `webapp` | 4316 | 1857 | 4595 | 3.41 | 41.3 | 3.82 | 42.7 / 46.8 | 26.5 / 27.4 | 21.1 / 39.3 |
| `steam` | 47318 | 30149 | 36006 | 56.85 | 496.22 | 48.75 | - | - | - |
| `code` | 3939 | 2235 | 14809 | 23.96 | 214.82 | 6.25 | 72.6 / 103.4 | 26.2 / 28.8 | 30 / 48.6 |
| `signal` | 5013 | 2679 | 6808 | 2.29 | 18.05 | 2.36 | - | - | - |
| `chromium` | 3460 | 1368 | 4548 | 5.66 | 133.1 | 4.36 | 29.7 / 45 | 25.7 / 30.1 | 3.7 / 19.8 |
| `firefox` | 3904 | 2081 | 7697 | 3.48 | 41.0 | 1.49 | 13.8 / 24.5 | 13.2 / 27.5 | 14.8 / 19.1 |
| `thunderbird` | 5144 | 2694 | 7338 | 1.47 | 12.7 | 0.57 | - | - | 9.9 / 12 |
| `text-editor` | 1293 | 956 | 5551 | 0.06 | 0.72 | 0.07 | 41.8 / 60.7 | 27.9 / 68 | 32.5 / 71.4 |
| `wolkit-todos` | 3721 | 685 | 748 | 2.37 | 114.38 | 2.54 | - | - | - / - |
| `servo` | 1157 | 388 | 1287 | 0.98 | 51.2 | 1.04 | 53.1 / 70.9 | 54.1 / 85 | 127.8 / 153.9 |

"Window" is the compositor's `openwindow` event after the launch (a mapped window has drawn its first buffer); "settled" is the first whole second in which the app's processes use under 5 % of a core. Scroll and type are scripted (15 pages down and up; 43 characters) and timed from the key press to the app's next `wl_surface.commit`; the frame interval is the gap between commits. `steam` and `signal` have no scripted interaction (they need an account to show content), `thunderbird` only types.

Where the memory of a web window sits (`docs/bench/webapps-roles.json`, PSS after 30 s idle in front, saver off):

| App | Total | Main process | GPU | Renderers (content) | Utility and helper processes | Zygotes |
|---|---|---|---|---|---|---|
| `helium`, one page | 406 | 137 | 50 | 142 (3 processes) | 43 | 33 |
| `signal` | 493 | 213 | 44 | 179 | 25 | 32 |
| `code` | 889 | 269 (2 `electron`) | 50 | 237 | 312 (5 utility processes) | 21 |
| `steam` | 957 | 215 (client) | - | 263 (`steamwebhelper`) | 46 | 7 `zygote` children, 414 together |
| `firefox` | 531 | 11 processes, all named `firefox` (an earlier run split them: parent and a helper 297 together, content processes 15 to 72 each) | | | | |
| `text-editor` | 103 | 103 (one process) | | | | |
| `wolkit-todos` | 86 | 8 (app) + 77 (shared Kit host) | | | | |

For a single trivial page the engine around it (main, GPU, utility, zygotes) is 263 MiB of 406, 65 %; the page itself is 142 MiB.

### Saver on versus off

| App | PSS after 2 min in the background, off | on | Saved | Swap in use, on | Back: key to frame off / on (ms) | PSS after coming back, off / on |
|---|---|---|---|---|---|---|
| `helium` | 404 | 301 | 26 % | 95 | 52 / 43 | 412 / 324 |
| `webapp` | 397 | 293 | 26 % | 97 | 42 / 44 | 403 / 313 |
| `steam` | 906 | 562 | 38 % | 281 | - | 912 / 570 |
| `code` | 802 | 354 | 56 % | 468 | 75 / 157 | 811 / 433 |
| `signal` | 489 | 309 | 37 % | 179 | - | 492 / 320 |
| `chromium` | 432 | 350 | 19 % | 81 | 40 / 44 | 441 / 373 |
| `firefox` | 505 | 234 | 54 % | 222 | n/m | 505 / 256 |
| `thunderbird` | 425 | 263 | 38 % | 116 | n/m | 425 / 295 |
| `text-editor` | 105 | 40 | 62 % | 64 | 21 / 13 | 110 / 64 |
| `wolkit-todos` | 87 | 41 | 53 % | 45 | - | 87 / 53 |
| `servo` | 271 | 85 | 69 % | 184 | 393 / 362 | 271 / 146 |

n/m: not meaningful (the app keeps committing frames in the background, so the first commit after the key is not caused by it). The one clear cost is `code`, whose first frame after coming back doubles (75 to 157 ms), in line with ADR 0026 (88 to 197 ms). `chromium` saves least (19 %) because its background content keeps running (120 to 165 wakeups per second in the background).

### Full runner output (both modes, all columns)

| App | Saver | Cold window / settled ms | Warm window / settled ms | PSS launch MiB | PSS 2 min front MiB | PSS 2 min back MiB | Swap back MiB | Idle front CPU ms/s | Idle front wakeups/s | Idle back CPU ms/s | Scroll key to frame p50/p95 ms | Scroll frame interval p50/p95 ms | Type key to frame p50/p95 ms | Back: settled ms / faults | Back: key to frame ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `helium` | on | 2476 / 5112 | 1117 / 2892 | 438.2 | 404 | 300.7 | 94.9 | 2.62 | 38.83 | 3.28 | 23.5 / 61.3 | 26.6 / 37.2 | 18.4 / 23 | 1024 / 2697 | 43.4 |
| `helium` | off | 3236 / 5353 | 1800 / 3690 | 437.6 | 404.2 | 404.5 | 0 | 4.16 | 43.82 | 2.79 | 37.2 / 51.1 | 26.6 / 29.3 | 21.6 / 38.3 | 579 / 1286 | 52.2 |
| `webapp` | on | 3731 / 6349 | 1545 / 3023 | 419.5 | 397 | 293.3 | 96.9 | 2.91 | 42.45 | 4.41 | 46.7 / 57 | 26.6 / 28.2 | 22.9 / 43.6 | 838 / 3098 | 43.8 |
| `webapp` | off | 4316 / 6357 | 1857 / 4595 | 420.1 | 397.1 | 397.3 | 0 | 3.41 | 41.3 | 3.82 | 42.7 / 46.8 | 26.5 / 27.4 | 21.1 / 39.3 | 742 / 1477 | 41.9 |
| `steam` | on | 43738 / 49446 | 29893 / 31456 | 944.4 | 904.7 | 562 | 281.3 | 65.56 | 495.28 | 65.57 | - / - | - / - | - / - | 690 / 2877 | - |
| `steam` | off | 47318 / 53612 | 30149 / 36006 | 963.6 | 903.8 | 905.9 | 0 | 56.85 | 496.22 | 48.75 | - / - | - / - | - / - | 769 / 2503 | - |
| `code` | on | 4846 / 26696 | 2333 / 18404 | 1124.2 | 839 | 354.1 | 468 | 23.68 | 196.48 | 10.82 | 76.4 / 156.6 | 26.4 / 44.8 | 75.2 / 99.7 | 1830 / 5020 | 156.9 |
| `code` | off | 3939 / 17141 | 2235 / 14809 | 1118.4 | 806.1 | 801.7 | 0 | 23.96 | 214.82 | 6.25 | 72.6 / 103.4 | 26.2 / 28.8 | 30 / 48.6 | 1853 / 1408 | 75.3 |
| `signal` | on | 6580 / 12362 | 3961 / 9099 | 590.2 | 484.3 | 309 | 178.7 | 2.07 | 19.28 | 4.01 | - / - | - / - | - / - | 901 / 2233 | - |
| `signal` | off | 5013 / 9378 | 2679 / 6808 | 590.3 | 485.9 | 489.2 | 0 | 2.29 | 18.05 | 2.36 | - / - | - / - | - / - | 902 / 596 | - |
| `chromium` | on | 3651 / 5448 | 1782 / 3933 | 428.2 | 429.6 | 350.4 | 80.8 | 21.39 | 219.6 | 27.01 | 43.3 / 46.4 | 26.4 / 28.1 | 21.2 / 40.4 | 1235 / 3131 | 43.5 |
| `chromium` | off | 3460 / 6753 | 1368 / 4548 | 470.6 | 432.9 | 432.1 | 0 | 5.66 | 133.1 | 4.36 | 29.7 / 45 | 25.7 / 30.1 | 3.7 / 19.8 | 955 / 1891 | 40.1 |
| `firefox` | on | 4801 / 9085 | 1825 / 6979 | 535.6 | 504.3 | 234 | 222 | 4 | 41.77 | 2.36 | 12.4 / 22.4 | 13 / 26.8 | 13.9 / 24 | 2153 / 2763 | 27.7 |
| `firefox` | off | 3904 / 11925 | 2081 / 7697 | 537.7 | 504.8 | 504.7 | 0 | 3.48 | 41 | 1.49 | 13.8 / 24.5 | 13.2 / 27.5 | 14.8 / 19.1 | 1952 / 1757 | 1.8 |
| `thunderbird` | on | 4134 / 7021 | 3422 / 8813 | 459.6 | 423.6 | 263.1 | 115.7 | 1.98 | 12.2 | 1.53 | - / - | - / - | 15 / 19.9 | 554 / 305 | 15.1 |
| `thunderbird` | off | 5144 / 10801 | 2694 / 7338 | 459.7 | 424.2 | 424.7 | 0 | 1.47 | 12.7 | 0.57 | - / - | - / - | 9.9 / 12 | 324 / 544 | 0.9 |
| `text-editor` | on | 2237 / 6024 | 1189 / 4965 | 102.7 | 102.6 | 39.6 | 64.2 | 0.11 | 0.78 | 0.14 | 34.8 / 49.7 | 26.2 / 52.8 | 26 / 72.6 | 1830 / 281 | 12.8 |
| `text-editor` | off | 1293 / 5621 | 956 / 5551 | 103.1 | 103 | 104.7 | 0 | 0.06 | 0.72 | 0.07 | 41.8 / 60.7 | 27.9 / 68 | 32.5 / 71.4 | 743 / 140 | 21.3 |
| `wolkit-todos` | on | 4047 / 4171 | 826 / 872 | 85 | 85.3 | 41.1 | 45.1 | 2.41 | 116.35 | 2.3 | - / - | - / - | - / - | 430 / 44 | - |
| `wolkit-todos` | off | 3721 / 3770 | 685 / 748 | 85.7 | 85.9 | 86.9 | 0 | 2.37 | 114.38 | 2.54 | - / - | - / - | - / - | 471 / 0 | - |
| `servo` | on | 1474 / 3208 | 490 / 1965 | 273.9 | 274.5 | 85.2 | 184 | 1.18 | 56.72 | 2.34 | 101.8 / 159 | 47.8 / 73.7 | 202.4 / 279.7 | 1272 / 3121 | 361.8 |
| `servo` | off | 1157 / 2292 | 388 / 1287 | 270.1 | 270.7 | 270.8 | 0 | 0.98 | 51.2 | 1.04 | 53.1 / 70.9 | 54.1 / 85 | 127.8 / 153.9 | 338 / 161 | 392.7 |

## How far to trust it

- One pass, three warm starts per row. Start-up times do not depend on the saver, so the on/off pairs show the noise: the cold window differs by 30 to 40 % for the same app (`helium` 2476 against 3236, `text-editor` 2237 against 1293). Treat start-up differences under about 40 % as noise and repeat before claiming them.
- Memory is stable: launch PSS differs between the two modes by under 3 % except for `chromium` (428 against 471).
- Software rendering (no GPU): the compositor's frame p50 is 9 to 18 ms and damage is 100 % for every app including the native ones, so frame times say little about GPU raster flags. Hardware must be measured before any GPU claim.
- `steam` takes 30 to 47 s to show a window; the VM has no account and probably no reachable update server, so this is not diagnosed as the client's normal start.
- The idle daemon is stopped for the run and the saver's session side is on, so idle numbers exclude lock, display-off and power-off events.
- `wolkit-todos` has 114 wakeups per second at idle: the native comparator is not quiet. That is the shared Kit host (`wol-kit-host`), a finding for the Wolkit owners and a reason not to read the gap below as "web against perfect".

## Gap against native

Native rows: `text-editor` (GTK 4, 103 MiB, 1 process) and `wolkit-todos` (86 MiB, 2 processes); saver off.

| Measure | A web window (`helium`, `webapp`) | Heaviest (`code`, `steam`) | GTK 4 | Wolkit |
|---|---|---|---|---|
| PSS after launch | 420 to 438 (4 to 5x) | 1118, 964 (11x, 9x) | 103 | 86 |
| Processes | 10 | 12, 15 | 1 | 2 |
| Warm start to window | 1.8 to 1.9 s | 2.2 s, 30 s | 1.0 s | 0.7 s |
| Warm start to settled | 3.7 to 4.6 s | 14.8 s, 36 s | 5.6 s | 0.7 s |
| Idle CPU ms/s, wakeups/s | 3.4 to 4.2, 41 to 44 | 24, 215 and 57, 496 | 0.06, 0.7 | 2.4, 114 |
| Scroll key to frame p50 | 37 to 43 ms | 73 ms, - | 42 ms | - |
| Type key to frame p50 | 21 ms | 30 ms, - | 33 ms | - |

Reading: the clear gaps are memory (4 to 11x), process count, idle wakeups and the heavy apps' start-up and settle time. Input latency and frame interval are not web gaps in this VM (the GTK window is no faster, and `firefox` at 13 ms intervals is the only row at the full refresh); that statement holds for software rendering only.

## Ranked wins (design only, nothing here is implemented)

Gains are estimates from the numbers above, not promises; each names what to re-measure.

1. **One shared engine host per user for the `chromium`-family apps and the web-app launcher** (mechanism: shared engine; one browser and GPU process, a content process per site; the `webengine` crate in progress is the vehicle). The fixed part of a window is 263 MiB for `helium`, 314 for `signal` and 652 for `code`. With three such apps open the saving is about two fixed parts, roughly 500 to 600 MiB of PSS, plus a warm start that does not create a browser, GPU and zygote (1.1 to 1.9 s today; the candidate `servo` shows 0.4 s for a single-process engine, the order to aim at). Re-measure: PSS with three apps open, warm window.
2. **Lower idle cost with background throttling and JavaScript patches for polling** (mechanism: timer alignment and intensive wake-up throttling in the engine, per-app patches that remove telemetry and poll loops). `steam` spends 57 ms of CPU and 496 wakeups every second doing nothing (6 % of a core, for as long as it runs); `code` 24 ms and 215; `chromium` 133 wakeups in front. A GTK window needs 0.7. Expected: halve `code`; 5 to 10x for `steam` if its poll loops are the cause (not diagnosed: profile the client page first). This is also the battery win.
3. **Discard hidden content in the engine ahead of the saver** (mechanism: tab discard and heap limits for hidden content processes). Renderers are 142 to 237 MiB of a window, and `chromium` shows the saver reaching only 19 % because its content keeps running. Discarding hidden renderers frees memory without zram traffic or swap (the saver leaves 80 to 470 MiB in swap per app). Expected: the saver's 26 to 56 % rises toward the renderer share, 35 to 60 %, and `code`'s return cost (157 ms) falls because less of the window is compressed and faulted back. Re-measure: background PSS and key to frame on return.
4. **Start-up: code cache and startup snapshot for the Electron-class apps** (mechanism: startup snapshot, preloaded shared engine). `code` needs 15 s to settle (2.2 s to its window), `signal` 6.8 s, `thunderbird` 7.3 s, against 0.7 s for the Wolkit app. Expected: halve the settle time for `code`; window time is dominated by the process set-up covered by win 1. Re-measure: settled ms over three warm runs, noise floor 40 %.
5. **Native chrome and dialogs for web-app windows** (mechanism: the launcher draws title bar, menus and file and print dialogs with Wolkit and leaves only the page to the engine). Not quantified by this baseline (the harness has no per-feature cost for it); it removes the part of the look-and-feel gap that is not memory or latency. Add a row when the first prototype exists.
6. **GPU raster and compositing flags** (mechanism: engine flags, damage-aware compositing). Not rankable here: the VM has no GPU, the compositor reports 100 % damage for every app, and frame intervals of 26 ms (half the 75 Hz refresh) appear for the native GTK window too. It needs the same harness on hardware; until then no gain is claimed.
7. **The Memory Saver itself** (not a web change): it already returns 19 to 69 % in the background. Open items from the data: swap grows to 468 MiB for `code`, and the saver does nothing for start-up cost. Keep it; do not count it as a web-runtime win.

## Rerun

    WOLVM=.../wolos-iso/tests/wolvm/wolvm WOLVM_HOME=... OUT=out sh tools/bench/webapps/run.sh VM

Same VM image, same settings; compare `result.json` row by row with `docs/bench/webapps-baseline.json`. Treat start-up differences inside the 40 % band as unchanged.
