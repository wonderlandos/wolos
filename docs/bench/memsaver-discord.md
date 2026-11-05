# Discord under a memory budget (ADR 0027 amendment)

Concept: the lead asked for Discord in 100 MB, in front and behind, "no 'oh but the js runtime'". This
record is how far that went on WolOS 0.12.1 in a VM, what held it there, and what did not reach 100 MB.
Why: the budget table (`crates/core/src/memsaver/budget.rs`) and every later app row are judged by these
numbers. All values are MiB of PSS summed over every process of the app's cgroup scope unless stated.

## Identity

| | |
|---|---|
| Date | 2026-11-04, 19:40 to 22:10 UTC |
| Image | WolOS 0.12.1 (`wolos-2026.11.04-x86_64.iso`), installed with `wolos.autoinstall`; kernel `7.3.0-5-wolos`; Discord from the Arch `discord` package, **logged out** (the login screen is what runs; a logged-in client holds more) |
| VM | wolvm on the forge: 4 vCPU (Xeon Platinum 8375C), 8 GiB, KVM, `virtio-vga` (software rendering), zram 3.9 GiB zstd, Hyprland session, 1280x800 |
| Probes | `tools/bench/discord/`: `msx.py` (PSS, `memory.current`, swap, PSI of the scope), `phases.sh` (fresh start, 60 s focused idle, 30 s of real key presses through QEMU, background on another workspace, back in front), `cdp.py` (page responsiveness through DevTools: animation frames in front, a constant evaluated in the background = how long a message waits), `govern.py` (the rejected governor), `evict.py`, `duty.sh`, `push.py` |
| Runs | one run per row, 3 samples at launch, 6 idle, 6 typing, the background window 60 s to 10 min; raw numbers per run in `memsaver-discord.json`. n is small: the spread between runs of the same configuration is about 5 MiB at rest and up to 50 MiB while typing |

## Before: today's Memory Saver

| State | PSS |
|---|---|
| Start, first 15 s | 554 |
| Focused, idle | 543 (max 552) |
| Focused, typing | 551 |
| Background, 1 min | 294 |
| Background, 10 min | 443 (median over the window 429) |
| Back in front | 443 |

Today's saver adopts the running app into `app-wol-discord-<pid>.scope` and gives its memory back once.
The renderer faults it in again; after ten minutes behind, 443 MiB are back.

## What did not work

| Try | Result (PSS focused idle) | Why |
|---|---|---|
| Adopt the running app, `memory.high` 100 MiB on its scope | 374 (typing 389, background 313, back 341) | `memory.current` sits at 100, PSS does not move: the pages charged before the adoption stay in the session scope (`memory.min` 256 MiB there) and swap-ins go back to the old charge |
| Start in its own `app-discord-<pid>.scope`, `memory.high` 100 MiB from the first instant | 263 (typing 278) | most likely the cached pages of the app's own files, charged to another scope earlier, stay where they are and are mapped from there; the next row changes that |
| The same, the app's cached files dropped first (`posix_fadvise DONTNEED`), limit from the first instant | PSS 37 to 43, but the app is stuck: swap grows 289 to 533 MiB (100 MiB limit), 126 to 256 (75 MiB), 423 to 1121 (150 MiB) and the window stays on its "Starting..." card (`e7`, `e8`, `e9`, screenshots taken at the end of each) | the cold start does not fit; under the limit it never finishes |
| The same, limit set after the start | PSS 63 to 81 | works: this is the budget's shape (`start_ms`) |
| A governor that follows the measured PSS and moves `memory.high` | `memory.current` 117 to 198 MiB against a target of 100 (two short runs, `e11`, `e12`) | it let the limit float; stopped, fixed values are used |

## After: the budget

What runs: the shell starts Discord by `systemd-run --user --scope --unit=app-discord-$$.scope
--property=MemorySwapMax=2048M` after dropping its cached files (`crates/shell/src/kit/mod.rs`
`budget_launch`); wold sets `memory.high` 150 MiB 60 s after the window first shows, 100 MiB once it has
been behind for the saver's delay (`crates/api/src/memsaver/engine.rs` `run_budget`). The freeze cycle (4 s
frozen, 1 s thawed while behind) is in the code and **ships off** (`ELECTRON.duty = None`): it raised the
wait of a message and no saving was measured from it. The runs below used the same limits from the bench
scripts, **with the cycle on** (`phases.sh` with `FRONT_HIGH=150M BACK_HIGH=100M DUTY=4:1`; the "behind" rows
are therefore not the shipped configuration, and no run without the cycle was made: a freeze releases no
memory, so the PSS behind is expected to be close, not measured) (`phases.sh` with `FRONT_HIGH=150M BACK_HIGH=100M DUTY=4:1`); the wold engine itself is covered by unit
tests and was NOT run in a guest (see the end).

| State | Before | After | Runs |
|---|---|---|---|
| Focused, idle (60 s after start) | 543 | median 93, range 89 to 97 | 7 runs |
| Focused, typing | 551 | 80 to 91; one run 140 (max 151) | 7 runs |
| Background, 1 min | 294 | 46 to 62 (81 once, duty 3 s / 2 s) | 5 runs |
| Background, 5 to 10 min | 443 | median 57 to 60, max 67 to 92, 82 at 600 s | m1 (5 min, 19 samples), w2-long (10 min, 39 samples) |
| Back in front | 443 | 71 to 89 | 3 runs |
| Start, first 15 s (no limit yet) | 554 | 41 to 73 | 7 runs |

Peaks: focused idle reached 96 to 105, typing up to 119 in the 150 MiB runs. The 100 MiB holds at the median
and not at every sample.

`memory.swap.current` of the scope is 270 to 350 MiB in every state: what the limit pushed out lives in
zram. The compressed size of that was not recorded per run (ADR 0026 measured about 3.4 : 1 for the four
apps on this guest). The saving to the machine is the PSS drop minus the compressed part.

Why 150 in front and not 100: with `memory.high` 100 MiB in front, 24 to 46 % of the time the scope was
waiting for memory (PSI `full` avg10 median 24 and 29 while idle in front, `m2-*`, runs cut short when the host ran out of
memory; `w2-L1-100`: median 14 while typing) and the front frame p95 was 947 ms (`w3-L1-100`) against
32 to 46 ms at 150. 125 MiB stalled 42 % of the samples.

## Engine switches

Tried on the lean side: `--force-prefers-reduced-motion --in-process-gpu --num-raster-threads=1
--renderer-process-limit=1 --disable-site-isolation-trials --js-flags="--optimize-for-size
--max-semi-space-size=1"`, against the default command line, same limits (`m1-*`, `w2-*`, `w3-*`):

| | Default | Lean |
|---|---|---|
| Focused idle | 92 (m1), 97, 96 | 89, 93 (m1), 95, 93 |
| Typing | 80 (m1), 90, 140 | 83, 91 (m1), 88, 86 |
| Background (m1) | 57 | 56 |
| Processes | 9 | 8 |

No difference beyond the spread: the GPU process folds into the main one (8.7 MiB there, the main process
44 against 38 here), so the sum stays. They are in `webruntime::THIRD_PARTY` as `Candidate` rows and do not
ship: two of them weaken the engine (site isolation, one renderer) for no measured gain.

## Responsiveness

| Probe (`cdp.py`) | Result |
|---|---|
| Front, 20 animation frames, limit 150 | p50 21 to 28 ms, p95 32 to 46, max 158 to 509 |
| Front, limit 100 | p50 44, p95 947, max 1323 |
| Back in front after the background, first 10 frames | p50 42 to 47 ms, p95 3129 to 4566, max 4430 to 5864 (limit back to 150 and the scope thawed; the app pages in) |
| Background: how long an evaluation waits, limit 100 from the start, duty cycle on (`w3-*`; the invocation was not recorded, 4 s / 1 s assumed) | 31 probes: median 2.6 s, 23 of 31 under 3.6 s, then 6.4 to 22 s |
| Background: the same, limit 150 dropping to 100 at the switch | 9 to 11 probes: 3.6 to 56 s, median about 11 s |

The goal was a message waiting under 5 s. It is NOT met in the first minute behind with the cycle on, which is why it ships off; with it off the wait was not probed (the probes above are in
that minute, while the limit drops from 150 to 100 and the scope pages out). The duty cycle alone keeps a
frozen scope silent for at most `stop_ms` (4 s); what exceeds it is the app paging its working set back in from
zram inside a 100 MiB limit while thawed for 1 s. The steady state after the first minute was not probed:
the runs that were meant to (`m1-*`, `m2-*`) lost their background probe (the guest agent takes one caller at a
time; `phases.sh` now runs the probe as a guest unit) or were cut short when the forge host ran out of memory.

## What reached 100 MB and what did not

Reached: PSS of the whole app in front at rest (median 93, from 543), behind (46 to 62, from 443), without
changing the app.

Not reached, with the reason:

- **The start.** The first 60 s run without a limit (the cold start never finished under one): the scope
  reads its files in, and the 41 to 73 MiB it shows then rise to about 90 once the window is up.
- **Every sample under 100 in front.** Peaks 105 to 119, one typing run at 140 (151 max): the limit is 150
  (`memory.high` above PSS by the kernel memory and unmapped cache the scope is charged for), the PSS target 100
  is a median, not a ceiling. A 100 MiB front limit stalls the app (PSI 24 to 46 %).
- **A message under 5 s in the first minute behind** (above).
- **Apps not started by the shell.** The budget needs the start in the scope; a terminal, a script or another
  launcher gives the old behaviour (adopted: 543 focused, 443 after ten minutes behind; the engine sets no limit and no freeze on such a scope). `budget::launch_sh` is the function a
  launcher calls.
- **A logged-in Discord, voice, a call.** Not measured: all runs are logged out, and no voice session was
  opened. The engine never freezes a scope that plays sound (unit test), and keeps its limit.

## Not verified

The wold engine run in a guest: a new wold in the VM puts the session on its lock screen, and typing the
test account's password was refused, so the shipped values were driven from the bench scripts. The shell
launch hook was unit tested and run on the forge (`kit::budget_tests`), not from a Dock click.
