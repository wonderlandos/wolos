// SPDX-License-Identifier: GPL-2.0-or-later
//! wold takes SIGCHLD with `sigwait`. FreeBSD discards a signal whose action is "ignore" at delivery,
//! and SIGCHLD's default action is ignore (kern.sig_discard_ign), so a blocked SIG_DFL SIGCHLD never
//! became pending: exited jobs stayed zombies and were never restarted (installed disk: wol-login
//! <defunct> under wold). `harness = false`: a single thread, as in wold before it spawns any.
/// Is `sig` pending (`pending`), or blocked in this thread's mask?
fn has(pending: bool, sig: i32) -> bool {
    // SAFETY: sigpending, or pthread_sigmask as a pure readback (null new set), and sigismember,
    // all on a local set.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        if pending {
            libc::sigpending(&mut set);
        } else {
            libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut set);
        }
        libc::sigismember(&set, sig) == 1
    }
}

fn pending(sig: i32) -> bool {
    has(true, sig)
}

fn blocked(sig: i32) -> bool {
    has(false, sig)
}

fn main() {
    for s in api::os::SIGNALS {
        // SAFETY: an inherited ignore (daemon(8) and friends) must not survive either.
        unsafe { libc::signal(s, libc::SIG_IGN) };
    }
    api::os::block_signals();
    // S4-2: a hang-up or a broken pipe can never end wold: both are blocked in every thread
    // (sigprocmask readback), so they wait for `wait_signal`, which ignores them.
    for s in [libc::SIGHUP, libc::SIGPIPE, libc::SIGTERM] {
        assert!(blocked(s), "signal {s} is not blocked");
    }
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !pending(libc::SIGCHLD) {
        assert!(
            std::time::Instant::now() < deadline,
            "SIGCHLD was discarded"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(api::os::wait_signal(), libc::SIGCHLD);
    child.wait().unwrap();
    println!("signals: SIGCHLD reached wait_signal");
}
