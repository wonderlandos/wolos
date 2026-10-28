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

/// Become the reaper of every descendant (Linux child subreaper). Best effort.
pub fn acquire() -> bool {
    #[cfg(target_os = "linux")]
    // SAFETY: prctl with integer arguments.
    unsafe {
        libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == 0
    }
    #[cfg(not(target_os = "linux"))]
    false
}

/// Every descendant of `root` in a `(pid, ppid)` table (orphans re-parent to the subreaper, so
/// the table holds the whole subtree).
pub fn descendants(root: i32, table: &[(i32, i32)]) -> Vec<i32> {
    let mut out = Vec::new();
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for &(pid, ppid) in table {
            if ppid == parent && pid != root && !out.contains(&pid) {
                out.push(pid);
                frontier.push(pid);
            }
        }
    }
    out
}

/// `(pid, ppid)` from `/proc/<pid>/stat` text (the command may contain spaces and parentheses).
pub fn parse_stat(text: &str) -> Option<(i32, i32)> {
    let pid = text.split_whitespace().next()?.parse().ok()?;
    let rest = &text[text.rfind(')')? + 1..];
    let ppid = rest.split_whitespace().nth(1)?.parse().ok()?;
    Some((pid, ppid))
}

/// Signal the whole subtree under this process (read from /proc). False when none was signalled.
pub fn kill_all(sig: i32) -> bool {
    let table: Vec<(i32, i32)> = std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("stat")).ok())
        .filter_map(|t| parse_stat(&t))
        .collect();
    let me = std::process::id() as i32;
    let pids = descendants(me, &table);
    // SAFETY: kill(2) on pids read from /proc; a pid that already exited just fails.
    let sent = pids
        .iter()
        .filter(|p| unsafe { libc::kill(**p, sig) } == 0)
        .count();
    sent > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtree_from_the_process_table() {
        let t = [(10, 1), (11, 10), (12, 11), (13, 1), (14, 12)];
        let mut d = descendants(10, &t);
        d.sort();
        assert_eq!(d, [11, 12, 14]);
        assert!(descendants(99, &t).is_empty());
        assert_eq!(
            parse_stat("42 (a (b) c) S 7 42 42 0 -1"),
            Some((42, 7)),
            "command with spaces and parens"
        );
        assert_eq!(parse_stat("garbage"), None);
    }

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
