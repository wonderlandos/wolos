//! wold takes SIGCHLD with `sigwait`. FreeBSD discards a signal whose action is "ignore" at delivery,
//! and SIGCHLD's default action is ignore (kern.sig_discard_ign), so a blocked SIG_DFL SIGCHLD never
//! became pending: exited jobs stayed zombies and were never restarted (installed disk: wol-login
//! <defunct> under wold). `harness = false`: a single thread, as in wold before it spawns any.
fn pending(sig: i32) -> bool {
    // SAFETY: sigpending/sigismember on a local set.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigpending(&mut set);
        libc::sigismember(&set, sig) == 1
    }
}

fn main() {
    for s in api::os::SIGNALS {
        // SAFETY: an inherited ignore (daemon(8) and friends) must not survive either.
        unsafe { libc::signal(s, libc::SIG_IGN) };
    }
    api::os::block_signals();
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
