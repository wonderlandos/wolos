# js-runtime-x benchmark (ADR 0061)

Record of the numbers: [docs/bench/js-runtime-x.md](../../../docs/bench/js-runtime-x.md).

- `jsx.py LABEL BGSECS` runs in a guest as root (a wolvm VM with a desktop session of uid 1000 and the
  helpers of `tools/bench/discord/`: `as-user.sh`, `cdp.py`). It reads `/var/tmp/jsx/LABEL.{pre,launch,post}`
  (shell: kernel knobs before the start; the command the compositor starts; the page-out run once after
  60 s), optional `LABEL.loop` (post again every 30 s in the background), `LABEL.cdp` (frame and script
  call latency through DevTools on port 9222), `LABEL.comms` and `LABEL.winclass` (processes and window
  class of a non-Discord app), `LABEL.noreset` (keep the knobs as booted). It writes
  `/var/tmp/jsx/LABEL.jsonl`: PSS split anon/file/shmem, SwapPss, per-role and per-mapping PSS, ksmd CPU,
  faults, MemAvailable, zswap.
- `vma.py` splits the PSS of the Discord processes by mapping and by shared/private clean/dirty.
- `bun-run.sh NAME [ENV=VAL ...] -- [bun flags]` runs `bun --bun x vite build` in `site/` five times to
  `/tmp`, prints the median peak RSS and wall time.
