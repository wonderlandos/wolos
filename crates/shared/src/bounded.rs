// SPDX-License-Identifier: BSD-2-Clause
//! Bounded reading for every line socket (STANDARDS.md section 1 "IPC", ADR 0020): a line has a
//! maximum length, a connection a total byte quota, a socket a read and a write timeout, and a
//! listener a cap on concurrent connections per uid and in total. A peer that exceeds any bound is
//! dropped, never buffered. [`LineSplitter`] is the pure, push-style core (usable from async code
//! such as GIO); [`BoundedLineReader`] is the pull-style wrapper for blocking `Read`ers; [`Quota`]
//! counts connections with an RAII [`Slot`]. [`read_file`] / [`read_text`] read a whole small file
//! under a byte cap (a longer file is refused, never cut).
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Default per-read timeout for a client socket: a peer silent this long is dropped.
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a bounded read stopped. Every variant means: drop the peer.
#[derive(Debug)]
pub enum LineError {
    /// A line longer than the maximum (bytes, without the newline).
    TooLong,
    /// More bytes on this connection than its quota.
    OverQuota,
    /// A line that is not UTF-8 (only from [`BoundedLineReader::read_line_str`]).
    NotUtf8,
    /// The transport failed, including a read timeout.
    Io(std::io::Error),
}

impl std::fmt::Display for LineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong => f.write_str("the peer sent a line longer than allowed"),
            Self::OverQuota => f.write_str("the peer sent more than its connection allows"),
            Self::NotUtf8 => f.write_str("the peer sent a line that is not text"),
            Self::Io(e) => write!(f, "the connection broke: {e}"),
        }
    }
}

impl std::error::Error for LineError {}

/// Splits pushed chunks into newline-terminated lines with a length and a byte quota.
#[derive(Debug)]
pub struct LineSplitter {
    buf: Vec<u8>,
    max_line: usize,
    quota: u64,
    used: u64,
}

impl LineSplitter {
    /// `max_line`: longest line in bytes, newline excluded. `quota`: total bytes this connection
    /// may send (`u64::MAX` for none).
    pub fn new(max_line: usize, quota: u64) -> Self {
        Self {
            buf: Vec::new(),
            max_line,
            quota,
            used: 0,
        }
    }

    /// Add bytes the peer sent. Err when the quota is spent or the pending line is already too
    /// long: the caller drops the peer.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), LineError> {
        let n = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        self.used = self.used.saturating_add(n);
        if self.used > self.quota {
            return Err(LineError::OverQuota);
        }
        self.buf.extend_from_slice(chunk);
        if !self.buf.contains(&b'\n') && self.buf.len() > self.max_line {
            return Err(LineError::TooLong);
        }
        Ok(())
    }

    /// The next complete line without its newline (a trailing `\r` is kept). Err when the next
    /// line is too long; None when no complete line is buffered yet.
    pub fn next_line(&mut self) -> Option<Result<Vec<u8>, LineError>> {
        let Some(nl) = self.buf.iter().position(|b| *b == b'\n') else {
            return (self.buf.len() > self.max_line).then_some(Err(LineError::TooLong));
        };
        if nl > self.max_line {
            return Some(Err(LineError::TooLong));
        }
        let mut line: Vec<u8> = self.buf.drain(..=nl).collect();
        line.pop();
        Some(Ok(line))
    }

    /// Bytes after the last complete line (the unterminated tail at EOF).
    pub fn rest(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

/// A blocking reader that yields bounded lines. The per-read timeout belongs to the socket: set it
/// with [`limit_socket`] before wrapping.
pub struct BoundedLineReader<R> {
    inner: R,
    split: LineSplitter,
    eof: bool,
}

impl<R: Read> BoundedLineReader<R> {
    /// Wrap `inner`; lines at most `max_line` bytes, at most `quota` bytes in total.
    pub fn new(inner: R, max_line: usize, quota: u64) -> Self {
        Self {
            inner,
            split: LineSplitter::new(max_line, quota),
            eof: false,
        }
    }

    /// The wrapped reader (for example to take a descriptor an `FdReader` received).
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Next line without its newline; an unterminated last line is returned at EOF. Ok(None) at
    /// EOF; Err when a bound was exceeded or the read failed or timed out.
    pub fn read_line(&mut self) -> Result<Option<Vec<u8>>, LineError> {
        loop {
            if let Some(line) = self.split.next_line() {
                return line.map(Some);
            }
            if self.eof {
                let rest = self.split.rest();
                return Ok((!rest.is_empty()).then_some(rest));
            }
            let mut chunk = [0u8; 4096];
            match self.inner.read(&mut chunk) {
                Ok(0) => self.eof = true,
                Ok(n) => self.split.push(chunk.get(..n).unwrap_or_default())?,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(LineError::Io(e)),
            }
        }
    }

    /// [`Self::read_line`] as UTF-8 text.
    pub fn read_line_str(&mut self) -> Result<Option<String>, LineError> {
        match self.read_line()? {
            None => Ok(None),
            Some(b) => String::from_utf8(b)
                .map(Some)
                .map_err(|_| LineError::NotUtf8),
        }
    }
}

/// Give a socket a read and a write timeout (`timeout` each). Err only if the OS refuses.
#[cfg(unix)]
pub fn limit_socket(s: &std::os::unix::net::UnixStream, timeout: Duration) -> std::io::Result<()> {
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))
}

/// The whole of the file at `path` when it holds at most `max` bytes. None when it is missing or
/// unreadable, or longer than `max` (never a truncated prefix: one byte past the cap is read to
/// tell a file that fits from one that does not, so a file that grows while being read is
/// refused too). Blocking.
pub fn read_file(path: &std::path::Path, max: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max.saturating_add(1))
        .read_to_end(&mut out)
        .ok()?;
    (u64::try_from(out.len()).ok()? <= max).then_some(out)
}

/// [`read_file`] as UTF-8 text; None also when the bytes are not text.
pub fn read_text(path: &std::path::Path, max: u64) -> Option<String> {
    String::from_utf8(read_file(path, max)?).ok()
}

/// Pause after a refused connection thread: the shortage (TasksMax, RLIMIT_NPROC, memory) lasts
/// until something ends, and the accept loop must not spin on it.
pub const SPAWN_BACKOFF: Duration = Duration::from_millis(500);

/// One connection's (or one stream's) named thread. A refusal (EAGAIN) drops that one connection
/// (the work closure, and the socket it owns), tells `log` (the caller's journal: a library does
/// not print, RR-004) and waits [`SPAWN_BACKOFF`] so the caller's loop does not spin; the daemon
/// stays up and serves the next one. `std::thread::spawn` panics in the thread that called it,
/// which ended an accept loop while the process lived on with a socket that never answered
/// (QA-SHADOW-40, QA-A9-SEC-3/7/11). Returns whether the thread started.
pub fn spawn_conn(what: &str, log: &dyn Fn(&str), work: impl FnOnce() + Send + 'static) -> bool {
    spawn_conn_with(std::thread::Builder::new(), what, log, work)
}

/// [`spawn_conn`] with the thread's `Builder` given (its stack size); the name is `what`.
pub fn spawn_conn_with(
    builder: std::thread::Builder,
    what: &str,
    log: &dyn Fn(&str),
    work: impl FnOnce() + Send + 'static,
) -> bool {
    try_spawn_conn(
        what,
        log,
        work,
        |w| builder.name(what.into()).spawn(w).map(drop),
        std::thread::sleep,
    )
}

/// [`spawn_conn`] with the thread start and the wait given, so a test can refuse the start.
pub fn try_spawn_conn<W>(
    what: &str,
    log: &dyn Fn(&str),
    work: W,
    spawn: impl FnOnce(W) -> std::io::Result<()>,
    wait: impl FnOnce(Duration),
) -> bool {
    match spawn(work) {
        Ok(()) => true,
        Err(e) => {
            log(&format!(
                "{what}: cannot start a connection thread, dropping it: {e}"
            ));
            wait(SPAWN_BACKOFF);
            false
        }
    }
}

#[derive(Debug, Default)]
struct Counts {
    total: usize,
    by_uid: HashMap<u32, usize>,
}

/// Concurrent connections per uid and in total. Cloning shares the counts.
#[derive(Debug, Clone)]
pub struct Quota {
    per_uid: usize,
    total: usize,
    counts: Arc<Mutex<Counts>>,
}

/// One admitted connection; dropping it gives the place back.
#[derive(Debug)]
pub struct Slot {
    uid: u32,
    counts: Arc<Mutex<Counts>>,
}

impl Quota {
    /// At most `per_uid` connections of one uid and `total` of all uids at once.
    pub fn new(per_uid: usize, total: usize) -> Self {
        Self {
            per_uid,
            total,
            counts: Arc::default(),
        }
    }

    /// A place for one more connection of `uid`; None when that uid or everyone is at the cap
    /// (the caller closes the connection).
    pub fn admit(&self, uid: u32) -> Option<Slot> {
        let mut c = self.counts.lock().unwrap_or_else(|p| p.into_inner());
        let mine = c.by_uid.get(&uid).copied().unwrap_or(0);
        if c.total >= self.total || mine >= self.per_uid {
            return None;
        }
        c.total += 1;
        c.by_uid.insert(uid, mine + 1);
        Some(Slot {
            uid,
            counts: self.counts.clone(),
        })
    }

    /// Connections admitted now (all uids).
    pub fn in_use(&self) -> usize {
        self.counts.lock().unwrap_or_else(|p| p.into_inner()).total
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut c = self.counts.lock().unwrap_or_else(|p| p.into_inner());
        c.total = c.total.saturating_sub(1);
        if let Some(n) = c.by_uid.get_mut(&self.uid) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                c.by_uid.remove(&self.uid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    /// QA-A9-SEC-3/7/11: a refused thread drops the work, waits, and reports it; the next start is
    /// served without a wait.
    #[test]
    fn a_refused_connection_thread_backs_off() {
        let waited = std::cell::Cell::new(None);
        let ran = std::cell::Cell::new(false);
        let said = std::cell::RefCell::new(String::new());
        let log = |l: &str| said.borrow_mut().push_str(l);
        let started = try_spawn_conn(
            "x",
            &log,
            || ran.set(true),
            |_| Err(std::io::Error::from_raw_os_error(11)),
            |d| waited.set(Some(d)),
        );
        assert!(!started && !ran.get());
        assert_eq!(waited.get(), Some(SPAWN_BACKOFF));
        assert!(said.borrow().starts_with("x: cannot start"), "{said:?}");
        waited.set(None);
        assert!(try_spawn_conn(
            "x",
            &log,
            || {},
            |_| Ok(()),
            |d| waited.set(Some(d))
        ));
        assert_eq!(waited.get(), None);
    }

    #[test]
    fn spawn_conn_runs_the_work_on_a_named_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(spawn_conn("wol-test-conn", &|_| {}, move || {
            let _ = tx.send(std::thread::current().name().map(str::to_owned));
        }));
        let name = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(name.as_deref(), Some("wol-test-conn"));
    }

    #[test]
    fn read_file_returns_the_whole_file_or_nothing() {
        let d = std::env::temp_dir().join(format!("wol-shared-bounded-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("f");
        std::fs::write(&p, b"12345").unwrap();
        assert_eq!(
            read_file(&p, 5).as_deref(),
            Some(&b"12345"[..]),
            "exactly the cap"
        );
        assert_eq!(read_file(&p, 4), None, "one byte over: refused, not cut");
        assert_eq!(read_text(&p, 64).as_deref(), Some("12345"));
        std::fs::write(&p, [0xff, 0xfe]).unwrap();
        assert_eq!(read_text(&p, 64), None, "not text");
        assert_eq!(read_file(&d.join("missing"), 64), None);
        assert_eq!(read_file(&p, u64::MAX).map(|b| b.len()), Some(2));
        let _ = std::fs::remove_dir_all(&d);
    }

    fn lines(input: &[u8], max: usize, quota: u64) -> Vec<Result<Vec<u8>, String>> {
        let mut r = BoundedLineReader::new(input, max, quota);
        let mut out = Vec::new();
        loop {
            match r.read_line() {
                Ok(Some(l)) => out.push(Ok(l)),
                Ok(None) => return out,
                Err(e) => {
                    out.push(Err(e.to_string()));
                    return out;
                }
            }
        }
    }

    #[test]
    fn splits_lines_and_returns_the_tail_at_eof() {
        assert_eq!(
            lines(b"a\nbc\nd", 8, u64::MAX),
            [Ok(b"a".to_vec()), Ok(b"bc".to_vec()), Ok(b"d".to_vec())]
        );
        assert!(lines(b"", 8, u64::MAX).is_empty());
    }

    #[test]
    fn a_long_line_is_refused_at_the_limit() {
        assert_eq!(
            lines(b"12345678\n", 8, u64::MAX),
            [Ok(b"12345678".to_vec())]
        );
        let got = lines(b"123456789\n", 8, u64::MAX);
        assert!(matches!(got.as_slice(), [Err(_)]), "{got:?}");
        let endless = vec![b'x'; 100_000];
        assert!(matches!(lines(&endless, 8, u64::MAX).as_slice(), [Err(_)]));
    }

    #[test]
    fn the_quota_ends_the_connection() {
        let got = lines(b"a\nb\nc\nd\n", 8, 5);
        assert!(got.last().is_some_and(Result::is_err), "{got:?}");
    }

    #[test]
    fn the_splitter_accepts_chunks_of_any_size() {
        let mut s = LineSplitter::new(4, u64::MAX);
        s.push(b"ab").unwrap();
        assert!(s.next_line().is_none());
        s.push(b"c\nd").unwrap();
        assert_eq!(s.next_line().unwrap().unwrap(), b"abc");
        assert!(s.next_line().is_none());
        assert!(s.push(b"efgh").is_err(), "pending line over the limit");
    }

    #[test]
    fn utf8_is_checked() {
        let mut r = BoundedLineReader::new(&b"\xff\n"[..], 8, u64::MAX);
        assert!(matches!(r.read_line_str(), Err(LineError::NotUtf8)));
    }

    #[test]
    fn a_silent_peer_times_out() {
        let (a, _b) = UnixStream::pair().unwrap();
        limit_socket(&a, Duration::from_millis(50)).unwrap();
        let mut r = BoundedLineReader::new(&a, 8, u64::MAX);
        assert!(matches!(r.read_line(), Err(LineError::Io(_))));
    }

    #[test]
    fn reads_from_a_socket() {
        let (a, mut b) = UnixStream::pair().unwrap();
        b.write_all(b"hello\n").unwrap();
        drop(b);
        let mut r = BoundedLineReader::new(&a, 8, u64::MAX);
        assert_eq!(r.read_line_str().unwrap().as_deref(), Some("hello"));
        assert_eq!(r.read_line_str().unwrap(), None);
    }

    #[test]
    fn quota_caps_per_uid_and_total_and_gives_back() {
        let q = Quota::new(2, 3);
        let a1 = q.admit(1000).unwrap();
        let _a2 = q.admit(1000).unwrap();
        assert!(q.admit(1000).is_none(), "per uid");
        let _b1 = q.admit(1001).unwrap();
        assert!(q.admit(1002).is_none(), "total");
        drop(a1);
        assert_eq!(q.in_use(), 2);
        assert!(q.admit(1000).is_some());
    }
}
