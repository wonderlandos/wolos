# Runtime tuning for JavaScript engines: measured (ADR 0061)

Concept: where the memory of an Electron, WebKitGTK and Bun app goes, and what each kernel and userland knob
did to it, with its speed cost. Why: the lead's order (2026-11-04) was Discord at 100 MB without a new
Discord, Bun, WebKit and V8 faster with less GC and memory, at kernel and userland level. How:
`tools/bench/js-runtime-x/` (`jsx.py` in the guest, `bun-run.sh` on the host), every run from a cold start
after `drop_caches`, one knob changed against the same VM image.

## Identity

| | |
|---|---|
| Run | 2026-11-04 22:10 to 2026-11-05 00:50 UTC, four VMs in parallel (`jsrx`..`jsrx4`, reflink copies of the stopped 0.12.1 install `msx4` of MEMSAVER-X, same base as the MEMSAVER-X Discord record) |
| Guest | wolvm on the forge, 4 vCPU Xeon Platinum 8375C, 8 GiB, KVM, `virtio-vga` (software rendering: llvmpipe), kernel `7.3.1-1-wolos`, `hyprland-wolos 0.56.2-6`, `mesa 26.2.6`, zram 3.9 GiB zstd with zswap on, THP `madvise`, MGLRU `min_ttl_ms=1000`, Memory Saver off |
| Discord | `discord 1:1.0.168-1` (the updater's own copy in `~/.config/discord/app-1.0.168`, 218 MB), Electron 44.10.0, Chromium 152.0.7977.169, the **login page** (no account), 8 processes |
| WebKitGTK | `webkitgtk-6.0 2.54.2`, `MiniBrowser https://discord.com/login`, 4 processes (one is the pre-warmed spare web process) |
| Bun | 1.4.4 (`bun-linux-x64`), `bun --bun x vite build` of `site/` to `/tmp`, host kernel `7.0.0-1014-aws` (THP `madvise`), median of 5 |
| Metric | PSS summed over every process of the app (`smaps_rollup`), split into `Pss_Anon`, `Pss_File`, `Pss_Shmem`; SwapPss separately; `MemAvailable`; script call and frame time through DevTools (`cdp.py`, port 9222) |
| Spread | same configuration, repeated: 534-542 MiB baseline; anon 293-313 between boots; paired runs below |

## Where the memory goes (baseline, 534 MiB focused, 8 processes)

| By mapping (PSS) | MiB | By process | MiB |
|---|---|---|---|
| PartitionAlloc (`[anon:partition_alloc]`) | 142 | renderer | 273 |
| `Discord` binary, executable pages | 103 | main (browser) | 128 |
| `Discord` binary, read-only data and relro | 76 (24 of it private dirty relocations) | GPU process | 50 |
| V8 heap inside the sandbox cage (`v8-sandbox`) | 61 | network + audio utility | 57 |
| V8 code and tables (`v8`, `v8-pointer-table`) | 29 + 4.5 | 3 zygotes | 30 |
| shared memory | 14 | | |
| `discord_voice`, `krisp`, `mediapipe`, `libLLVM` | 31 | | |

File-backed pages are 215 MiB (40 %), anonymous 305, shared memory 14. The binary's code is mapped by all
eight processes; PSS charges each its share of every page a fault mapped, used or not.

## Knobs, one at a time (Discord, front, 60 s after the window)

| Run | Knob | PSS | anon | file | Note |
|---|---|---|---|---|---|
| b0, b0r, s0, f0 | baseline | 534, 537, 542, 541 | 305 | 215 | |
| fa4k, s1 | `fault_around_bytes` 4096 | 446, 448 | 307 | 126 | -88 MiB, all in file |
| fa8k / fa16k | 8192 / 16384 | 465 / 484 | | 144 / 164 | |
| 6 pairs | 65536 against 4096: window 6.7 against 6.4 s median, CPU in 45 s 13.0 against 14.0 s, minor faults 286k against 413k, frame p50 26 against 26 ms | | | | cost is small |
| k1, 8 pairs (K1-K4) | KSM on, same VM and start | 504-518 | **275-280** | | against 299-305 off: **-22..-26 MiB in 8 of 8 pairs** |
| k1L | KSM 10 minutes | 505 | 268 | | still falling slowly |
| u2 | `--js-flags=--lite-mode` | 521 | 296 | 211 | -13 MiB, jitless |
| u1 | `--max-semi-space-size=1 --optimize-for-size` | 541 | 309 | 219 | none |
| u7 | `--single-threaded-gc --max-lazy` | 536 | 308 | 215 | none |
| u3 | `--in-process-gpu` + `NetworkServiceInProcess2` | 537 | 308 | 216 | 7 processes, same sum |
| u5 | `--renderer-process-limit=1 --disable-site-isolation-trials --num-raster-threads=1` | 594 | 313 | 219 | shmem +48 MiB while in front |
| u6 | `MALLOC_ARENA_MAX=1 MALLOC_TRIM_THRESHOLD_=65536 MALLOC_TOP_PAD_=0` | 537 | 307 | 216 | none |
| k2 | THP `always` | 538 | 309 | 215 | none |
| rcl0 | `memory.reclaim 700M swappiness=0` on the scope | 535 -> 500 -> 493 | | 215 -> 181 | pages charged to other cgroups |
| g1 | KSM scan-time advisor, 30 s target | 441 | 298 | 129 | ksmd +111 s in 320 s (35 % of a core) |
| g2 | KSM 500 pages / 100 ms | 446 -> 423 | 281 | 129 | ksmd +7.1 s in 320 s (2.2 %); same merges as 2000 / 20 ms (26 % of a core) |
| g3 | KSM + `use_zero_pages` | 451 -> 440 | 297 | 129 | fewer merges |

## Page-out and the 100 MiB question (Discord login page)

`process_madvise(MADV_PAGEOUT)` over every private mapping of the app, anon and file, with fault-around 4096
and KSM on. The first measurement was the same call made by `pageout.py` (Python, one call per range); the last three rows are
the shipped `wolos-mm pageout` and `wolos-mm exec-merge`, from a freshly booted VM with the boot knobs applied (first version: a tmpfiles file; now `wolos-mm apply`, same values).

| Run | Configuration | in front before | right after the call | 1 min later | background 1 min | background 10 min | back in front, 20 s |
|---|---|---|---|---|---|---|---|
| f0 | baseline | 541 | | | 535 | 536 | 534 |
| f1 | fault-around + KSM, no page-out | 422 | | | 415 | 415 | 416 |
| s3 | + page-out of anon + file once | 420 | **112** | 132 | 133 | | 135 |
| s4 | + page-out of anon only | 422 | 207 | 228 | 229 | | 230 |
| s5 | baseline + page-out of anon only | 542 | 301 | 319 | 320 | | 323 |
| f2 | + page-out of anon + file every 30 s in the background | 425 | **115** | 136 | **101** | **96** | **115** |
| F0 (shipped files, booted) | baseline | 549 | | | 543 | 533 | 533 |
| F1 | shipped defaults (`exec-merge`, fault-around 4096), no page-out | 470 | | | 464 | 457 | 457 |
| F2 | + `wolos-mm pageout` anon+file every 30 s | 432 | 153 (1 min) | | **104** | **100** | **128** |

Where 100 MiB sits at the end of F2 (background, 10 minutes): anon 23, file 63, shared memory 14. The
swapped part is 245 MiB of SwapPss, in the zswap pool at 3.2 : 1 (about 75 MiB of RAM); `MemAvailable` rose
170 MiB while PSS fell 310 (s3). So **the PSS figure of 96-104 MiB is real as PSS, and the machine holds
about 75 MiB compressed beside it**. A logged-in client holds more.

Speed of the paged app (DevTools `Runtime.evaluate` of a constant, the delay a message sees):

| Run | first call after the page-out | median / p95 / max of 8 | frames in front (10, two animation frames) |
|---|---|---|---|
| f0 baseline | | 2 / 3 / 3 ms | p50 26, p95 32 |
| f1 | | 2 / 2 / 5 ms | p50 32, p95 39 |
| s3 | 15 ms | 1 / 1 / 15 ms | |
| f2 | 29 ms | 6 / 19 / 29 ms; 7 ms after 10 min behind | p50 24, p95 28 |
| F2 | | 2 / 6 / 10 ms | p50 29, p95 31 |
| F1 | | | p50 27, p95 31 |

## WebKitGTK (MiniBrowser on discord.com/login, front, 60 s)

| Run | Configuration | PSS | anon | file |
|---|---|---|---|---|
| w0 | baseline | 762 | 577 | 179 |
| w1 | `JSC_useJIT=false` | 670 | 496 | 168 |
| w2 | one GC marker, no concurrent GC or JIT threads | 766 | 581 | 179 |
| w3 | fault-around 4096 + KSM | 573 | 459 | 108 |
| w4 | + page-out once | 93 right after, 111 after a minute, 146 after 2 | 41 / 56 / 84 | 46 / 49 / 56 |
| W0s | shipped (`exec-merge`, fault-around 4096), booted VM | 612 front, 543 after 5 min | 491 / 424 | 114 / 113 |

Two web processes run: the page and a pre-warmed spare (190 MiB resident) that WebKitGTK starts for the next
navigation; an app can switch it off through the API (`webkit_web_context_set_process_model`, shared
process), which is the app's choice, not an environment variable (not measured).

## Bun (vite build of `site/`, peak RSS, median of 5)

| Configuration | RSS MiB | wall s |
|---|---|---|
| default | 888, 859 | 0.75, 0.91 |
| `--smol` | 869 | 0.77 |
| `BUN_JSC_useConcurrentGC=0` | 857 | 0.77 |
| `BUN_JSC_numberOfGCMarkers=1` | 876 | 0.79 |
| `BUN_GARBAGE_COLLECTOR_LEVEL=2` | 865 | 0.76 |
| `BUN_JSC_useJIT=0` | 806 | 0.94 |
| `MIMALLOC_PURGE_DELAY=10` | 834 | 0.92 |
| `MIMALLOC_PURGE_DELAY=0` | 705 | 0.80 |
| `MIMALLOC_ARENA_EAGER_COMMIT=0` | **368** | 0.82 |
| both | **321** | 0.87 |
| both + `--smol` | 321 | 0.84 |

## Not measured

- A logged-in Discord (no account in the VM), voice and screen share, message send latency (needs an account).
- Real hardware and a GPU (the VM renders in software); laptop battery cost of ksmd and the page-out wake-ups.
- Frames and typing under the page-out in front (the page-out is a background tool; the front figures above are
  the app before the call).
- Bun inside the WolOS VM (measured on the forge host kernel 7.0); WebKitGTK with a real WebKit app.
- `lockdown=integrity` kernels (the debugfs write fails there; see the module page, "Kernel lane").
