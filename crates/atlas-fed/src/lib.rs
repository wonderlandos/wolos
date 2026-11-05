// SPDX-License-Identifier: BSD-2-Clause
//! The Atlas browser's Federate helper (ADR 0052, "Atlas"; ADR 0043 D4: no Rust in the browser).
//! The browser's `fed` URL loader (chrome/browser/atlas/fed in the fork) opens one connection per
//! load to a Unix socket (`$XDG_RUNTIME_DIR/atlas/fed.sock`) and this process answers it with
//! [`federate::serve_load`]: the verified bytes, or the crate's page that says which check failed.
//!
//! Protocol `FED/1`, one request per connection, then the helper closes:
//! - request: `FED/1 D <url>\n` (a document: page or frame) or `FED/1 S <url>\n` (a subresource);
//!   the URL is at most [`MAX_URL`] bytes, printable ASCII without spaces (the browser sends the
//!   canonical, escaped spelling);
//! - reply: `FED/1 <status> <length> <mime>\n`, then exactly `<length>` body bytes.
//!
//! Limits as the web runtime's scheme workers (ADR 0052): [`WORKERS`] loads at once, [`QUEUE`]
//! waiting, one more is answered `503` at once. A silent peer is dropped after
//! [`REQUEST_TIMEOUT`].
#![forbid(unsafe_code)]

use federate::resolve::Outcome;
use federate::{Load, Reply, Source};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// A write to a connection that found the queue full may take this long before it is dropped.
const REFUSE_WRITE_WAIT: Duration = Duration::from_secs(2);
/// The idle watcher looks at the clock at least this often.
const IDLE_TICK_MAX: Duration = Duration::from_secs(1);
/// How long the accept loop waits for the first byte of a connection that arrives during the idle exit.
const WAKE_PEEK: Duration = Duration::from_millis(100);
/// How long a starting helper waits for the lock of one that is leaving (see `bind_wait`).
pub const LOCK_WAIT: Duration = Duration::from_millis(1500);
/// How often a new helper looks again while an old one still holds the lock (`bind_wait`); bounded by [`LOCK_WAIT`].
const LOCK_POLL: Duration = Duration::from_millis(20);

/// The protocol word that starts every request and reply line.
pub const PROTOCOL: &str = "FED/1";
/// Longest URL a request carries (the crate caps a path at 2048 bytes; a query rides along).
pub const MAX_URL: usize = 4096;
/// Longest request line: the word, the kind, two spaces, the URL and the newline.
const MAX_LINE: usize = MAX_URL + 16;
/// Longest reply header line a client reads.
const MAX_HEAD: usize = 512;
/// Loads served at once.
pub const WORKERS: usize = 8;
/// Loads waiting for a worker; one more is refused with 503 at once.
pub const QUEUE: usize = 256;
/// How long a connection may take to send its request line.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a reply may take to be read by the browser.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// What answers one load: the URL as the browser sent it, and its kind.
pub type Handler = Arc<dyn Fn(&str, Load) -> Reply + Send + Sync>;

/// How the server runs: workers, queue, and an idle time after which [`run`] returns.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub workers: usize,
    pub queue: usize,
    /// `None`: serve until the process is stopped.
    pub idle: Option<Duration>,
    /// Time the idle exit waits between deciding to stop and waking the accept loop. Zero in
    /// production; a test widens this window to put a real connection into it (QA-A9-WEB-ATLAS-10).
    pub wake_delay: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            workers: WORKERS,
            queue: QUEUE,
            idle: None,
            wake_delay: Duration::ZERO,
        }
    }
}

/// `$XDG_RUNTIME_DIR/atlas/fed.sock`, beside the bridge's `bridge.sock`; the environment is read
/// through `env` (the binary passes it).
pub fn default_socket(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let dir = env("XDG_RUNTIME_DIR").filter(|d| d.starts_with('/'))?;
    Some(Path::new(&dir).join("atlas").join("fed.sock"))
}

/// One request line (with or without its newline): the kind and the URL, or why not.
pub fn parse_request(line: &[u8]) -> Result<(Load, String), String> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let text = std::str::from_utf8(line).map_err(|_| "the request is not text".to_owned())?;
    let mut parts = text.splitn(3, ' ');
    if parts.next() != Some(PROTOCOL) {
        return Err(format!("the request does not start with {PROTOCOL}"));
    }
    let kind = match parts.next() {
        Some("D") => Load::Document,
        Some("S") => Load::Subresource,
        _ => return Err("the request names no load kind (D or S)".into()),
    };
    let url = parts.next().unwrap_or("");
    if url.is_empty() {
        return Err("the request names no address".into());
    }
    if url.len() > MAX_URL {
        return Err(format!("the address is longer than {MAX_URL} bytes"));
    }
    if !url.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("the address has a space or a control character".into());
    }
    Ok((kind, url.to_owned()))
}

/// The request line for `url` loaded as `kind`.
pub fn request_line(url: &str, kind: Load) -> String {
    let k = match kind {
        Load::Document => "D",
        Load::Subresource => "S",
    };
    format!("{PROTOCOL} {k} {url}\n")
}

/// The content type as it may travel on the header line: printable, no newline, never empty.
fn clean_mime(mime: &str) -> String {
    let m: String = mime
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(200)
        .collect();
    let m = m.trim();
    if m.is_empty() {
        "application/octet-stream".into()
    } else {
        m.to_owned()
    }
}

/// The reply's header line.
pub fn reply_head(reply: &Reply) -> String {
    format!(
        "{PROTOCOL} {} {} {}\n",
        reply.status,
        reply.body.len(),
        clean_mime(&reply.mime)
    )
}

/// Reads one line of at most `max` bytes (the newline included); a peer that sends more, or
/// closes first, gets an error. Reads byte by byte: the peer sends nothing after the line.
fn read_line_bounded(r: &mut impl Read, max: usize) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if r.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the line ended early",
            ));
        }
        line.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(line);
        }
        if line.len() >= max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the line is too long",
            ));
        }
    }
}

/// Reads the request of one connection.
pub fn read_request(r: &mut impl Read) -> Result<(Load, String), String> {
    let line =
        read_line_bounded(r, MAX_LINE).map_err(|e| format!("the request was not read ({e})"))?;
    parse_request(&line)
}

/// Reads one reply (header line and body), refusing a body over `max_body` bytes. For clients:
/// the `--get-socket` command and the tests.
pub fn read_reply(r: &mut impl Read, max_body: u64) -> io::Result<Reply> {
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());
    let line = read_line_bounded(r, MAX_HEAD)?;
    let text = std::str::from_utf8(&line).map_err(|_| bad("the reply is not text"))?;
    let text = text.trim_end_matches('\n');
    let mut parts = text.splitn(4, ' ');
    if parts.next() != Some(PROTOCOL) {
        return Err(bad("the reply does not start with FED/1"));
    }
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("the reply has no status"))?;
    let len: u64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| bad("the reply has no length"))?;
    let mime = parts.next().unwrap_or("").to_owned();
    if len > max_body {
        return Err(bad("the reply is larger than allowed"));
    }
    let mut body = Vec::new();
    r.take(len).read_to_end(&mut body)?;
    if u64::try_from(body.len()).ok() != Some(len) {
        return Err(bad("the reply ended before its length"));
    }
    Ok(Reply { status, mime, body })
}

/// A small page of the helper's own (overload, a fault in the resolver).
fn own_page(status: u16, title: &str, line: &str) -> Reply {
    let body = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n\
<style>:root{{color-scheme:light dark}}body{{font:16px/1.5 system-ui,sans-serif;max-width:40rem;\
margin:12vh auto;padding:0 1.5rem}}h1{{font-size:1.4rem}}</style>\n</head>\n<body>\n<h1>{title}</h1>\n\
<p>{line}</p>\n</body>\n</html>\n"
    );
    Reply {
        status,
        mime: "text/html; charset=utf-8".into(),
        body: body.into_bytes(),
    }
}

/// The answer when every worker is busy and the queue is full.
pub fn busy_reply() -> Reply {
    own_page(
        503,
        "Too many Federate pages are loading",
        "This page asked for more files at once than WolOS loads for it. Load the page again.",
    )
}

fn fault_reply() -> Reply {
    own_page(
        500,
        "This Federate page could not be checked",
        "The part of WolOS that checks Federate pages stopped on this address. Nothing from it was shown.",
    )
}

fn write_reply(w: &mut impl Write, reply: &Reply) -> io::Result<()> {
    w.write_all(reply_head(reply).as_bytes())?;
    w.write_all(&reply.body)?;
    w.flush()
}

/// Serves one connection: reads the request, answers it with `handler`, closes.
pub fn handle(stream: UnixStream, handler: &Handler) {
    handle_after(None, stream, handler);
}

/// [`handle`] for a connection whose first byte was already read (the accept loop reads it during
/// the idle exit, to tell a client from the exit's own wake).
fn handle_after(first: Option<u8>, mut stream: UnixStream, handler: &Handler) {
    // A silent or slow peer cannot hold a worker longer than these.
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let prefix: &[u8] = first.as_slice();
    let reply = match read_request(&mut prefix.chain(&stream)) {
        Ok((kind, url)) => {
            catch_unwind(AssertUnwindSafe(|| handler(&url, kind))).unwrap_or_else(|_| fault_reply())
        }
        Err(why) => federate::page::reply("", Outcome::BadAddress(why)),
    };
    // The browser may have gone (tab closed): nothing to do about a write that fails.
    let _ = write_reply(&mut stream, &reply);
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// Answers a connection that found the queue full, on the accepting thread (a short write with a
/// short timeout, so an overload cannot stall the accept loop for long).
fn refuse(mut stream: UnixStream) {
    let _ = stream.set_write_timeout(Some(REFUSE_WRITE_WAIT));
    let _ = write_reply(&mut stream, &busy_reply());
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

struct Shared {
    /// Connections queued or being served.
    open: AtomicUsize,
    last: Mutex<Instant>,
    stop: AtomicBool,
}

impl Shared {
    fn touch(&self) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }
    fn idle_for(&self) -> Duration {
        self.last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }
}

fn worker(rx: Arc<Mutex<Receiver<Incoming>>>, handler: Handler, shared: Arc<Shared>) {
    loop {
        let next = rx.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok((first, stream)) = next else {
            return;
        };
        handle_after(first, stream, &handler);
        shared.touch();
        shared.open.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A connection for a worker: its first byte if the accept loop already read it, and the stream.
type Incoming = (Option<u8>, UnixStream);

/// What a connection accepted while the idle exit is under way is: the exit's own wake (it
/// closes without a byte), or a client, whose first byte (if it sent one in time) is kept.
fn classify_during_exit(stream: &UnixStream) -> Option<Option<u8>> {
    // None: the wake. Some(first): a client.
    if stream.set_read_timeout(Some(WAKE_PEEK)).is_err() {
        return Some(None);
    }
    let mut byte = [0u8; 1];
    match (&*stream).read(&mut byte) {
        Ok(0) => None,
        Ok(_) => Some(Some(byte[0])),
        Err(_) => Some(None),
    }
}

/// Hands a connection to the workers, or answers 503 at once when the queue is full.
fn enqueue(
    first: Option<u8>,
    stream: UnixStream,
    tx: &mpsc::SyncSender<Incoming>,
    shared: &Shared,
) {
    shared.touch();
    shared.open.fetch_add(1, Ordering::SeqCst);
    match tx.try_send((first, stream)) {
        Ok(()) => {}
        Err(TrySendError::Full((_, stream))) => {
            shared.open.fetch_sub(1, Ordering::SeqCst);
            refuse(stream);
        }
        Err(TrySendError::Disconnected(_)) => {
            shared.open.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Serves `listener` with `handler` until the idle time of `limits` passes with no connection
/// open (then returns `Ok`), or forever without one.
pub fn run(listener: UnixListener, handler: Handler, limits: Limits) -> io::Result<()> {
    let shared = Arc::new(Shared {
        open: AtomicUsize::new(0),
        last: Mutex::new(Instant::now()),
        stop: AtomicBool::new(false),
    });
    let (tx, rx) = mpsc::sync_channel::<Incoming>(limits.queue.max(1));
    let rx = Arc::new(Mutex::new(rx));
    let mut workers = Vec::new();
    for n in 0..limits.workers.max(1) {
        let (rx, handler, shared) = (rx.clone(), handler.clone(), shared.clone());
        workers.push(
            thread::Builder::new()
                .name(format!("atlas-fed-{n}"))
                .spawn(move || worker(rx, handler, shared))?,
        );
    }
    let path = listener
        .local_addr()?
        .as_pathname()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("the socket has no path"));
    if let Some(idle) = limits.idle {
        let path = path
            .as_ref()
            .map_err(|e| io::Error::other(e.to_string()))?
            .clone();
        let shared = shared.clone();
        let wake_delay = limits.wake_delay;
        thread::Builder::new()
            .name("atlas-fed-idle".into())
            .spawn(move || {
                let tick = idle.min(IDLE_TICK_MAX);
                loop {
                    thread::sleep(tick);
                    if shared.open.load(Ordering::SeqCst) == 0 && shared.idle_for() >= idle {
                        shared.stop.store(true, Ordering::SeqCst);
                        thread::sleep(wake_delay);
                        // Wakes the accept below: a connection that sends nothing and closes.
                        // A real client that connects between the decision and this wake is
                        // served, not dropped (see `is_wake`).
                        let _ = UnixStream::connect(&path);
                        return;
                    }
                }
            })?;
    }
    for conn in listener.incoming() {
        let Ok(stream) = conn else {
            continue;
        };
        let mut first = None;
        if shared.stop.load(Ordering::SeqCst) {
            match classify_during_exit(&stream) {
                None => break,
                Some(byte) => first = byte,
            }
        }
        enqueue(first, stream, &tx, &shared);
    }
    // Leaving: new clients find no socket (and start a helper that waits for our lock, see
    // `bind_wait`), and whoever already connected is still answered.
    if let Ok(p) = &path {
        let _ = fs::remove_file(p);
    }
    if listener.set_nonblocking(true).is_ok() {
        while let Ok((stream, _)) = listener.accept() {
            let _ = stream.set_nonblocking(false);
            enqueue(None, stream, &tx, &shared);
        }
    }
    drop(listener);
    drop(tx);
    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

/// A bound socket and the lock that makes this process its only server.
pub struct Bound {
    pub listener: UnixListener,
    pub path: PathBuf,
    _lock: File,
}

/// Binds `path` (mode 0600, its directory 0700), after taking `<path>.lock`; a stale socket of a
/// helper that stopped is replaced. Another helper holding the lock is `AddrInUse`.
pub fn bind(path: &Path) -> io::Result<Bound> {
    bind_wait(path, Duration::ZERO)
}

/// [`bind`], waiting up to `wait` for the lock: a helper that is leaving (idle exit) still holds it
/// for a moment, and a client that found no socket has just started this one, so it takes over
/// instead of exiting and leaving the client to time out (QA-A9-WEB-ATLAS-10). A helper that is
/// serving keeps the lock: after `wait` this is `AddrInUse` as before.
pub fn bind_wait(path: &Path, wait: Duration) -> io::Result<Bound> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the socket path has no directory"))?;
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let mut lock_name = path.as_os_str().to_owned();
    lock_name.push(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(PathBuf::from(lock_name))?;
    let started = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) if started.elapsed() < wait => {
                thread::sleep(LOCK_POLL);
            }
            Err(fs::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("another atlas-fed serves {}", path.display()),
                ));
            }
            Err(fs::TryLockError::Error(e)) => return Err(e),
        }
    }
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(Bound {
        listener,
        path: path.to_path_buf(),
        _lock: lock,
    })
}

/// A source that answers from files under `dir` when they exist (`dir/v1/manifest/<address>`,
/// `dir/v1/block/<address>`, `dir/v1/root`) and from `inner` otherwise. For tests and for showing
/// what a tampering distributor gets: what it serves is checked like anything else (the root key
/// stays pinned), so it can refuse or break a page, never forge one.
pub struct Overlay {
    dir: PathBuf,
    inner: Box<dyn Source>,
}

impl Overlay {
    pub fn new(dir: PathBuf, inner: Box<dyn Source>) -> Overlay {
        Overlay { dir, inner }
    }
}

/// The relative file of a protocol path (`/v1/...`, lower-case letters, digits and slashes, no
/// empty part), or `None`.
pub fn overlay_file(path: &str) -> Option<&str> {
    let rel = path.strip_prefix('/')?;
    let ok = rel.starts_with("v1/")
        && rel
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'/')
        && rel.split('/').all(|p| !p.is_empty());
    ok.then_some(rel)
}

impl Source for Overlay {
    fn get(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        if let Some(rel) = overlay_file(path) {
            let file = self.dir.join(rel);
            if file.is_file() {
                let mut bytes = Vec::new();
                File::open(&file)
                    .and_then(|f| f.take(max + 1).read_to_end(&mut bytes))
                    .map_err(|e| {
                        format!("the overlay file {} was not read ({e})", file.display())
                    })?;
                if u64::try_from(bytes.len()).ok() > Some(max) {
                    return Err(format!(
                        "{path} from the overlay is larger than {max} bytes"
                    ));
                }
                return Ok(bytes);
            }
        }
        self.inner.get(path, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_line_round_trips_for_both_kinds() {
        for kind in [Load::Document, Load::Subresource] {
            let line = request_line("fed://home.fed/style.css", kind);
            assert_eq!(
                parse_request(line.as_bytes()),
                Ok((kind, "fed://home.fed/style.css".to_owned()))
            );
        }
    }

    #[test]
    fn bad_request_lines_are_refused_with_a_reason() {
        let cases: &[&[u8]] = &[
            b"GET fed://home.fed\n",
            b"FED/1 X fed://home.fed\n",
            b"FED/1 D \n",
            b"FED/1 D fed://home.fed/a b\n",
            b"FED/1 D fed://home.fed/\x01\n",
            b"FED/1 D \xff\n",
        ];
        for c in cases {
            assert!(
                parse_request(c).is_err(),
                "{:?}",
                String::from_utf8_lossy(c)
            );
        }
        let long = format!("FED/1 D fed://home.fed/{}\n", "a".repeat(MAX_URL));
        assert!(
            parse_request(long.as_bytes())
                .unwrap_err()
                .contains("longer")
        );
    }

    #[test]
    fn a_reply_round_trips_through_its_header_line() {
        let r = Reply {
            status: 502,
            mime: "text/html; charset=utf-8".into(),
            body: b"<p>no</p>\n".to_vec(),
        };
        let mut wire = Vec::new();
        write_reply(&mut wire, &r).unwrap();
        assert!(wire.starts_with(b"FED/1 502 10 text/html; charset=utf-8\n"));
        assert_eq!(read_reply(&mut wire.as_slice(), 1 << 20).unwrap(), r);
    }

    #[test]
    fn a_reply_header_never_carries_a_newline_from_the_mime() {
        let r = Reply {
            status: 200,
            mime: "text/plain\r\nX-Evil: 1".into(),
            body: vec![],
        };
        let head = reply_head(&r);
        assert_eq!(head.matches('\n').count(), 1);
        assert!(head.ends_with('\n'));
        let empty = Reply {
            status: 200,
            mime: String::new(),
            body: vec![],
        };
        assert!(reply_head(&empty).contains("application/octet-stream"));
    }

    #[test]
    fn a_short_or_oversized_reply_is_an_error() {
        let short = b"FED/1 200 10 text/plain\nabc".to_vec();
        assert!(read_reply(&mut short.as_slice(), 100).is_err());
        let big = b"FED/1 200 1000 text/plain\n".to_vec();
        assert!(read_reply(&mut big.as_slice(), 100).is_err());
    }

    #[test]
    fn overlay_paths_stay_under_v1_and_never_climb() {
        assert_eq!(overlay_file("/v1/root"), Some("v1/root"));
        assert_eq!(overlay_file("/v1/manifest/41ab"), Some("v1/manifest/41ab"));
        for bad in [
            "/v1/../etc",
            "/v1//root",
            "v1/root",
            "/etc/passwd",
            "/v1/Root",
            "/v1/a.b",
        ] {
            assert_eq!(overlay_file(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_socket_lives_in_the_runtime_directory_only() {
        let env = |k: &str| (k == "XDG_RUNTIME_DIR").then(|| "/run/x".to_owned());
        assert_eq!(
            default_socket(&env),
            Some(PathBuf::from("/run/x/atlas/fed.sock"))
        );
        assert_eq!(default_socket(&|_| None), None);
        assert_eq!(default_socket(&|_| Some("relative".into())), None);
    }
}
