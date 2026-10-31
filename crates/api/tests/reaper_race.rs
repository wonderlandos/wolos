// SPDX-License-Identifier: GPL-2.0-or-later
//! wold's reaper waits on every child. A package change that ran fine once came back as failed
//! because the reaper took the exit first. Own binary: the loop below waits on all children.
use api::reaper;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn a_childs_exit_code_survives_the_reaper() {
    static STOP: AtomicBool = AtomicBool::new(false);
    let t = std::thread::spawn(|| {
        while !STOP.load(Ordering::Relaxed) {
            reaper::global().reap_all(|_| false);
        }
    });
    for _ in 0..200 {
        assert_eq!(reaper::run(Command::new("true").arg("x")), 0);
        assert_eq!(reaper::run(Command::new("sh").args(["-c", "exit 3"])), 3);
    }
    STOP.store(true, Ordering::Relaxed);
    t.join().unwrap();
}
