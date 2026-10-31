# WolKernel series (linux-wolos)

Shape (like Android's common kernel): stable core + small WolOS series + fixed userspace interface + KMI.

| layer | where |
|---|---|
| stable core | github.com/wonderlandos/linux at the ref pinned by `kernel/linux-wolos/PKGBUILD` (public) |
| config | `kernel/conf/wolos.config` (public): latency, sched_ext, MODVERSIONS, datacenter-only drivers off |
| perf series | `linux/patches/*.patch`, applied in name order by the PKGBUILD |
| scheduler | `scx_wolos/` (sched_ext BPF + loader + unit); interface: `scx_wolos/INTERFACE.md` |
| KMI | `kmi/` symbol freeze per release line |

## Series (base: wonderlandos/linux branch wolos, 3f88c73)

The fork carries the former 0001-0003 (bounded futex spin, `/sys/kernel/wolos` hints, i915 deadline boost) and the public legacy fixes (lazy audit syscall work, int $0x80 from 64-bit, retbleed stuffing, kernel defaults; KUnit `wolos_compat`).

| patch | area | what | upstream status |
|---|---|---|---|
| 0004-futex-adaptive-spin | sync | 0001's spin only when the CPU has nothing else runnable, budget scaled by a per-bucket score (wakeup +1, timeout /2); KUnit `wolos_futex_spin` | none upstream |
| 0005-wolos-policy-kunit | test | `<linux/wolos_policy.h>`: the decisions below as pure functions; KUnit `wolos_policy` (UML) | WolOS-only |
| 0006-tsc-trust-single-package | time | no clocksource watchdog on one-package bare metal with an invariant TSC, even without TSC_ADJUST (AMD desktops); `tsc=watchdog` restores it | Feng Tang's qualified-platform idea, not merged |
| 0007-vt-kbd-graphics-bypass | input | `kbd_event` returns early in K_OFF + KD_GRAPHICS (compositor session): no keymap, LED tasklet or console work per key; key_down kept | none upstream |
| 0008-oom-foreground-bias | memory | OOM badness: foreground tgid /4, SCHED_IDLE + RAM/8; oom_score_adj on top | none upstream |
| 0009-input-boost | input/cpufreq | input handler on key/button devices opens a `wolos_input_boost.ms` (100 ms) window, rate-limited 50 ms; schedutil floors CPUs at `floor_pct` (60%); `wolos_input_boost_chain` for GPU drivers; KUnit `wolos_policy` input_boost | Android cpu-boost idea, none upstream |
| 0010-slub-sheaf-scale | memory | sheaf capacity x `slub_sheaf_scale` (1..4, default 2; 1 = stock); hackbench socket +4% vs scale 1 on one binary, not shown kept vs base; KUnit `wolos_slub_sheaf` | none upstream |
| 0011-futex-spin-rcu | sync | 0004's spin holds RCU across the bucket access (a rehash frees the old hash by RCU) | fix to 0004 |
| 0012-futex-waitv-inline | sync | `futex_waitv` on up to 4 futexes uses an on-stack vector, no kzalloc per call; `futex.waitv_inline=0`; KUnit `wolos_futex_spin` | none upstream |
| 0013-futex-precheck | sync | FUTEX_WAIT and `futex_waitv` read the word first: a mismatch returns before hashing; `futex.precheck=0`; KUnit `wolos_futex_spin` | none upstream |
| 0030-wolos-bend | lib | `lib/wolos_bend`: CAS link/subst, Chase-Lev deques with batched stealing, worker arenas, generation-checked index pools, fork/join on idle CPUs; KUnit `wolos_bend` (UML, 4 CPUs) | WolOS-only |
| 0031-clear-large-folios-parallel | memory | folios of 4 MiB and up clear on idle CPUs: 1 GiB first touch 91.7 to 24.1 ms idle, no gain with every CPU busy, 2 MiB THP unchanged; smaller folios pay one compare | none upstream |
| 0032-syscall-exit-status-store | entry | no write to `thread_info.status` on 64-bit syscall exit unless a bit is set: `perf c2c` shows no more HITMs on the first `task_struct` line (976 of 14741 in `perf bench epoll wait`); throughput unchanged within noise | candidate |
| 0033-enosys-under-ptrace | entry | out-of-range syscalls return -ENOSYS after a tracer wrote `ax`; `syscall_numbering_64` passes without x32 | candidate |
| 0034-wolos-bend-reclaim | lib | `bend_fork_join` is safe from reclaim: rescuer on the queue, `GFP_NOWAIT` per-run state; KUnit under lockdep | WolOS-only |

Not ported, Linux already has it: `futex_waitv` (5.16), the sysret fast return (x86 entry always tries SYSRET), ntsync (6.14), memory-pressure freezing (PSI triggers + cgroup v2 `cgroup.freeze`: wold uses these, no patch).
Not ported, not needed: i915 VBT file fallback (Linux has `i915.vbt_firmware=`; VMs use virtio-gpu).
Next, in value order: xe deadline/waitboost (needs GuC SLPC min-freq raise, ~60 lines); amdgpu foreground boost (power profile switch).
Generic fixes that are not product-specific go to the fork as commits on a `wolos` branch, not here. Today: none.

Rules: one concern per patch, < 100 lines, each with a header comment and a runtime off switch (module param or sysfs). New userspace-visible names live only in 0002.

## Rebase on a stable bump

1. Bump the ref in the PKGBUILD; `git am linux/patches/*.patch` on the new ref in a scratch clone.
2. Conflict: fix, `git format-patch` back into `linux/patches/` (same names). A patch upstream now has: delete it, note it above.
3. `make olddefconfig` with `wolos.config`; diff `scripts/diffconfig` against the last release: every lost line is a decision.
4. Build, `kmi/kmi.sh check O kmi/wolkernel-<line>.symvers` (same line) or `extract` (new line), build `scx_wolos`, boot test.

## WolKernel userspace ABI rules

- Anything that runs on stock Arch runs here: 32-bit, Wine (modify_ldt, binfmt_misc), Flatpak/AppImage/Snap, Docker/Podman, KVM/VirtualBox, Chromium sandbox, DKMS (nvidia, zfs, v4l2loopback).
- Removed (public list: `kernel/abi-removed.txt`):
  - x32 ABI: glibc/musl ship x32 only as a separate target; Arch, Flatpak runtimes, Docker images and Steam have no x32 binaries.
  - Everything else Arch already disables (uselib, sysfs(2), legacy PTYs, TIOCSTI, uevent helper, COMPAT_BRK). vsyscall emulation and ioperm stay: old static binaries and Wine/DOS tools still use them.
- Added: only `<linux/wolos.h>` + `/sys/kernel/wolos`. `abi_version` increments on any change; fields are never repurposed.
- Mitigations: Arch defaults (auto per CPU). No remote/browser-facing mitigation is disabled; tuning happens per CPU generation via the default `auto` selection only.
- The UAPI headers package (`linux-wolos-api-headers`) is built from the same tree with these patches, so userspace sees exactly this ABI.

## KMI policy

- A release line = one WolKernel major (e.g. 7.3.y). Its first release freezes vmlinux exports + `kmi/modules.list` (`kmi.sh extract`, CRCs from CONFIG_MODVERSIONS).
- Within a line, a build that removes or changes a frozen symbol fails (`kmi.sh check`). Adding symbols is fine.
- New line = new baseline; DKMS rebuilds once per line (as on any kernel bump).

## Licensing note

Kernel patches and `scx_wolos.bpf.c` are GPL-2.0 (Linux and the sched_ext kfuncs require it). Shipping binaries built from them obliges offering their source to recipients.

FreeBSD-era patches and checks are kept for reference in `freebsd/`; nothing builds them.
