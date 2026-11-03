# Wolkit benchmarks

`sh run.sh` measures what the Crystal shard adds on top of the engine, for sample app 1 (todos) on the headless engine:

- cold start to the first frame (in process: engine created, first patch applied; not the process start);
- the time from a text change to the patch applied (shard side of budget 1);
- bytes allocated per interaction (budget 12; the limit is 8 KB);
- a full collection at a 16 MB and a 64 MB heap (budget 13);
- the encode cost of a 100-op patch.

Result: one JSON line with `median` and `p95` over 30 runs (`WOLKIT_BENCH_RUNS` to change). Build the engine first (`make -C wolos-crystal/shards/wolkit all`) or point `WOLKIT_BUILD` at a directory with `libwolkit.so.1` and `libwol.so.1`.

The numbers are only comparable on the same machine with the same commit. The engine's frame times and damage come from `wolkit_engine_stats_json` (the HUD shows them) and the compositor's `wolstats`; the credible multi-stack comparison (Kit path, native CPU, native Vulkan, web) and its rules are in `docs/modules/wolkit-bench.md`. Apps 2 and 3 (monitor, table) need the canvas and virtual list of the GPU stream and are not here yet.
