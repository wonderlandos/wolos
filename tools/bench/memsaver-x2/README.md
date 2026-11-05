# MEMSAVER-X2 benchmark: Memory Saver with the runtime tuning, in a real session

Record of the numbers: [docs/bench/memsaver-x2.md](../../../docs/bench/memsaver-x2.md); raw runs in
`docs/bench/results/memsaver-x2/`.

- `x2g.py` runs in the guest as root (a wolvm VM with a desktop session of uid 1000): `window` finds the app's
  window, `sample PID` reads PSS/RSS/SwapPss of the app's process tree and `app-*.scope`, CPU ticks, faults,
  context switches and the kernel knobs, `watch` repeats it, `kill` closes the app.
- `x2drive.py VM MODE APP[,APP] RUNS OUT.jsonl [--shots DIR] [--monitor-run N] [--prime]` runs on the host, starts
  each app through the real launcher (Search, typed as QEMU key events), and walks the phases: 15 s, front idle
  60 s, typing 60 s, behind 10 min (Ctrl+Alt+2), back (Ctrl+Alt+1) with QEMU screenshots for the return times.
- `x2plan.py VM APPS ORDER RUNS` does both modes on one guest: `stock` is the image as installed with the kernel
  knobs at their defaults, `shipped` adds a systemd-sysext layer `zz-x2-shipped` with the binaries of the tree
  (and reboots with `boot-running.yaml`).
- `x2showcase.py VM MODE APP[,APP]` takes the System Monitor Apps-page screenshots after the runs.
- `x2report.py RES_DIR` prints the tables of the record.
