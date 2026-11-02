# scx_wolos interface (for crates/api/src/qos.rs)

No new syscall. A class is applied per thread (iterate `/proc/<pid>/task/*`) with `sched_setattr(2)`:

| class (raw) | policy | nice | uclamp_min | uclamp_max |
|---|---|---|---|---|
| 0 UserInteractive | SCHED_OTHER | -10 | 768 | 1024 |
| 1 UserInitiated (frontmost app) | SCHED_OTHER | -5 | 384 | 1024 |
| 2 Default | SCHED_OTHER | 0 | 0 | 1024 |
| 3 Utility | SCHED_OTHER | 10 | 0 | 512 |
| 4 Background | SCHED_IDLE | 19 | 0 | 256 |

`sched_flags = SCHED_FLAG_UTIL_CLAMP_MIN | SCHED_FLAG_UTIL_CLAMP_MAX` (+ `SCHED_FLAG_RESET_ON_FORK` not set: new threads inherit).
Constants: `<linux/wolos.h>` (`WOLOS_QOS_*`, `WOLOS_UCLAMP_*`). Needs CAP_SYS_NICE for classes 0-1 (wold is root).
scx_wolos classifies from these fields; with it unloaded EEVDF uses the same nice/uclamp, so the API never changes.
Classes 0-1 also need a grant: the loader pins a map at `/sys/fs/bpf/wolos/boosted` (root, 0600), key tgid -> highest class (0 or 1); key 0 present = grants in use. With grants in use an ungranted task is capped at default; with no grants uclamp is capped at foreground; a high-class task averaging 16 ms bursts drops to default (audit A19).

Frontmost app for GPU boost: write its tgid to `/sys/kernel/wolos/foreground_pid` (0 = none). Replaces the FreeBSD `compat.linuxkpi.i915_wol_fg_pid` sysctl.
Detect: `/sys/kernel/wolos/abi_version` = 1; scheduler active: `/sys/kernel/sched_ext/root/ops` = `wolos`.
