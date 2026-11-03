// SPDX-License-Identifier: GPL-2.0-or-later
//! Frontmost-app QoS hand-off (macOS app boosting): tell wold which process is in front.
//! Wire: `shared::qos` — one line `foreground <pid>` on `shared::qos::SOCKET`.
//! Hot path is allocation-free: stack buffer, repeats dropped, nonblocking writes.
//!
//! Invariant (audit S4-8): `foreground` runs on the compositor thread (the compositor's focus hook) and
//! never blocks. Connecting and the ADR 0033 hello (up to `shared::jobs::HELLO_TIMEOUT`) happen
//! on a helper thread, which hands the ready, nonblocking stream over and sends the newest pid
//! asked for meanwhile.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

/// After a failed connect, the next try waits this long (wold not running: stay cheap).
const RETRY_MS: u64 = 1000;

/// What the compositor thread and the connecting thread share.
struct Link {
    socket: PathBuf,
    /// The greeted, nonblocking connection; taken with `try_lock` only on the hot path.
    conn: Mutex<Option<UnixStream>>,
    /// A helper thread is connecting now.
    connecting: AtomicBool,
    /// Newest pid asked for while there was no connection (0: none); sent once connected.
    pending: AtomicI32,
    /// The last pid wold was told (0: none, or unknown after a failure).
    last: AtomicI32,
    /// Milliseconds since `epoch` before which no new connect is tried.
    retry_at: AtomicU64,
    epoch: std::time::Instant,
}

pub struct Qos {
    link: Arc<Link>,
}

impl Default for Qos {
    fn default() -> Self {
        Self::at(shared::qos::SOCKET)
    }
}

impl Qos {
    pub fn at(socket: impl Into<PathBuf>) -> Self {
        Self {
            link: Arc::new(Link {
                socket: socket.into(),
                conn: Mutex::new(None),
                connecting: AtomicBool::new(false),
                pending: AtomicI32::new(0),
                last: AtomicI32::new(0),
                retry_at: AtomicU64::new(0),
                epoch: std::time::Instant::now(),
            }),
        }
    }

    /// Report the frontmost process; repeats are dropped. Never blocks (S4-8): without a
    /// connection the pid is kept and a helper thread connects and sends it. Returns true when
    /// the line was written now.
    pub fn foreground(&mut self, pid: i32) -> bool {
        let l = &self.link;
        if pid <= 0 || l.last.load(Relaxed) == pid {
            return false;
        }
        let mut buf = [0u8; 24];
        let n = shared::qos::format_foreground(pid, &mut buf);
        // The helper holds the lock only to hand the stream over: never wait for it.
        let Ok(mut conn) = l.conn.try_lock() else {
            l.pending.store(pid, Relaxed);
            return false;
        };
        if let Some(c) = conn.as_mut() {
            if c.write_all(&buf[..n]).is_ok() {
                l.last.store(pid, Relaxed);
                return true;
            }
            *conn = None; // wold restarted: connect again
        }
        drop(conn);
        l.pending.store(pid, Relaxed);
        l.last.store(0, Relaxed);
        self.connect_later();
        false
    }

    /// Start the helper thread unless one runs or the last try failed less than [`RETRY_MS`] ago.
    fn connect_later(&self) {
        let l = &self.link;
        let now = u64::try_from(l.epoch.elapsed().as_millis()).unwrap_or(u64::MAX);
        if now < l.retry_at.load(Relaxed) || l.connecting.swap(true, Relaxed) {
            return;
        }
        let link = self.link.clone();
        let started = std::thread::Builder::new()
            .name("wol-qos-connect".into())
            .spawn(move || connect(&link));
        if started.is_err() {
            l.connecting.store(false, Relaxed);
        }
    }
}

/// The helper thread: connect, answer wold's hello, send the newest pending pid, hand over.
fn connect(l: &Link) {
    let ready = shared::jobs::connect(&l.socket).and_then(|mut c| {
        c.set_nonblocking(true)?;
        let pid = l.pending.swap(0, Relaxed);
        if pid > 0 {
            let mut buf = [0u8; 24];
            let n = shared::qos::format_foreground(pid, &mut buf);
            c.write_all(&buf[..n])?;
            l.last.store(pid, Relaxed);
        }
        Ok(c)
    });
    match ready {
        Ok(mut c) => {
            let mut conn = l.conn.lock().unwrap_or_else(|p| p.into_inner());
            // A pid asked for while this thread was greeting: send it now, under the lock, so
            // the hot path cannot race it.
            let pid = l.pending.swap(0, Relaxed);
            if pid > 0 {
                let mut buf = [0u8; 24];
                let n = shared::qos::format_foreground(pid, &mut buf);
                if c.write_all(&buf[..n]).is_ok() {
                    l.last.store(pid, Relaxed);
                }
            }
            *conn = Some(c);
        }
        Err(_) => {
            let now = u64::try_from(l.epoch.elapsed().as_millis()).unwrap_or(u64::MAX);
            l.retry_at.store(now.saturating_add(RETRY_MS), Relaxed);
        }
    }
    l.connecting.store(false, Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    #[test]
    fn sends_wire_lines_and_drops_repeats() {
        let dir = std::env::temp_dir().join(format!("wol-qos-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("q.sock");
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path).unwrap();
        let wold = std::thread::spawn(move || {
            let (c, _) = l.accept().unwrap();
            shared::jobs::greet(&c).unwrap();
            let mut s = String::new();
            (&c).read_to_string(&mut s).unwrap();
            s
        });
        let mut q = Qos::at(&path);
        // The first call connects off-thread and is sent by the helper once wold greeted.
        assert!(!q.foreground(10));
        let t = std::time::Instant::now();
        while q.link.last.load(Relaxed) != 10 {
            assert!(
                t.elapsed() < std::time::Duration::from_secs(5),
                "never connected"
            );
            std::thread::yield_now();
        }
        // Wait for the hand-over, then the hot path writes directly.
        while q.link.conn.lock().unwrap().is_none() {
            std::thread::yield_now();
        }
        assert!(!q.foreground(10), "a repeat is dropped");
        assert!(!q.foreground(0));
        assert!(q.foreground(11));
        drop(q);
        let s = wold.join().unwrap();
        assert_eq!(s, "foreground 10\nforeground 11\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Audit S4-8: a wold that accepts but never greets (starting, under memory pressure) does
    /// not hold the compositor thread: every call returns at once.
    #[test]
    fn a_wold_that_does_not_greet_never_blocks_the_caller() {
        let dir = std::env::temp_dir().join(format!("wol-qos-mute-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("q.sock");
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let mute = std::thread::spawn(move || {
            let (_c, _) = l.accept().unwrap();
            let _ = rx.recv(); // holds the connection, says nothing
        });
        let mut q = Qos::at(&path);
        let t = std::time::Instant::now();
        for pid in 1..50 {
            assert!(!q.foreground(pid));
        }
        assert!(
            t.elapsed() < std::time::Duration::from_millis(100),
            "{:?}",
            t.elapsed()
        );
        let _ = tx.send(());
        mute.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_daemon_is_cheap() {
        let mut q = Qos::at("/nonexistent/wol-qos.sock");
        let t = std::time::Instant::now();
        for pid in 1..1000 {
            q.foreground(pid);
        }
        assert!(t.elapsed().as_millis() < 200, "{:?}", t.elapsed());
    }
}
