// SPDX-License-Identifier: GPL-2.0-or-later
//! wold's reaper waits on every child. A package change that ran fine once came back as failed
//! because the reaper took the exit first. Own binary: the loop below waits on all children.
use api::reaper;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const LONG: Duration = Duration::from_secs(60);

#[test]
fn a_childs_exit_code_survives_the_reaper() {
    static STOP: AtomicBool = AtomicBool::new(false);
    let t = std::thread::spawn(|| {
        while !STOP.load(Ordering::Relaxed) {
            reaper::global().reap_all(|_| false);
        }
    });
    for _ in 0..200 {
        assert_eq!(reaper::run(Command::new("true").arg("x"), LONG), 0);
        assert_eq!(
            reaper::run(Command::new("sh").args(["-c", "exit 3"]), LONG),
            3
        );
    }
    STOP.store(true, Ordering::Relaxed);
    t.join().unwrap();
}

/// Audit S4-11: a command past its deadline is stopped with everything it started (its process
/// group), and the answer is TIMED_OUT, promptly.
#[test]
fn a_command_past_its_deadline_is_stopped_with_its_children() {
    let dir = std::env::temp_dir().join(format!("wol-reaper-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pidfile = dir.join("grandchild");
    // A child that starts a grandchild (pacman under wol pkg) and both hang.
    let script = format!("sleep 30 & echo $! > {}; sleep 30", pidfile.display());
    let t = Instant::now();
    let code = reaper::run(
        Command::new("sh").args(["-c", &script]),
        Duration::from_millis(300),
    );
    assert_eq!(code, reaper::TIMED_OUT);
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    // The grandchild got the group's SIGTERM too: it is gone within seconds, not 30.
    let pid = std::fs::read_to_string(&pidfile).unwrap();
    // Through reaper::run: the other test's reaper thread may take any child's exit.
    let gone = reaper::run(
        Command::new("timeout").args([
            "5",
            "tail",
            &format!("--pid={}", pid.trim()),
            "-f",
            "/dev/null",
        ]),
        LONG,
    );
    assert_eq!(gone, 0, "the grandchild outlived the deadline");
    let _ = std::fs::remove_dir_all(&dir);
}
