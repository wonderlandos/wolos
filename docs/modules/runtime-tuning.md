# Runtime tuning: kernel and userland memory for JavaScript engines (ADR 0061)

Third-party apps bring their own engine (Electron and Chromium: V8; WebKitGTK and Bun: JavaScriptCore).
Their memory is mostly not the script heap: it is allocator arenas, a very large binary mapped by every
process, and pages the kernel maps ahead of use. This module is what WolOS gives those apps below the
app. Numbers: [bench/js-runtime-x.md](../bench/js-runtime-x.md); decision: [0061](../adr/0061-runtime-tuning-js-engines.md).

## Pieces

| Piece | Where | What |
|---|---|---|
| Tables and pure logic | `crates/core/src/jsruntime.rs` | `ROWS` (every engine knob measured, shipped or not, with its MiB), `runtime_flags(class, mode)` -> `Runtime {args, env, ksm_merge}`, `Runtime::prefix()`, `MM_WRITES`, `ksm_policy(passwd)`, `pageout_ranges(maps, what)` (the one range filter), `pageout_allowed(cgroup, uid, caller, any)` |
| Boot | `files/usr/lib/systemd/system/wolos-mm.service` | oneshot `wolos-mm apply` after `sys-kernel-debug.mount`; enabled by the preset, runs on the live medium too |
| Kernel calls | `crates/controller/src/bin/wolos-mm.rs` (`/usr/bin/wolos-mm`) | `apply`, `ksm` (status dump, printed once; no loop), `exec-merge -- /abs/path CMD...`, `pageout [--any] PID [anon|all]`; the only `unsafe` of this module (2 blocks) |
| Memory Saver's page-out | `crates/api/src/memsaver/pageout.rs`, `engine.rs` | calls `jsruntime::pageout_ranges(maps, Pageout::Anon)` for the one give-back and `Pageout::All` every 30 s (`PAGEOUT_EVERY_MS`) for a budgeted app behind; one range per `process_madvise` call, the pidfd asked before each; decides when |

## KSM only with one human account

KSM merges identical pages of every process that opted in (`exec-merge`), across processes and across users,
and a write to a merged page is slower than to a private one. A process of one person can learn from that
whether another person's process holds a page with a given content: a cross-process deduplication timing
side channel. So `wolos-mm apply` writes `/sys/kernel/mm/ksm/run=1` only when `/etc/passwd` has exactly one
human account (uid 1000 and up with a login shell, as Settings > Accounts counts them) and writes `run=0`
otherwise, with a journal line (`KSM off: 2 human accounts ...`); `exec-merge` applies the same rule and asks
the kernel for nothing on such a machine. Read the line with `journalctl -u wolos-mm`. The other knobs
(fault-around, the scan rate) do not depend on accounts.

## Page-out: one filter, who may be paged out

`jsruntime::pageout_ranges` is the only place that decides which mappings go: by default writable anonymous
memory (heap, stacks, `[anon:...]`), memfd and /dev/shm memory, and writable private file data; `all` adds the
read-only mappings and the code. `wolos-mm pageout` and Memory Saver both call it; its test table is the only
one. `wolos-mm pageout` opens a pidfd first, reads the cgroup, the owner and the maps only after, and checks
the pidfd still names the process; it refuses (exit 3) a target that is not the caller's own user's or not in
an `app-*.scope`; `--any` lifts that, for root only.

## The table

Discord 1.0.168 (Electron 44.10, Chromium 152) on the login page, kernel 7.3.1, 4 vCPU 8 GiB VM, PSS of the
whole app, cold start after dropping the page cache; baseline 534 MiB. Run-to-run spread about 5 MiB (anon
up to 15 between boots).

| Layer | Knob | Result | Speed cost | Status |
|---|---|---|---|---|
| kernel | `fault_around_bytes` 65536 -> 4096 | file PSS 215 -> 126 MiB (8192: 144, 16384: 164) | +8 % CPU and +45 % minor faults in the start; window and frame time unchanged | shipped (`wolos-mm apply`) |
| kernel | KSM `run=1` (one human account only), 500 pages / 100 ms, app opted in with `exec-merge` | Electron anon -22..-26 MiB (8 of 8 paired runs), WebKitGTK -118 MiB; falls to 2.5 % of a core | none seen (frames p50 26 -> 27 ms) | shipped |
| kernel | KSM 2000 pages / 20 ms | same merges, 26 % of a core | | not shipped |
| kernel | KSM scan-time advisor, 30 s | 35 % of a core, unstable | | not shipped |
| kernel | KSM `use_zero_pages=1` | fewer merges (-12) | | not shipped |
| kernel | THP `always` | 0 | | not shipped (kept `madvise`) |
| kernel | `memory.reclaim swappiness=0` on the scope | -34 MiB of 215 (pages charged to other cgroups) | | not used |
| kernel | `process_madvise(MADV_PAGEOUT)` anon + file, `wolos-mm pageout` | PSS 424 -> 112 right after; 130 after a minute; swap-PSS 214-250 MiB (zswap pool 69 MiB) | first script call after: 2-29 ms | tool shipped; caller is Memory Saver |
| Electron | `--js-flags=--lite-mode` | -13 MiB anon | jitless: scripts several times slower, no WebAssembly | `Mode::Lean` row only |
| Electron | `--optimize-for-size --max-semi-space-size=1` | +5 (none) | | no gain |
| Electron | `--single-threaded-gc --max-lazy` | 0 | | no gain |
| Electron | `--in-process-gpu` + `NetworkServiceInProcess2` | 0 (one process fewer, same sum) | | no gain |
| Electron | `--renderer-process-limit=1 --disable-site-isolation-trials --num-raster-threads=1` | +10..+60 (shmem +40 in front) | | no gain |
| Electron | `MALLOC_ARENA_MAX=1`, `MALLOC_TRIM_THRESHOLD_` | 0 (PartitionAlloc holds the heap; glibc 8 MiB) | | no gain |
| WebKitGTK | `JSC_useJIT=false` | 762 -> 670 MiB | scripts several times slower | `Mode::Lean` row only |
| WebKitGTK | `JSC_useConcurrentGC=false` + one marker/compiler thread | +4 (none) | | no gain |
| Bun | `MIMALLOC_ARENA_EAGER_COMMIT=0` | peak RSS 859 -> 368 MiB | none (0.91 -> 0.82 s) | shipped (`bun-arena-lazy-commit`) |
| Bun | `MIMALLOC_PURGE_DELAY=0` (on top) | 368 -> 321 MiB; alone 888 -> 705 | none measurable | shipped (`bun-purge-now`) |
| Bun | `--smol` | 888 -> 869 (2 %) | | no gain |
| Bun | `BUN_JSC_useJIT=0` | 888 -> 806 MiB | +25 % wall | `Mode::Lean` row only |
| Bun | `BUN_JSC_useConcurrentGC=0`, `numberOfGCMarkers=1`, `BUN_GARBAGE_COLLECTOR_LEVEL=2` | -2..-4 % | | no gain |

Option names checked in the engine sources: V8 `src/flags/flag-definitions.h` (`lite_mode`, `jitless`,
`optimize_for_size` implying `max_semi_space_size=1`, `single_threaded_gc`, `max_lazy`, `memory_reducer`,
`flush_bytecode`), JavaScriptCore `runtime/OptionsList.h` (`useJIT`, `useConcurrentGC`,
`numberOfGCMarkers`, `gcMaxHeapSize`; set by `JSC_<name>`, in Bun by `BUN_JSC_<name>`), mimalloc
(`MIMALLOC_ARENA_EAGER_COMMIT`, `MIMALLOC_PURGE_DELAY`).

## What a launcher does

```rust
let rt = wol_core::jsruntime::runtime_flags(Class::Electron, Mode::Default);
// argv = rt.prefix() + [systemd-run ... app] + rt.args ; environment: rt.env
```
`Runtime::prefix()` is `/usr/bin/wolos-mm exec-merge --` when KSM is asked: it goes in front of the whole
command (before `systemd-run`), because the flag is inherited through every exec and fork. `exec-merge`
refuses a bare command name, so the launcher puts absolute paths after it (`/usr/bin/systemd-run`, the app's
path). The budget path
(`memsaver::budget`, MEMSAVER-X) owns the call.

## PSS or memory

PSS is what a task manager charges an app; it counts a mapped page of a shared file in every process that
maps it. `fault_around_bytes` changes that charge, not the page cache: `MemAvailable` did not move with it
(6.56 GiB either way). The page-out and KSM free real memory: after the page-out `MemAvailable` rose by
170 MiB for 215 MiB paged (the zswap pool holds the rest, 69 MiB).

## Kernel lane

- `fault_around_bytes` exists only in debugfs; under `lockdown=integrity` (the kernel here has
  `LOCK_DOWN_KERNEL_FORCE_NONE`, so today it is writable) the boot write fails. Proposal: a sysctl
  `vm.fault_around_bytes` in `mm/memory.c` (a `ctl_table` entry next to `vm.max_map_count` that reads and
  writes `fault_around_bytes`, clamped to a power of two between PAGE_SIZE and 64 KiB). A per-process
  setting (`prctl`) would let Electron keep 64 KiB for its code and a game its own value; not measured.
- Zswap is on (`CONFIG_ZSWAP_DEFAULT_ON`) in front of the zram swap, and both compress: a page paged out goes
  to the zswap pool first and reaches zram only when the pool is written back. Measured 3.2:1 in the pool.
  Not changed here (no measured harm); the kernel lane decides whether zswap is wanted at all with zram.
- `CONFIG_KSM`, `CONFIG_LRU_GEN`, `CONFIG_TRANSPARENT_HUGEPAGE` (madvise) and `CONFIG_DAMON_RECLAIM` are on.
  DAMON-based reclaim of cold pages per process would be the in-kernel form of the page-out; not tried.

## Not verified

- A logged-in Discord (no account in the VM): the login page holds less script heap than a client with
  servers loaded. The page-out numbers are for the page as it stands; the flags may matter more there.
- Real hardware, a GPU, voice and screen share. The VM draws in software; `libLLVM` (llvmpipe) is 15 MiB of
  the GPU process there and would not be on a GPU.
- Battery cost of ksmd on a laptop (10 wake-ups a second while an opted-in app runs).
