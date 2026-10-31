# 0026 - One app cannot take the desktop with it
Status: accepted

Context: a fork bomb, a memory bomb or a full file table from a normal user must not freeze the desktop. Before this, app scopes had only systemd's default task limit, no memory limit, and the system-wide open file count was lower than one process's hard limit.

Decision:
- Defaults, no kernel patch: measurement did not ask for one.
- Every app window's scope (Memory Saver's `adopt_args`) gets TasksMax=8192, 80% of RAM and 25% of swap. `user@.service` shares one such set. A user's slice holds 16384 tasks.
- The session scope gets CPUWeight=1000 and 256 MB of memory.min, passed down through its slices. `system.slice` keeps 128 MB, so login and Force Quit work under pressure.
- Fork storms die on their own: wold reads `pids.events` each tick and kills a scope refused at least 2000 forks a tick, two ticks running (`memsaver/storm.rs`). A program that backs off is never touched.
- `fs.file-max` goes back to systemd's value. Open files are charged to the app's memory, so the limit is memory, not a count everyone shares.
- `/tmp` and `/dev/shm` hold 25% of RAM each. Their pages outlive the app that wrote them.

Measured in a throwaway guest (8 vCPU, 8 GB, WolKernel, btrfs, zram; `image/vm/test/limits`). Stand-in compositor at 120 Hz in the session scope, the bomb in a window scope, 20 s then killed:

| Attack | Before | After |
|---|---|---|
| Fork bomb | runs until killed by hand | dead in 7.1 s, no failed spawns, 5 ms worst input latency |
| Thread bomb | runs until killed by hand | dead in 6.0 s |
| Memory bomb | 95 s outage, spawn median 3.7 s, root login up to 13 s | spawn median 3.5 ms, root login 0.9 s at worst |
| Pipe fill | guest wedged in 1 of 2 runs | 18 of 18 spawns, root 16 of 16 |
| File table | 19 of 20 spawns fail, root 1 of 3 | no failures, root 18 of 18 |
| PID, inotify, tmpfs, disk fill | no desktop harm | no desktop harm |

Recovery after the kill was under 1 s in every run after the change. `make -j16` beside a 4000-task browser-like tree: 4.91 s before, 5.00 s after, one run each, nothing killed.

Limits: fork() from unrelated scopes returned EAGAIN with 16,000 tasks in one scope on the 8 GB guest, cause not found; 8192 stays under it. Two windows together can still reach the slice limit. A full disk or a full tmpfs stays full until something deletes the files. inotify limits stay per user.
