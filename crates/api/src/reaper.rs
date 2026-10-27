//! Single owner of `waitpid`: every exit (job roots and reparented orphans) is reaped here and
//! delivered to whoever registered the pid. Exits that arrive before registration are kept.
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Default)]
pub struct Reaper {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    waiting: HashMap<i32, Sender<i32>>,
    early: HashMap<i32, i32>,
}

impl Reaper {
    /// Receive `pid`'s exit code (128 + signal when killed).
    pub fn register(&self, pid: i32) -> Receiver<i32> {
        let (tx, rx) = channel();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match g.early.remove(&pid) {
            Some(code) => {
                let _ = tx.send(code);
            }
            None => {
                g.waiting.insert(pid, tx);
            }
        }
        rx
    }

    /// `known` = the pid belongs to a job (orphans are dropped instead of kept).
    pub fn exited(&self, pid: i32, code: i32, known: bool) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match g.waiting.remove(&pid) {
            Some(tx) => {
                let _ = tx.send(code);
            }
            None if known => {
                g.early.insert(pid, code);
            }
            None => {}
        }
    }

    /// Reap every dead child without blocking. `is_job` tells job roots from orphans.
    pub fn reap_all(&self, is_job: impl Fn(i32) -> bool) -> usize {
        let mut n = 0;
        loop {
            let mut status = 0;
            // SAFETY: waitpid with a valid out pointer.
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
            if pid <= 0 {
                return n;
            }
            n += 1;
            self.exited(pid, exit_code(status), is_job(pid));
        }
    }
}

/// waitpid status -> shell-style exit code.
pub fn exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        -1
    }
}

/// Become the reaper of every descendant (FreeBSD procctl, Linux subreaper). Best effort.
pub fn acquire() -> bool {
    #[cfg(target_os = "freebsd")]
    // SAFETY: procctl on our own pid with no data.
    unsafe {
        libc::procctl(
            libc::P_PID,
            libc::getpid() as libc::id_t,
            libc::PROC_REAP_ACQUIRE,
            std::ptr::null_mut(),
        ) == 0
    }
    #[cfg(target_os = "linux")]
    // SAFETY: prctl with integer arguments.
    unsafe {
        libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == 0
    }
    #[cfg(not(any(target_os = "freebsd", target_os = "linux")))]
    false
}

/// `struct procctl_reaper_kill` (sys/procctl.h); not exported by libc.
#[cfg(target_os = "freebsd")]
#[repr(C)]
struct ReaperKill {
    rk_sig: libc::c_int,
    rk_flags: libc::c_uint,
    rk_subtree: libc::pid_t,
    rk_killed: libc::c_uint,
    rk_fpid: libc::pid_t,
    rk_pad0: [libc::c_uint; 15],
}

/// FreeBSD: signal the whole reaper subtree. Returns false where unsupported.
pub fn kill_all(sig: i32) -> bool {
    #[cfg(target_os = "freebsd")]
    // SAFETY: procctl with a zeroed, correctly typed reap_kill struct.
    unsafe {
        let mut rk: ReaperKill = std::mem::zeroed();
        rk.rk_sig = sig;
        libc::procctl(
            libc::P_PID,
            libc::getpid() as libc::id_t,
            libc::PROC_REAP_KILL,
            (&raw mut rk).cast(),
        ) == 0
    }
    #[cfg(not(target_os = "freebsd"))]
    {
        let _ = sig;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_before_and_after_registration() {
        let r = Reaper::default();
        let rx = r.register(10);
        r.exited(10, 3, true);
        assert_eq!(rx.recv().unwrap(), 3);
        r.exited(11, 4, true); // exit raced ahead of register
        assert_eq!(r.register(11).recv().unwrap(), 4);
        r.exited(12, 0, false); // orphan: dropped
        assert!(r.register(12).try_recv().is_err());
    }

    #[test]
    #[allow(clippy::zombie_processes)] // reaped by Reaper::reap_all, the point of the test
    fn reaps_real_child_with_code() {
        let r = Reaper::default();
        let child = std::process::Command::new("sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let rx = r.register(pid);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while rx.try_recv().map(|c| assert_eq!(c, 7)).is_err() {
            r.reap_all(|p| p == pid);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
