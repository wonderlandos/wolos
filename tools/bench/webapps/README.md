# Web-app benchmark

`sh run.sh VM [runner args]` measures every web-based app WolOS ships or offers, plus native comparison apps, inside an installed WolOS VM driven by `wolvm` (`wolos-iso/tests/wolvm`). The baseline and the gap analysis built from it are `docs/bench/webapps-baseline.md`. Every later web-runtime package is gated on these numbers: rerun on the same VM image and compare.

Files:

- `apps.json`: the rows. `kind` `web` (a browser engine draws the window), `native` (comparison) or `candidate` (a runtime under evaluation). `origin` says where WolOS ships or offers it: `base`, `store:<catalog id>`, `module:<id>`, `webapp` (the `wol webapp add` window) or `example`. `wl_argv` (optional) is the command for the protocol-log start when the launcher detaches the app from its stderr (`code`'s command-line front end does). Store apps that are not web-based are listed under `not_web` with the reason. `crates/controller/tests/bench_webapps.rs` fails when a Store app, a module or the web-app launcher is missing here, so a new app is measured.
- `webapps.py`: the runner (`run`, as root in the guest) and the report (`report RESULT.json`). Python standard library only.
- `features.js`: a web-platform feature probe the runner loads in every row with `features: true`.
- `run.sh`: host side. Builds the input probe `vk` (`design/tools/perf/build.sh`), ships the runner, fixtures and optional extras into the guest over QEMU user networking, starts the runner as the transient unit `wolbench-webapps`, waits, and writes `$OUT/result.json`, `$OUT/result.md` and `$OUT/runner.log`. Optional: `WOLKIT_TODOS` and `WOLKIT_LIB` (a release build of `wolos-crystal/examples/todos` and its libraries), `SERVO_DIR` (the Rust browser engine's shell), `WOLBENCH_PASSWORD` (unlocks a locked test session).

What one row costs and measures, per Memory Saver mode (`--modes on,off`):

1. A first run (`--prime` seconds) before anything is timed, so first-run pages and profile creation are not in the numbers; then the feature probe.
2. Cold start (page cache dropped) and `--warm` warm starts. Window: the compositor's `openwindow` event after the launch (a mapped window has drawn its first buffer). Settled: the start of the first whole second in which the app's processes use under 5 % of one CPU.
3. Memory of the whole app (the launched process tree, plus new processes in its app scope; never a process that ran before the launch, so the shell's Kit host counts only when the launch started it): RSS, PSS, swap and swap PSS from `smaps_rollup`, after launch, after `--idle` seconds idle in front, and after `--idle` seconds in the background (another workspace: Memory Saver's cue). The idle-in-front sample also splits the PSS by process role (`roles`: `--type=` word, `gpu`, `child`, else the executable's name).
4. Idle cost over the second half of each idle phase: CPU ms per second and wakeups (context switches) per second, summed over every thread (as `design/tools/perf/idle.py`).
5. Coming back: show the app, send one key, the time until its CPU settles again and the page faults on the way.
6. Rows with `interact`: a second warm start with the client's Wayland protocol log (`WAYLAND_DEBUG=client`). The scripted interaction (15 pages down, 15 up, 200 ms apart; Tab; 43 characters typed 60 ms apart) is sent with `vk`, whose own protocol log stamps each key press in the same clock. Key to frame: the first `wl_surface.commit` after the press. Frame interval: the gaps between commits closer than 100 ms. Then `--back` seconds in the background and the first key's latency back. The log costs CPU, which is why idle numbers come from the first start.

Session conditions the runner sets: the idle daemon is stopped for the run (no lock, display-off or power-off; restarted afterwards with the same command line), Memory Saver's session side is started when nothing started it (`saver_session` in the result says which), and `~/.config/wolos/memory-saver.json` is set per mode and left on at the end.

Not measured by design: frames of X11 windows (no client protocol log), apps that need an account to show content (their rows have `interact: none`), GPU paths (the VM has no GPU; every number is software rendering).

Example, the full baseline on the forge VM `ms`:

    WOLVM=/scratch/split/wolos-iso/tests/wolvm/wolvm WOLVM_HOME=/scratch/memsaver/wolvm-home \
    WOLKIT_TODOS=/scratch/memsaver/todos WOLKIT_LIB=$PWD/wolos-crystal/shards/wolkit/build \
    SERVO_DIR=/scratch/memsaver/engine-shell OUT=/scratch/memsaver/baseline \
    sh tools/bench/webapps/run.sh ms

A quick check of one app: `sh run.sh ms --only code --modes on --warm 1 --idle 20 --back 40`.
