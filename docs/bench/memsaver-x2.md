# Memory Saver with the runtime tuning, measured in a real session (MEMSAVER-X2)

Concept: how much memory the apps of a WolOS session keep in use, in front, behind and back in front, with
the Memory Saver of 0.12.1 (`stock`) and with the one this tree ships (`shipped`: the engine switches of
ADR 0061 wired into the launcher and into wold). Why: the lead's order (2026-11-05) was to test Memory Saver
"against Discord and the others and see how much memory stays in use in focus and so on". The earlier records
(`memsaver-discord.md`, `js-runtime-x.md`) drove values from scripts; this one starts each app the way a
person does and lets wold decide. How: `tools/bench/memsaver-x2/` (`x2drive.py` on the host, `x2g.py` in the
guest, `x2plan.py` for the two modes, `x2report.py` for the tables); raw runs in
`docs/bench/results/memsaver-x2/*.jsonl`.

## Identity

| | |
|---|---|
| Date | 2026-11-05, 02:32 to 05:50 UTC (five guests in parallel) |
| Guests | wolvm on the forge, five copies (`x2a`..`x2e`) of one 0.12.1 install with kernel `7.3.1-1-wolos`, 4 vCPU (Xeon Platinum 8375C), 8 GiB, KVM, `virtio-vga` (software rendering), zram 3.9 GiB zstd, THP `madvise`, 1280x800, `hyprland-wolos 0.56.2-6`, `mesa 26.2.6-1`. `wolos-system 0.12.1-1` |
| Apps | `discord 1.0.168-1`, `code 1.138.0-1` (Code - OSS on `electron42 42.11.18-1`), `signal-desktop 8.33.0-1`, `thunderbird 158.0.1-1`, `helium-browser 0.20.3.1-1`, WebKitGTK `MiniBrowser` of `webkitgtk-6.0 2.54.2-1`, `gnome-text-editor 51.0-1`, the Wolkit `todos` example (release build of 2026-11-03) |
| Accounts | none. Discord shows its login page, Signal its link screen, Thunderbird its account wizard, VS Code its welcome page, Helium a new tab, MiniBrowser `https://discord.com/login`. A logged-in client holds more. No password was typed into any app |
| `stock` | the image as installed: its own `wold`, `wol-shell`, `wol-search`, `wol-realtime`; no `wolos-mm`; the kernel knobs put back to the kernel defaults at the start of the mode (`fault_around_bytes` 65536, KSM off; the image carries the tuning file of ADR 0061, which `stock` overrides at run time). Memory Saver on, App Nap off |
| `shipped` | `stock` plus a systemd-sysext layer with the binaries of this tree built on the forge (`wold`, `wol-realtime`, `wolos-mm`, `wol-shell`, `wol-search`, `wol-settings`, `wol-store`, `wol-app`, `wol-kit-host`; wolos `wave3/memsaver-x2`, shell `wave3/memsaver-x2`), `wolos-mm.service` enabled, and the image's tuning file in force (`fault_around_bytes` 4096, KSM for opted-in apps). Memory Saver on, App Nap off |
| `shipped-nopageout` | Discord only, `x2e`: `shipped` with a `wold` built with `PAGEOUT_EVERY_MS = u64::MAX` (a measurement build, not in the tree): the budget's limit but no page-out |
| Launch | the real path: Search (`wol-search`, which calls `kit::spawn_app`), the name typed as key events through QEMU, Return. Behind = workspace 2 by the key combination Ctrl+Alt+2, back = Ctrl+Alt+1. The idle daemon is stopped, caches dropped before each start, the previous app closed. One throwaway start per app per mode first (profile creation, first-run pages) |
| Runs | 3 per app and mode, medians. Phases from the window mapping: 15 s, front idle 60 s, typing 60 s (`wolvm type`, real key events), behind 10 min (`1 min` is the sample 60 s in, `10 min` the one at 600 s), back in front 20 s. ~14 min a run |
| Metric | PSS and RSS summed over the whole app: the process tree of the window's pid plus every process of its `app-*.scope` (`smaps_rollup`), split into anonymous, file and shared memory; SwapPss; the scope's `memory.current`; context switches and CPU ticks of the tree; the guest's `MemAvailable`. `x2g.py` |
| System Monitor | `wol-monitor` of `wave3/sysmon` (shell `5cf864a`, its own base) built on the forge and layered into every guest; opened through Search on the behind workspace; screenshots in `/tmp/memsaver-x2-shots/` |

## What the shipped build does, per app (verified in the guests)

- Fault-around 4 KiB and the KSM opt-in reach every app of the machine or none: `fault_around_bytes` is a
  kernel knob (every sample of every `shipped` run reads 4096, every `stock` run 65536).
- The launcher puts `wolos-mm exec-merge --` in front only for an app whose engine it knows: Discord (a
  budget row, Electron) and WebKitGTK's MiniBrowser. KSM merged pages (`/sys/kernel/mm/ksm/pages_sharing`)
  during the runs: MiniBrowser 29.6k in 3 of 3 runs, Discord 343 to 8840, every other app 0.
- Discord starts in its own `app-discord-<pid>.scope` with `memory.high` 150 MiB 60 s after the window and 100
  MiB behind (`budget_launch`, `run_budget`); only Discord is paged out every 30 s while behind (the
  `Page-outs counted` column: 20 in 10 minutes; 0 for every other app). VS Code, Signal, Thunderbird and Helium
  have no budget row and no engine class, so their `shipped` numbers are fault-around plus the one give-back.
- Settings > Energy > Memory Saver shows the one switch with the sentence "Apps you aren't using give their
  memory back, apps share identical memory, and Discord keeps to a limit." (`docs/bench/results/memsaver-x2/shots/settings-memory-saver.png`; the System Monitor Apps page, before and after, for Discord and VS Code: `discord-antes-depois.png`, `vscode-antes-depois.png` in the same directory);
  `wol doctor memory` in the guest answers: "Apps map only the file pages they touch (4 KB at a time).
  Identical memory of apps is shared (1 MB now), because this computer has one account. Memory Saver is on.
  Discord is in the background and gave its memory back 13 times."

## PSS of the whole app, MiB, median of 3 runs

| App | Mode | n | Start, 15 s | Front, idle 60 s | Front, typing 60 s | Behind 1 min | Behind 10 min | Back in front, 20 s |
|---|---|---|---|---|---|---|---|---|
| Discord | stock | 3 | 639 | 540 | 546 | 283 | 438 | 442 |
| Discord | shipped | 3 | 473 | 15 | 51 | 33 | 28 | 35 |
| Discord | shipped-nopageout | 3 | 569 | 419 | 45 | 36 | 35 | 42 |
| VS Code (Code - OSS) | stock | 3 | 1086 | 882 | 814 | 296 | 423 | 439 |
| VS Code (Code - OSS) | shipped | 3 | 821 | 761 | 691 | 215 | 317 | 321 |
| Signal | stock | 3 | 589 | 491 | 491 | 289 | 306 | 306 |
| Signal | shipped | 3 | 508 | 409 | 409 | 184 | 201 | 201 |
| Thunderbird | stock | 3 | 453 | 446 | 428 | 240 | 292 | 320 |
| Thunderbird | shipped | 3 | 371 | 366 | 348 | 147 | 198 | 229 |
| Helium | stock | 3 | 427 | 405 | 405 | 297 | 308 | 318 |
| Helium | shipped | 3 | 325 | 302 | 302 | 195 | 204 | 211 |
| WebKitGTK MiniBrowser | stock | 3 | 778 | 774 | 804 | 224 | 386 | 495 |
| WebKitGTK MiniBrowser | shipped | 3 | 673 | 610 | 602 | 156 | 317 | 402 |
| Text Editor (GTK 4) | stock | 3 | 96 | 97 | 101 | 42 | 42 | 60 |
| Text Editor (GTK 4) | shipped | 3 | 78 | 78 | 83 | 22 | 22 | 40 |
| Wolkit Todos | stock | 3 | 77 | 77 | 78 | 34 | 33 | 44 |
| Wolkit Todos | shipped | 3 | 60 | 60 | 62 | 16 | 16 | 27 |

Reading the Discord rows: the budget limits nothing for the first 60 s (the cold start does not finish under
a limit), so `Start, 15 s` is the unlimited start (473 against 639: the 4 KiB fault-around). The limit lands 67 to
108 s after the window (67 s in the three shipped runs), around the `Front, idle 60 s` sample, which is why that cell is 15 for `shipped` (limit
in force, three runs) and 419 for `shipped-nopageout` (limit not yet in force in two of three runs); the next
column is after it everywhere. `stock` is the 0.12.1 Memory Saver: one give-back 30 s behind, and 438 MiB are back
by minute 10 (the earlier record has 443).

## RSS, MiB, median of 3 runs

| App | Mode | n | Start, 15 s | Front, idle 60 s | Front, typing 60 s | Behind 1 min | Behind 10 min | Back in front, 20 s |
|---|---|---|---|---|---|---|---|---|
| Discord | stock | 3 | 1134 | 994 | 1001 | 728 | 888 | 899 |
| Discord | shipped | 3 | 751 | 88 | 131 | 111 | 101 | 112 |
| Discord | shipped-nopageout | 3 | 822 | 696 | 121 | 111 | 108 | 119 |
| VS Code (Code - OSS) | stock | 3 | 1769 | 1570 | 1500 | 975 | 1105 | 1118 |
| VS Code (Code - OSS) | shipped | 3 | 1133 | 1088 | 1018 | 538 | 638 | 642 |
| Signal | stock | 3 | 970 | 871 | 871 | 669 | 687 | 688 |
| Signal | shipped | 3 | 721 | 622 | 622 | 397 | 415 | 416 |
| Thunderbird | stock | 3 | 708 | 686 | 669 | 479 | 532 | 560 |
| Thunderbird | shipped | 3 | 485 | 473 | 455 | 254 | 305 | 336 |
| Helium | stock | 3 | 1185 | 1149 | 1150 | 1042 | 1057 | 1068 |
| Helium | shipped | 3 | 676 | 648 | 649 | 542 | 551 | 559 |
| WebKitGTK MiniBrowser | stock | 3 | 1128 | 1121 | 1151 | 566 | 601 | 714 |
| WebKitGTK MiniBrowser | shipped | 3 | 855 | 868 | 898 | 387 | 468 | 570 |
| Text Editor (GTK 4) | stock | 3 | 155 | 155 | 159 | 100 | 100 | 118 |
| Text Editor (GTK 4) | shipped | 3 | 115 | 115 | 120 | 60 | 60 | 77 |
| Wolkit Todos | stock | 3 | 134 | 134 | 135 | 90 | 90 | 101 |
| Wolkit Todos | shipped | 3 | 95 | 95 | 96 | 50 | 50 | 62 |

## Where the memory is (PSS split)

| App | Mode | 15 s: anon | file | shmem | Behind 10 min: anon | file | shmem |
|---|---|---|---|---|---|---|---|
| Discord | stock | 396 | 222 | 22 | 209 | 221 | 8 |
| Discord | shipped | 347 | 121 | 6 | 14 | 13 | 0 |
| Discord | shipped-nopageout | 428 | 128 | 13 | 20 | 15 | 0 |
| VS Code (Code - OSS) | stock | 834 | 236 | 16 | 187 | 237 | 2 |
| VS Code (Code - OSS) | shipped | 705 | 99 | 17 | 197 | 118 | 3 |
| Signal | stock | 401 | 182 | 6 | 118 | 183 | 4 |
| Signal | shipped | 402 | 100 | 6 | 94 | 102 | 5 |
| Thunderbird | stock | 244 | 169 | 41 | 123 | 167 | 2 |
| Thunderbird | shipped | 243 | 88 | 40 | 110 | 87 | 1 |
| Helium | stock | 198 | 216 | 14 | 88 | 216 | 6 |
| Helium | shipped | 195 | 114 | 17 | 78 | 116 | 10 |
| WebKitGTK MiniBrowser | stock | 583 | 191 | 6 | 208 | 183 | 1 |
| WebKitGTK MiniBrowser | shipped | 563 | 104 | 6 | 208 | 106 | 1 |
| Text Editor (GTK 4) | stock | 61 | 34 | 1 | 7 | 34 | 1 |
| Text Editor (GTK 4) | shipped | 62 | 15 | 1 | 6 | 15 | 1 |
| Wolkit Todos | stock | 51 | 27 | 0 | 7 | 26 | 0 |
| Wolkit Todos | shipped | 51 | 10 | 0 | 6 | 10 | 0 |

## Behind 10 minutes, and what it costs

| App | Mode | PSS | SwapPss | memory.current | Wakeups/s front idle | Wakeups/s behind (5-10 min) | CPU % of a core, front idle | CPU % of a core, behind (5-10 min) | Major faults/s, front idle | Major faults/s, behind | Page-outs counted |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Discord | stock | 438 | 86 | 313 | 220 | 225 | 3 | 1 | 0 | 1 | 0 |
| Discord | shipped | 28 | 258 | 100 | 565 | 426 | 13 | 18 | 132 | 1102 | 20 |
| Discord | shipped-nopageout | 35 | 251 | 100 | 188 | 413 | 2 | 14 | 0 | 964 | 0 |
| VS Code (Code - OSS) | stock | 423 | 365 | 414 | 67 | 38 | 2 | 1 | 0 | 62 | 0 |
| VS Code (Code - OSS) | shipped | 317 | 363 | 332 | 166 | 32 | 4 | 0 | 0 | 3 | 0 |
| Signal | stock | 306 | 184 | 111 | 63 | 130 | 2 | 0 | 0 | 2 | 0 |
| Signal | shipped | 201 | 208 | 14 | 63 | 135 | 2 | 0 | 0 | 2 | 0 |
| Thunderbird | stock | 292 | 86 | 179 | -2 | 7 | 0 | 0 | 0 | 13 | 0 |
| Thunderbird | shipped | 198 | 94 | 4 | 1 | 5 | 0 | 0 | 0 | 21 | 0 |
| Helium | stock | 308 | 88 | 54 | 1086 | 269 | 17 | 1 | 0 | 2 | 0 |
| Helium | shipped | 204 | 93 | 2 | 15 | 6 | 0 | 0 | 0 | 1 | 0 |
| WebKitGTK MiniBrowser | stock | 386 | 322 | 320 | 342 | 38 | 2 | 0 | 0 | 1 | 0 |
| WebKitGTK MiniBrowser | shipped | 317 | 225 | 178 | 463 | 44 | 6 | 0 | 0 | 0 | 0 |
| Text Editor (GTK 4) | stock | 42 | 59 | 1 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| Text Editor (GTK 4) | shipped | 22 | 61 | 0 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| Wolkit Todos | stock | 33 | 45 | 0 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |
| Wolkit Todos | shipped | 16 | 46 | 1 | 1 | 1 | 0 | 0 | 0 | 0 | 0 |

`Wakeups/s` are context switches a second of the whole tree; a negative value is a counter that fell when a
thread ended. CPU is a share of one core (4 vCPU guest). `Page-outs counted` is `AppStatus.pageouts` from
`/var/run/wol-memsaver.json` at the end of the behind phase.

## What the machine has left

| App | Mode | Start, 15 s | Front, idle (last sample) | Behind 10 min |
|---|---|---|---|---|
| Discord | stock | 6388 | 6582 | 6652 |
| Discord | shipped | 6077 | 6478 | 6699 |
| Discord | shipped-nopageout | 6349 | 6589 | 6780 |
| VS Code (Code - OSS) | stock | 6039 | 6244 | 6580 |
| VS Code (Code - OSS) | shipped | 6163 | 6227 | 6510 |
| Signal | stock | 6494 | 6578 | 6686 |
| Signal | shipped | 6501 | 6586 | 6747 |
| Thunderbird | stock | 6633 | 6660 | 6759 |
| Thunderbird | shipped | 6627 | 6658 | 6779 |
| Helium | stock | 6672 | 6707 | 6779 |
| Helium | shipped | 6649 | 6697 | 6785 |
| WebKitGTK MiniBrowser | stock | 6289 | 6327 | 6636 |
| WebKitGTK MiniBrowser | shipped | 6325 | 6387 | 6619 |
| Text Editor (GTK 4) | stock | 6857 | 6865 | 6911 |
| Text Editor (GTK 4) | shipped | 6838 | 6840 | 6893 |
| Wolkit Todos | stock | 6873 | 6883 | 6917 |
| Wolkit Todos | shipped | 6832 | 6834 | 6868 |

The PSS drops are not all memory the machine gets back. Fault-around makes a process map fewer file pages; the
pages stay in the page cache, which `MemAvailable` counts as available. The anonymous memory a limit or a
page-out pushes out lives in zram: a one-shot read of `/sys/block/zram0/mm_stat` on `x2e` with Discord shipped
and behind was 170 MiB of original data in 44 MiB (3.9 : 1), so Discord's 258 MiB of SwapPss cost about 66 MiB
of RAM (not recorded per run). The guest-wide `MemAvailable` moves by tens of MiB between configurations (the
spread between runs is of that size), not by the hundreds of MiB the PSS column shows, except where anonymous
memory really went (Discord, +47 MiB at minute 10 against `stock`).

## Coming back after 10 minutes

| App | Mode | First visible change after the key, s | Last visible change, s | CPU in the first 4 s, ms | Major faults in 20 s | PSS at the return -> 20 s later |
|---|---|---|---|---|---|---|
| Discord | stock | 1.1 | 1.1 | 210 | 2836 | 439 -> 442 |
| Discord | shipped | 1.1 | 9.7 | 1540 | 29307 | 21 -> 35 |
| Discord | shipped-nopageout | 1.1 | 12.7 | 1830 | 35441 | 29 -> 42 |
| VS Code (Code - OSS) | stock | 0.9 | 2.2 | 240 | 4502 | 419 -> 439 |
| VS Code (Code - OSS) | shipped | 0.9 | 0.9 | 180 | 2364 | 310 -> 321 |
| Signal | stock | 0.9 | 1.6 | 80 | 274 | 306 -> 306 |
| Signal | shipped | 0.9 | 0.9 | 80 | 411 | 201 -> 201 |
| Thunderbird | stock | 1.1 | 1.1 | 200 | 1518 | 292 -> 320 |
| Thunderbird | shipped | 1.1 | 1.1 | 200 | 3240 | 198 -> 229 |
| Helium | stock | 0.9 | 0.9 | 780 | 1092 | 308 -> 318 |
| Helium | shipped | 0.9 | 1.6 | 80 | 1522 | 204 -> 211 |
| WebKitGTK MiniBrowser | stock | 1.0 | 2.6 | 2010 | 21880 | 386 -> 495 |
| WebKitGTK MiniBrowser | shipped | 1.2 | 2.5 | 2060 | 18344 | 317 -> 402 |
| Text Editor (GTK 4) | stock | 1.3 | 2.0 | 250 | 3864 | 42 -> 60 |
| Text Editor (GTK 4) | shipped | 1.0 | 1.8 | 230 | 4778 | 22 -> 40 |
| Wolkit Todos | stock | 1.1 | 4.5 | 180 | 1811 | 33 -> 44 |
| Wolkit Todos | shipped | 1.1 | 1.1 | 120 | 1861 | 16 -> 27 |

The first column is the time from the key press to the first frame that differs from the behind screen, the
second to the last frame that still changes: QEMU screenshots about 0.8 s apart (a resolution, not a latency; the
workspace animation is in both). `Major faults` are those of the whole tree in the 20 s after the key.

## What the person feels

- Every app except Discord is back in about a second and settled in 0.9 to 2.6 s, `stock` and `shipped` alike
  (Wolkit Todos `stock` settles in 4.5 s in all three runs, `shipped` in 1.0 to 1.4 s; VS Code 2.2 against 0.9 s).
  The shipped numbers are lower in memory and no slower to return.
- Discord `shipped` is back in 1.1 s but keeps changing for 9.7 s (12.7 s without page-out): 29k major faults and
  1.5 s of CPU in the first four seconds, against 2.8k faults and 0.2 s for `stock`. While behind it burns 18 % of a core
  (stock 1 %) paging against its 100 MiB limit (about 1100 major faults a second), the System Monitor lists it as
  "moderate energy" (18 % CPU; "high energy" at 32 % CPU in another screenshot), and in front the limit costs 13 % of a
  core at idle against 3 %. The page-out itself is not the cost: without it the behind PSS is 35 against 28 and
  the return is no faster.
- The earlier record (`memsaver-discord.md`) judged the 150 MiB front limit by 20 animation frames (p50 21 to
  28 ms); this run did not measure frames, only that typing reaches the field and the page repaints.

## Not measured

- Frame p50 in front and the latency of the first key after the return (no DevTools port, no input-to-frame probe
  in this harness); the return times above are screenshot-based.
- A logged-in Discord or Signal, mail in Thunderbird, a project in VS Code; the Atlas browser (not in the image;
  Helium is the Chromium browser measured).
- The Memory Saver with App Nap on, a second account (KSM is off by policy then), real hardware, GPU rendering.
- The kernel's own zram size per run (one reading only, above).

## Run notes

- A first start of the harness typed the app names into a locked session (the idle daemon had locked the guests
  before the harness began): the three guests looked hung and were restarted; no measurement came from them.
- `x2a` `shipped` Discord run 1 shared the guest with a WebKit MiniBrowser left from a test (killed before run
  2); its start is 510 MiB against 473 and 473 in runs 2 and 3, and the other phases are within the spread of the later runs.
- The System Monitor Apps page in the runs' own screenshots only opened in the first stock runs; the final
  screenshots (`/tmp/memsaver-x2-shots/apps/`) come from a second, shorter pass (2 minutes front, 2.5 behind) with
  the same launcher, and the stock VS Code, Thunderbird and Helium ones are the 10-minute ones of the runs.

## Runs

| App | Mode | Runs ok | Runs failed | Window after the launcher, s (median) |
|---|---|---|---|---|
| Discord | stock | 3 | 0 | 21.2 |
| Discord | shipped | 3 | 0 | 21.2 |
| Discord | shipped-nopageout | 3 | 0 | 18.9 |
| VS Code (Code - OSS) | stock | 3 | 0 | 20.8 |
| VS Code (Code - OSS) | shipped | 3 | 0 | 19.2 |
| Signal | stock | 3 | 0 | 19.0 |
| Signal | shipped | 3 | 0 | 20.5 |
| Thunderbird | stock | 3 | 0 | 22.3 |
| Thunderbird | shipped | 3 | 0 | 20.5 |
| Helium | stock | 3 | 0 | 19.9 |
| Helium | shipped | 3 | 0 | 19.4 |
| WebKitGTK MiniBrowser | stock | 3 | 0 | 19.4 |
| WebKitGTK MiniBrowser | shipped | 3 | 0 | 19.4 |
| Text Editor (GTK 4) | stock | 3 | 0 | 19.2 |
| Text Editor (GTK 4) | shipped | 3 | 0 | 19.7 |
| Wolkit Todos | stock | 3 | 0 | 19.6 |
| Wolkit Todos | shipped | 3 | 0 | 19.6 |
