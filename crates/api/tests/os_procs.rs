//! Real process trees (own test binary: the lib tests reap with waitpid(-1)).
use std::time::{Duration, Instant};

#[test]
fn stop_tree_stops_root_and_children() {
    let mut child = std::process::Command::new("sh")
        .args(["-c", "sleep 30 & wait"])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    api::os::stop_tree(child.id() as i32);
    assert!(!child.wait().unwrap().success());
    assert!(t.elapsed() < Duration::from_secs(5));
}
