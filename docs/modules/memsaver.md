# Memory Saver and per-app memory budgets (ADR 0026, ADR 0027 amendment)

Memory Saver (ADR 0026) gives a background app's memory back to zram. Since the 2026-11-04 amendment an app can also
carry a **budget** (ADR 0027, amendment 2026-11-04): a number of MiB of PSS it is held to, in front and
behind, whatever its engine does. Discord is the first, with 100 MiB. The numbers and what did and did
not reach 100 MiB are in [bench/memsaver-discord.md](../bench/memsaver-discord.md).

## Why a budget needs a start in the right cgroup

A page is charged to the cgroup of the process that first touched it and stays there. An app adopted
into its own scope after it started (what Memory Saver does for every other app) keeps its first
hundreds of MiB charged to the session scope, which has `memory.min` 256 MiB; a `memory.high` on the new
scope pins `memory.current` and cannot move PSS (measured: 390 MiB of PSS under `memory.high` 100 MiB).
So a budgeted app is **born** in its scope:

1. the cached pages of its own files are dropped (`budget::evict`, `posix_fadvise(DONTNEED)` on the
   files under the table row's `evict` directories; pages a process maps stay),
2. the launcher runs `systemd-run --user --scope --unit=app-<engine name>-$$.scope
   --property=MemorySwapMax=2048M -- <app> <shipped engine switches>` (`budget::launch_sh`; `$$` is the pid the
   app keeps through the execs). Chromium moves itself into `app-<name>-<pid>.scope` unless that unit
   exists, so choosing that exact name keeps the app where it started.

## Pieces

| Piece | Where | What |
|---|---|---|
| Budget table | `crates/core/src/memsaver/budget.rs` (`TABLE`, `ELECTRON` default row) | per app: PSS promise, `memory.high` in front and behind, `start_ms`, swap cap, duty cycle, shipped engine switches, directories to evict |
| Launch | `crates/shell/src/kit/mod.rs` `spawn_app` -> `budget_launch` | wraps the argv when `budget::of_exec` knows the program, Memory Saver is on and the app is not excluded |
| Engine switches | `crates/core/src/webruntime.rs` `THIRD_PARTY`, `third_party_flags()` | one-word command line switches for third-party Electron apps (they read no flag file of ours); the six tried are `Candidate` rows and none ships: no gain in the benchmark |
| Limits | `crates/api/src/memsaver/engine.rs` `run_budget` | wold writes `memory.high` of the app's scope: none for `start_ms` after the window first shows, the front value in use, the background value from the saver's delay on |
| Freeze cycle (off by default) | `budget::Cycle`, `engine.rs` `run_budget`, `due()` | a row with `duty: Some(..)` only; no row has it (`ELECTRON.duty` is `None`: with 4 s / 1 s a message in the first minute behind waited 3.6 to 56 s and no saving was measured). In the background past the saver's delay, `cgroup.freeze` is on for `stop_ms` and off for `run_ms`, repeated; the ticker wakes for the next step (`memsaver/mod.rs` `next_wait`) so the app reads its socket at least every `stop_ms + run_ms` |
| Not adopted again | `engine.rs` `budgeted` | a window whose process already sits in `app-<name>-<digits>.scope` is not moved into a Memory Saver scope |
| Only when born there | `engine.rs` `discover` (`is_unit_of`) | `App.budget` is set only for a scope named `app-<engine name>-<digits>.scope`; an app adopted into `app-wol-<id>-<pid>.scope` gets no `memory.high` and no freeze (test `an_adopted_discord_gets_no_limit_and_no_freeze`) |

Never limited, and never frozen by the cycle when a row has one: the saver off, the app excluded, a game, an app playing sound
(a call keeps the limit but is never frozen), the app in use.

## Adding an app

One row in `budget::TABLE` (`..ELECTRON` for the Discord defaults), the program's file name as `exec`, the
name its engine gives its scope as `engine_name`, and the directories under `$HOME` with its own cached
files as `evict`. Then measure it with `tools/bench/discord/` and put the table in `docs/bench/`; a row
whose `record` is not under `docs/bench/` is refused by `budget::tests`.

## Limits

- The promise is PSS of the scope while the app runs. Pages the limit pushes out live in zram (up to
  `swap_max_mib`, compressed; the table in the bench record counts them), so the machine's saving is the
  PSS drop minus their compressed size.
- The cold start is not limited (`start_ms`); under the limit it did not finish in the benchmark.
- Apps started in any other way (a terminal, a script, another launcher) are not budgeted; they are
  adopted by Memory Saver as before (Discord: 543 MiB focused, 443 after ten minutes behind). `budget::launch_sh` is the one function a future `wol` verb or
  desktop-entry override calls.
