# 0027 - One app cannot take the desktop with it
Status: accepted

Context: a fork bomb, a memory bomb or a full file table from a normal user must not freeze the desktop. Before this, app scopes had only systemd's default task limit, no memory limit, and the system-wide open file count was lower than one process's hard limit.

Decision:
- Defaults, no kernel patch: measurement did not ask for one.
- Every app window's scope (Memory Saver's `adopt_args`) gets TasksMax=8192, 80% of RAM and 25% of swap. `user@.service` shares one such set. A user's slice holds 16384 tasks.
- The session scope gets CPUWeight=1000 and 256 MB of memory.min, passed down through its slices. `system.slice` keeps 128 MB, so login and Force Quit work under pressure.
- Fork storms die on their own: wold reads `pids.events` each tick and kills a scope refused at least 2000 forks a tick, two ticks running (`memsaver/storm.rs`). A program that backs off is never touched.
- An app with 4 OOM kills within 5 ticks is killed too: one lost tab is not a loop. When the user's slice refuses forks like a storm at 95% of its tasks, with no app at its own limit, the app with the most tasks goes (`memsaver/storm.rs`, `engine.rs`). Unit tested, not yet run in a guest.
- `fs.file-max` goes back to systemd's value. Open files are charged to the app's memory, so the limit is memory, not a count everyone shares.
- `/tmp` and `/dev/shm` hold 25% of RAM each. Their pages outlive the app that wrote them.

Measured in a throwaway guest (8 vCPU, 8 GB, WolKernel, btrfs, zram; `wolos-iso/tests/test/limits`). Stand-in compositor at 120 Hz in the session scope, the bomb in a window scope, 20 s then killed:

| Attack | Before | After |
|---|---|---|
| Fork bomb | runs until killed by hand | dead in 7.1 s, no failed spawns, 5 ms worst input latency |
| Thread bomb | runs until killed by hand | dead in 6.0 s |
| Memory bomb | 95 s outage, spawn median 3.7 s, root login up to 13 s | spawn median 3.5 ms, root login 0.9 s at worst |
| Pipe fill | guest wedged in 1 of 2 runs | 18 of 18 spawns, root 16 of 16 |
| File table | 19 of 20 spawns fail, root 1 of 3 | no failures, root 18 of 18 |
| PID, inotify, tmpfs, disk fill | no desktop harm | no desktop harm |

Recovery after the kill took 1.0 s or less in every run after the change. `make -j16` beside a 4000-task browser-like tree: 4.91 s before, 5.00 s after, one run each, nothing killed.

Limits: fork() from unrelated scopes returned EAGAIN with 16,000 tasks in one scope on the 8 GB guest, cause not found; 8192 stays under it. The table was measured before the OOM loop and full-slice kills existed. A full disk or a full tmpfs stays full until something deletes the files. inotify limits stay per user.

Consequences: an app that needs more than 8192 tasks or 80% of RAM cannot get it on WolOS; the limits are systemd properties, so a future change is a drop-in and no code. `fs.file-max` and the tmpfs sizes differ from Arch's defaults (the drop-ins under `files/` are the record). The kill rules in `memsaver/storm.rs` have unit tests only until they run in a guest (see Limits).

## Amendment 2026-11-04: per-app memory budgets

Context: the lead asked for Discord in 100 MB. Memory Saver (ADR 0026) adopts a running app into a scope and gives its pages back once; after ten minutes behind, Discord held 443 MiB of PSS again, 543 in front. A limit on an adopted scope pins `memory.current` and leaves PSS at about 390, because pages charged before the adoption stay in the session scope, which has `memory.min` 256 MiB.

Decision:
- An app can carry a budget (`wol_core::memsaver::budget::TABLE`): a PSS promise, `memory.high` in front and behind, a swap cap, an optional freeze cycle (off in every row today), directories to drop from the page cache. Discord is the first row (100 MiB; `memory.high` 150 MiB in front, 100 MiB behind, `memory.swap.max` 2048 MiB). The default row (`ELECTRON`) is what a third-party Electron app with its own engine gets when it is added.
- A budgeted app is started in its scope, not adopted: the shell's `spawn_app` drops the cached pages of the app's own files (`posix_fadvise DONTNEED`), then runs it under `systemd-run --user --scope --unit=app-<engine name>-$$.scope`. That is the name Chromium would move itself to, so it stays. The same function (`budget::launch_sh`) is what any other launcher calls. Only when Memory Saver is on and the app is not excluded.
- wold governs the scope after the start: no limit for 60 s after the window first shows (a cold start under a limit never finished), then the front value; the background value once the saver's delay has passed; the freeze cycle (`cgroup.freeze` on for `stop_ms`, off for `run_ms`) is in the code and **off by default**: with 4 s / 1 s a message in the first minute behind waited 3.6 to 56 s and no memory saving was measured from it (a freeze releases nothing; no run compared with and without). Games, excluded apps, the saver off: no limit. An app playing sound keeps its limit and is never frozen when a row turns the cycle on. A window whose process already sits in `app-<name>-<digits>.scope` is not adopted again. Only an app born in that scope gets the budget: Discord adopted into `app-wol-discord-<pid>.scope` (started any other way) is a plain Memory Saver app, with no `memory.high` and no freeze, because a limit on an adopted scope pins `memory.current` and leaves PSS where it was.
- Fixed values, not a governor that follows the measured PSS: the one tried let `memory.current` float to 117 to 198 MiB against a target of 100.
- Engine switches for third-party apps (`webruntime::THIRD_PARTY`): tried, not shipped. Same limits, same PSS with and without them, and two weaken the engine.

Measured (docs/bench/memsaver-discord.md): Discord logged out, focused idle 543 to median 93 MiB of PSS, typing 551 to 80 to 91, one minute behind 294 to 46 to 62, five to ten minutes behind 443 to a median of 57 to 60 (82 at 600 s).

Limits, in numbers: peaks above 100 (idle up to 105, typing up to 119, one run 151); the first 60 s are unbounded; a 100 MiB limit in front stalls the app 24 to 46 % of the time, so the front limit is 150 and the 100 is a median; a message may wait more than 5 s in the first minute behind (3.6 to 56 s measured with the freeze cycle on while the limit dropped from 150 to 100; the cycle ships off, a run without it was not made); what the limit pushes out sits in zram (270 to 350 MiB uncompressed). Not run: a logged-in Discord, voice, the wold engine in a guest.

Consequences: a budgeted app costs a cold start in its own scope and zram for what it gave up; one Discord window cannot hold more than 150 MiB of PSS in front. An app started outside the shell keeps today's behaviour (adopted: 543 MiB focused, 443 after ten minutes behind). Adding an app is a row in the table and a record under docs/bench/.
