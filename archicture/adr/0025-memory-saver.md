# 0025 - Memory Saver
Status: accepted

Context: Electron and Chromium apps keep hundreds of megabytes while hidden. The goal is the App Nap and memory compressor behaviour: a background app gives its memory back, and a focused app is usable at once. The idea of doing this per app came from memory-sentry (yanlvl99). No code or numbers come from it.

Decision:
- wold moves each windowed app, whole process tree, into its own systemd scope, because Chromium leaves its zygote and renderers in the login session scope.
- 30 s after an app stops being in use, wold gives its memory back once, to zram, on a nice 19 worker. `memory.reclaim` covers pages charged to the scope, and `process_madvise(MADV_PAGEOUT)` covers pages charged before adoption, because charges do not move with a process. Focus cancels the worker. Pages return by normal faults.
- Never touched: the compositor, the shell, audio, the frontmost app, apps playing sound or video, games, other users' processes, per-app exclusions.
- Dirty pages are compressed or swapped, never discarded. An app resumes unchanged. Runtime off switch: the Settings row, on by default.
- App Nap (freeze after 10 minutes hidden and idle) is optional and off by default.

Measured on Code, Chromium with 20 tabs, Discord and Signal, on zram with zstd, zswap off, THP madvise, medians of three cycles:

| | Off | Just after saving |
| --- | --- | --- |
| PSS of the four apps | 3618 MiB | 1450 MiB |
| MemAvailable | 10452 MiB | 11993 MiB |
| Focus to first frame, Code | 88 ms | 197 ms |
| Chromium | 16 ms | 45 ms |
| Discord | 33 ms | 121 ms |
| Signal | 22 ms | 28 ms |

- Session to session the saved PSS varied by about 200 MiB.
- 2433 MiB of pages fit in 720 MiB of zram. Audio apps had 0 swap in every run.
- Background tabs keep running, so Chromium faults back: after 4 minutes the four apps hold about 2400 MiB of PSS, and MemAvailable sits about 750 MiB above Off. That is the honest steady number.

Rejected, because they did not win beyond noise:
- Per-role `memory.high` ceilings: they never bound on real apps.
- Prefetch on focus, whole app or main and GPU processes: same first frame, and it left hundreds of megabytes in swap cache and reduced the saving.
- Prefetch of the recorded restore working set: Code stayed at about 200 ms even with the set brought in before the switch.
- Asking the JS engine to collect garbage first: 3 MiB of MemAvailable. It also needs a debug port.
- KSM across apps: 183 MiB worse with saving, because merged pages cannot be paged out, and it has a side-channel cost.
- `vm.page-cluster=3`: no change in first frame.
- A kernel patch: stacks are under 1 MiB after saving and hot anonymous memory is what remains, so ordering V8 regions has nothing to act on, and one pass already works from userspace.

Consequences:
- A frozen app stops answering the compositor's ping, so Hyprland shows its not-responding dialog over it. App Nap stays off until that is solved. Chromium with busy tabs never counts as idle and does not nap at all.
- Memory stays compressed in zram, and the first focus after saving costs up to about 110 ms.
- The numbers come from a VM with software rendering. Restore on real hardware has not been measured.
