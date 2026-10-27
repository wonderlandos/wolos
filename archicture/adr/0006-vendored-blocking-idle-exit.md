# 0006 - vendored `blocking` with idle thread exit
Status: accepted
Context: zbus (async-io) runs socket setup on the `blocking` crate pool. Upstream 1.7.0 only stops a pool thread when `thread_count > thread_limit`, so the last thread waits in 500 ms slices forever: 2 wakeups/s in an idle wol-shell (measured, `design/tools/perf/idle.py`). tokio is not an option offline and would add a runtime.
Decision: vendor `blocking` 1.7.0 in `third_party/blocking` via `[patch.crates-io]`; idle threads exit after the 500 ms timeout when the queue is empty. Test: `crates/shell/tests/idle_threads.rs`.
Consequences: one patched crate to rebase on upgrade (diff is one condition). Drop the patch once upstream exits idle threads.
