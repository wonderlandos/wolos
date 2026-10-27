//! Frontmost-app QoS hand-off (macOS app boosting): tell wold which process is in front.
//! Wire: `shared::qos` — one line `foreground <pid>` on `shared::qos::SOCKET`.
//! Hot path is allocation-free: stack buffer, repeats dropped, nonblocking writes.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub struct Qos {
    socket: PathBuf,
    conn: Option<UnixStream>,
    last: Option<i32>,
}

impl Default for Qos {
    fn default() -> Self {
        Self::at(shared::qos::SOCKET)
    }
}

impl Qos {
    pub fn at(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            conn: None,
            last: None,
        }
    }

    /// Report the frontmost process; repeats are dropped. Best effort: never blocks.
    /// Returns true when the line was written.
    pub fn foreground(&mut self, pid: i32) -> bool {
        if pid <= 0 || self.last == Some(pid) {
            return false;
        }
        let mut buf = [0u8; 24];
        let n = shared::qos::format_foreground(pid, &mut buf);
        for _ in 0..2 {
            if self.conn.is_none() {
                self.conn = UnixStream::connect(&self.socket).ok();
                if let Some(c) = &self.conn {
                    let _ = c.set_nonblocking(true);
                }
            }
            match self.conn.as_mut().map(|c| c.write_all(&buf[..n])) {
                Some(Ok(())) => {
                    self.last = Some(pid);
                    return true;
                }
                Some(Err(_)) => self.conn = None, // wold restarted: reconnect once
                None => return false,             // wold not running
            }
        }
        false
    }
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
        let mut q = Qos::at(&path);
        assert!(q.foreground(10));
        assert!(!q.foreground(10));
        assert!(!q.foreground(0));
        assert!(q.foreground(11));
        drop(q);
        let mut s = String::new();
        l.accept().unwrap().0.read_to_string(&mut s).unwrap();
        assert_eq!(s, "foreground 10\nforeground 11\n");
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
