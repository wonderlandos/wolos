//! Live perf: wol-shell's zbus connection left a `blocking-1` pool thread waking every 500 ms
//! forever (2 wakeups/s idle). Idle blocking threads must exit (vendored `blocking` patch).
#![cfg(target_os = "linux")]

fn blocking_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|t| std::fs::read_to_string(t.ok()?.path().join("comm")).ok())
        .filter(|n| n.starts_with("blocking-"))
        .count()
}

#[test]
fn idle_blocking_pool_threads_exit() {
    let v = futures_lite_block_on(blocking::unblock(|| 7));
    assert_eq!(v, 7);
    assert!(blocking_threads() >= 1, "pool thread started");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(blocking_threads(), 0, "idle pool thread still polling");
}

fn futures_lite_block_on<T>(f: impl std::future::Future<Output = T>) -> T {
    async_io::block_on(f)
}
