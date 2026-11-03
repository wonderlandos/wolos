# Wolkit benchmarks (ADR 0039)

How the UI kit is measured and what has been measured so far. The engine is `crates/wolkit` (`docs/modules/libwolkit.md`), the Crystal shard is `wolos-crystal/shards/wolkit` (`docs/modules/wolkit.md`). Every number below names its machine; numbers are only comparable on the same machine at the same commit.

## Harnesses

| What | Where | How to run | Gate |
|---|---|---|---|
| Engine microbenchmarks: text, batching, style, layout, GPU | `crates/wolkit/benches/{text,batch,style,layout,gpu}.rs` (plain programs, no benchmark crate) | `cargo bench -p wolkit --bench <name> -- --check` | `crates/wolkit/benches/budgets.toml`: a median over its limit fails `--check`. Limits are ceilings with CI noise in mind; lower them by hand after a forge run, never raise one without a recorded reason. |
| Shard side: cold start, text change to patch applied, bytes per interaction, a full collection at 16 MB and 64 MB, a 100-op patch encode | `tools/bench/wolkit/` (`bench.cr`, `run.sh`, `README.md`) | `sh tools/bench/wolkit/run.sh` (libraries from `WOLKIT_BUILD`, else `make -C wolos-crystal/shards/wolkit all`) | one JSON line, median and p95 over 30 runs (`WOLKIT_BENCH_RUNS`) |
| Allocation per frame (Rust) | `crates/wolkit/tests/alloc_free.rs`, `paint_alloc.rs`, `style_alloc.rs` | `cargo test -p wolkit` | a counting allocator: 0 allocations in a warm frame |
| Allocation per interaction (Crystal) | `wolos-crystal/shards/wolkit/spec/budget_spec.cr` | `make -C wolos-crystal/shards/wolkit spec` | `GC.stats` deltas |
| GPU against the CPU reference | `crates/wolkit/tests/gpu_golden.rs` | `cargo test -p wolkit --test gpu_golden` | no channel off by more than 2, at most 0.1% of pixels off by more than 1 |
| Frame times, damage, draws of a running app | `wolkit_engine_stats_json` (the HUD shows it), the compositor's `wolstats` | in a session | none (observation) |

## Method

The comparison the kit has to win is written down before it is run, so the result cannot pick its own rules.

- **Three sample apps.** 1: to-dos (a list, a text field, 1,000 rows: typing latency, small updates, accessibility). 2: monitor (4 live charts of 1,000 points at 60 to 120 Hz: canvas, continuous animation, damage). 3: table (100,000 rows, search, column sort: virtualisation, scroll, large text).
- **Five stacks.** S1 Wolkit on the Kit backend; S2 Wolkit native with the CPU rasteriser; S3 Wolkit native with Vulkan; S4 today's Kit path through `wol.h` (to-dos only: apps 2 and 3 do not fit its 200-part, no-canvas limits, which is itself a result); S5 a declarative script library in the image's own browser engine, in a kiosk window.
- **Metrics.** Cold start to the first frame; RSS idle and under load; input to photon with the frame-hash probes of `design/tools/perf` (`dmg`, `vk`, `vp`, `latency.py`); the frame-time histogram (p50, p95, p99, missed frames per 1,000 at 120 Hz) from the engine's frame ring and `wolstats`; CPU time per frame from `CLOCK_THREAD_CPUTIME_ID`; energy over a 60 s script from the RAPL sampler (`wol-realtime gpu sample`); GC statistics and pauses; damaged bytes per frame; binary size; lines of app code and the number of concepts per stack.
- **Rules.** Same machine, same scripted scenario, 30 runs, median and p95. A report states the commit, the machine and the display refresh, and says which numbers were taken at 60 Hz headless and which at 120 Hz on hardware. A nightly forge run fails on a regression over 10% against `budgets.toml`.

## Budgets

Targets are for a mid-range integrated GPU at 120 Hz (an 8.33 ms frame). "Measured" is the best evidence so far; the forge is a shared 128-core build machine with no GPU, the CTO box an Ubuntu workstation with no GPU, so every GPU number there comes from a software Vulkan driver.

| # | Budget | Target | Measured |
|---|---|---|---|
| 1 | Input to frame submitted (engine) | p50 <= 3 ms, p95 <= 5 ms | GPU path including the list rebuild with text: CTO box p50 0.80 / p95 1.08 ms, forge p50 2.03 / p95 2.87 ms |
| 2 | Main-thread CPU per interaction frame | <= 2.5 ms p95 | CTO box 0.12 ms, forge 0.35 ms |
| 3 | Handler and effect flush (Crystal) | <= 0.5 ms for <= 20 effects | not yet in a bench |
| 4 | Patch apply | <= 0.2 ms per 100 ops | not yet in a bench (`[patch]` is reserved in `budgets.toml`) |
| 5 | Style resolve | <= 0.2 ms per 100 dirty nodes | forge, release: 100 state flips 72 to 74 us, 100 inline 113 us, restyle of 1,000 nodes 130 us, a clean frame 7 ns |
| 6 | Layout | <= 0.5 ms per 1k-node boundary; full 10k relayout <= 4 ms | forge: boundary 277 to 390 us, an edit inside it 11 to 13 us, full 10k 2.9 to 4.6 ms (through the scene 3.4 to 4.0 ms), resize 0.7 to 0.8 ms; CTO box boundary 225 us, full 10k 2.4 ms |
| 7 | Scroll at 120 Hz | <= 1 ms CPU per frame, no dropped frames over 100k rows | 9 px steps / a fling across all 100,000 rows, p95: CTO box 0.045 / 0.049 ms, forge 0.107 / 0.115 ms; 0 of 6,000 frames over 8.33 ms |
| 8 | Text | run-cache hit <= 2 us; cold 40-character Latin line <= 50 us; atlas hit rate 100% steady | hit 64 ns (CTO box) / 99 ns (forge); cold 3.7 / 5.7 us; atlas lookup 19 / 31 ns; hit rate 1.000 over 50 mixed Latin, CJK and Hebrew frames; a warm 4-line paragraph 11 us |
| 9 | Batching | <= 10 draw calls for a typical window | 1 to 2 for the typical window (sidebar, toolbar, 40 text rows, a menu: about 600 to 900 instances); 4 for the layered golden scene |
| 10 | Damage | a caret blink or hover damages only its bounds; idle 0 | caret 16 px, hover two rows (26,880 px), idle 0 px and 0 draws |
| 11 | Allocation (Rust) | 0 per steady-state frame | 0 over 100 patch applies and 100 frames (`tests/alloc_free.rs`) |
| 12 | Allocation (Crystal) | 0 idle; <= 8 KB per interaction | 0 idle; 285 bytes for a signal write plus its patch, about 750 bytes through the engine |
| 13 | GC pause | p99 <= 3 ms for a heap <= 64 MB | full collection, median / p95: 16 MB 1.5 / 1.6 ms, 64 MB 4.9 / 6.0 ms with parallel marking; one marker 2.9 / 3.0 ms and 13.4 / 13.7 ms. Collecting at idle (after 1 MB allocated) keeps every frame under 0.4 ms, so the shard collects only at idle and keeps its heap small |
| 14 | Start | first frame <= 150 ms native (CPU <= 100 ms), <= 250 ms kit | not yet measured on hardware |
| 15 | Memory | empty window <= 25 MB RSS; <= 350 bytes per node | not yet in a bench |
| 16 | Idle | <= 0.1 ms/s CPU, 0 wakeups | no frame callbacks or timers at idle by construction; not yet measured with `design/tools/perf/idle.py` |

The CPU reference rasteriser draws the typical window at 640 by 480 in 8.2 ms (CTO box) and 13.3 ms (forge): it is the golden reference and the fallback, not the fast path.

## Developer loop

Shard build times for a 3,081-line app of 70 screens on the forge: `crystal build --no-codegen` (the template's `make check`) 1.8 s, a debug build 3.8 s, a rebuild after a one-line change 2.5 s, a release build 17.7 s. The hot-reload target is at most 5 s for a 2,000-line app.

## Not measured yet

The five-stack comparison itself (sample apps 2 and 3 need the canvas and virtual-list paths on the native backend), input to photon on hardware, energy, RSS, start time and the idle probe. When they are run, their results go in a new section here with the commit, the machine and the refresh rate.
