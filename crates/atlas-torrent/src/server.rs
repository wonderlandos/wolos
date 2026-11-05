// SPDX-License-Identifier: BSD-2-Clause
//! The socket side: `$XDG_RUNTIME_DIR/atlas/torrent.sock` in a 0700 folder, the socket 0600; a
//! connection is accepted only from this account (SO_PEERCRED uid) running one of the allowed
//! programs (the browser, `/proc/<pid>/exe` read at accept), as the bridge does. The first line must
//! be `hello`; then requests in, events out, plus a row for every job whose row changed.
use crate::engine::Engine;
use crate::proto::{self, ErrorCode, Event, Job, Request};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};

/// `$XDG_RUNTIME_DIR/atlas/torrent.sock`.
pub fn default_socket(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let dir = env("XDG_RUNTIME_DIR").filter(|d| d.starts_with('/'))?;
    Some(Path::new(&dir).join("atlas").join("torrent.sock"))
}

/// Who may talk to the helper: this account (SO_PEERCRED uid; another account's uid is not even
/// mapped into the service's user namespace) and, outside a unit (tests), the browser program by
/// `/proc/<pid>/exe`. Inside the confined service another process's `/proc/<pid>/exe` cannot be
/// read (user namespace), so there the real check is uid + a per-launch token: `launch` makes a
/// random one, hands it to the service in a 0600 file it reads once and removes (never argv or
/// environment: DL-SEC-06), and prints it to the browser that ran it; the browser's hello carries
/// it, compared in constant time; a wrong hello closes the connection.
#[derive(Debug, Clone)]
pub struct Peers {
    /// Canonical paths of the programs allowed (the browser binary), checked when readable.
    pub exes: Vec<PathBuf>,
    /// The launch token; None: no token (an unconfined run, as in the tests).
    pub token: Option<String>,
}

/// What the accept found out about a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    Refused,
    /// The program was seen and is the browser.
    Browser,
    /// This account; the program could not be seen, so the hello must carry the token.
    NeedsToken,
}

impl Peers {
    pub fn check(&self, s: &impl std::os::fd::AsRawFd) -> Peer {
        let Some(p) = shared::sys::peer(s) else {
            return Peer::Refused;
        };
        if p.uid != shared::sys::euid() || p.pid <= 0 {
            return Peer::Refused;
        }
        // The peer's /proc entry is pinned once, at accept, and its program read through that
        // descriptor (STANDARDS 7.1 Identity: a pid number may name another process later).
        let exe = std::fs::File::open(format!("/proc/{}", p.pid)).and_then(|dir| {
            use std::os::fd::AsRawFd;
            std::fs::read_link(format!("/proc/self/fd/{}/exe", dir.as_raw_fd()))
        });
        match exe {
            Ok(exe) if self.exes.contains(&exe) => Peer::Browser,
            Ok(_) => Peer::Refused,
            Err(_) if self.token.is_some() => Peer::NeedsToken,
            Err(_) => Peer::Refused,
        }
    }

    /// The hello's token is the launch token (compared in constant time).
    pub fn token_ok(&self, got: Option<&str>) -> bool {
        match (&self.token, got) {
            (Some(want), Some(got)) if want.len() == got.len() => {
                want.bytes()
                    .zip(got.bytes())
                    .fold(0u8, |a, (x, y)| a | (x ^ y))
                    == 0
            }
            _ => false,
        }
    }
}

/// Write the launch token into a fresh 0600 file `token.<tag>` of the 0700 socket folder `run`
/// (DL-SEC-06: never argv or the service environment, which other programs of the account read
/// through `/proc` and `systemctl show`).
pub fn write_token_file(run: &Path, token: &str, tag: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::set_permissions(run, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("socket folder: {e}"))?;
    let p = run.join(format!("token.{tag}"));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&p)
        .map_err(|e| format!("token file: {e}"))?;
    f.write_all(token.as_bytes())
        .map_err(|e| format!("token file: {e}"))?;
    Ok(p)
}

/// Read the token once and remove the file (the service does it right after its confinement
/// check); None when it is missing or is not 64 hex digits.
pub fn take_token_file(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut s = String::new();
    let r = std::fs::File::open(p).and_then(|f| f.take(128).read_to_string(&mut s));
    let _ = std::fs::remove_file(p);
    r.ok()?;
    let t = s.trim().to_string();
    (t.len() == 64 && t.bytes().all(|c| c.is_ascii_hexdigit())).then_some(t)
}

/// Bind the socket: the folder is made 0700 (and must belong to this account), a stale socket
/// nobody answers on is removed, a live one means another helper runs (error).
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("socket has no folder"))?;
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "a torrent helper already runs",
        ));
    }
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(l)
}

/// How long the helper stays with nothing to do (no job looking for metadata, checking or
/// downloading) before it ends, even while the browser is connected: the DHT node and the peer
/// connections live only while a job needs them (DL-SEC-08). The browser starts it again for the
/// next job.
pub const NO_WORK: Duration = Duration::from_secs(20);
/// How often the serve loop looks at its connections and jobs while nobody connects.
const ACCEPT_TICK: Duration = Duration::from_secs(1);

/// Serve until idle: no connection from the browser for `idle` (None: until killed), or nothing to
/// do for [`NO_WORK`]. When the last connection drops every running job is paused at once, so
/// while the helper waits for the browser it opens no new peer connections and asks no tracker;
/// the jobs it paused continue if the browser is back in time.
pub async fn run(
    listener: UnixListener,
    engine: Arc<Engine>,
    peers: Peers,
    idle: Option<Duration>,
) {
    let (tx, _) = broadcast::channel::<Event>(256);
    let conns = Arc::new(AtomicUsize::new(0));
    // rows: every tick, the rows that changed go to every connection
    {
        let tx = tx.clone();
        let engine = Arc::clone(&engine);
        tokio::spawn(async move {
            let mut last: HashMap<String, Job> = HashMap::new();
            loop {
                tokio::time::sleep(Duration::from_millis(proto::TICK_MS)).await;
                let now = engine.jobs().await;
                for j in &now {
                    if last.get(&j.id) != Some(j) {
                        let _ = tx.send(Event::Job(j.clone()));
                    }
                }
                for id in last.keys() {
                    if !now.iter().any(|j| &j.id == id) {
                        let _ = tx.send(Event::Gone { id: id.clone() });
                    }
                }
                last = now.into_iter().map(|j| (j.id.clone(), j)).collect();
            }
        });
    }
    let mut quiet_since = Instant::now();
    let mut working_since = Instant::now();
    let mut away = false;
    loop {
        let accept = tokio::time::timeout(ACCEPT_TICK, listener.accept()).await;
        match accept {
            Ok(Ok((s, _))) => {
                let who = peers.check(&s);
                if who == Peer::Refused {
                    continue; // not this account, or not the browser: closed unanswered
                }
                if conns.load(Ordering::SeqCst) == 0 {
                    engine.resume_after_absence().await;
                }
                away = false;
                working_since = Instant::now();
                let peers = peers.clone();
                let engine = Arc::clone(&engine);
                let rx = tx.subscribe();
                let conns = Arc::clone(&conns);
                conns.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    serve_one(s, engine, rx, who, &peers).await;
                    conns.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Ok(Err(_)) => {} // a failed accept is the peer's; the next one is unaffected
            Err(_) => {}
        }
        // Like any other download, a torrent moves only while Atlas is open: without a connection
        // the jobs pause at once and, with `idle`, the helper stops after it (the jobs stay in its
        // state folder and continue at the next start).
        let busy_now = conns.load(Ordering::SeqCst) > 0;
        if busy_now {
            quiet_since = Instant::now();
        } else {
            if !away {
                engine.pause_for_absence().await;
                away = true;
            }
            if idle.is_some_and(|idle| quiet_since.elapsed() >= idle) {
                engine.stop().await;
                return;
            }
        }
        if engine.active().await {
            working_since = Instant::now();
        } else if idle.is_some() && busy_now && working_since.elapsed() >= NO_WORK {
            engine.stop().await;
            return;
        }
    }
}

async fn read_line(
    r: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
    buf: &mut Vec<u8>,
) -> Option<()> {
    buf.clear();
    let n = r
        .take(proto::MAX_LINE as u64)
        .read_until(b'\n', buf)
        .await
        .ok()?;
    if n == 0 || buf.last() != Some(&b'\n') {
        return None; // closed, or a line longer than the limit
    }
    Some(())
}

async fn serve_one(
    s: UnixStream,
    engine: Arc<Engine>,
    mut rows: broadcast::Receiver<Event>,
    who: Peer,
    peers: &Peers,
) {
    let (rd, mut wr) = s.into_split();
    let (out, mut out_rx) = mpsc::channel::<Event>(64);
    let writer = tokio::spawn(async move {
        while let Some(ev) = out_rx.recv().await {
            if wr
                .write_all(proto::event_line(&ev).as_bytes())
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let mut r = BufReader::new(rd);
    let mut buf = Vec::new();
    // hello first
    let hello = match read_line(&mut r, &mut buf).await {
        Some(()) => std::str::from_utf8(&buf)
            .ok()
            .and_then(|l| proto::parse_request(l).ok()),
        None => None,
    };
    let ok = match &hello {
        Some(Request::Hello {
            version: proto::VERSION,
            token,
        }) => who == Peer::Browser || peers.token_ok(token.as_deref()),
        _ => false,
    };
    if !ok {
        let _ = out
            .send(Event::Error {
                code: ErrorCode::BadRequest,
                text: ErrorCode::BadRequest.text().into(),
            })
            .await;
        drop(out);
        let _ = writer.await;
        return;
    }
    let _ = out
        .send(Event::Hello {
            version: proto::VERSION,
            sharing: engine.sharing(),
        })
        .await;
    for j in engine.jobs().await {
        let _ = out.send(Event::Job(j)).await;
    }
    let fwd_out = out.clone();
    let fwd = tokio::spawn(async move {
        loop {
            match rows.recv().await {
                Ok(ev) => {
                    if fwd_out.send(ev).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    while read_line(&mut r, &mut buf).await.is_some() {
        let ev = match std::str::from_utf8(&buf)
            .map_err(|_| ErrorCode::BadRequest)
            .and_then(proto::parse_request)
        {
            Ok(req) => handle(&engine, req).await,
            Err(c) => vec![Event::Error {
                code: c,
                text: c.text().into(),
            }],
        };
        for e in ev {
            if out.send(e).await.is_err() {
                break;
            }
        }
    }
    fwd.abort();
    drop(out);
    let _ = writer.await;
}

fn err(c: ErrorCode) -> Vec<Event> {
    vec![Event::Error {
        code: c,
        text: c.text().into(),
    }]
}

async fn handle(engine: &Arc<Engine>, req: Request) -> Vec<Event> {
    match req {
        Request::Hello { .. } => err(ErrorCode::BadRequest),
        Request::AddMagnet { magnet } => {
            match crate::magnet::parse_with(&magnet, engine.local_peers()) {
                None => err(ErrorCode::BadMagnet),
                Some(m) => match engine.add_magnet(m) {
                    Ok(id) => vec![Event::Added { id }],
                    Err(c) => err(c),
                },
            }
        }
        Request::AddTorrent { torrent_hex } => match proto::decode_hex(&torrent_hex) {
            Err(c) => err(c),
            Ok(b) => match engine.add_torrent(b).await {
                Ok(id) => vec![Event::Added { id }],
                Err(c) => err(c),
            },
        },
        Request::Pause { id } | Request::Resume { id } | Request::Cancel { id, .. }
            if !proto::valid_id(&id) =>
        {
            err(ErrorCode::NotFound)
        }
        Request::Pause { id } => engine.pause(&id).await.err().map(err).unwrap_or_default(),
        Request::Resume { id } => engine.resume(&id).await.err().map(err).unwrap_or_default(),
        Request::Cancel { id, delete } => match engine.cancel(&id, delete).await {
            Ok(()) => vec![Event::Gone { id }],
            Err(c) => err(c),
        },
        Request::List => engine.jobs().await.into_iter().map(Event::Job).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_must_match_exactly() {
        let p = Peers {
            exes: vec![],
            token: Some("ab12".into()),
        };
        assert!(p.token_ok(Some("ab12")));
        assert!(!p.token_ok(Some("ab13")));
        assert!(!p.token_ok(Some("ab1")));
        assert!(!p.token_ok(None));
        let none = Peers {
            exes: vec![],
            token: None,
        };
        assert!(!none.token_ok(Some("")));
    }

    #[test]
    fn the_token_file_is_read_once_and_removed() {
        let d = std::env::temp_dir().join(format!("atlas-torrent-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let t = "ab".repeat(32);
        let f = write_token_file(&d, &t, "x").unwrap();
        assert_eq!(
            std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&d).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(
            write_token_file(&d, &t, "x").is_err(),
            "never over an existing file"
        );
        assert_eq!(take_token_file(&f).as_deref(), Some(t.as_str()));
        assert!(!f.exists(), "the file is gone after the first read");
        assert_eq!(take_token_file(&f), None);
        let bad = write_token_file(&d, "short", "y").unwrap();
        assert_eq!(take_token_file(&bad), None);
        assert!(!bad.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn this_process_on_a_pair_is_checked_by_program() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        let me = std::fs::read_link("/proc/self/exe").unwrap();
        assert_eq!(
            Peers {
                exes: vec![me],
                token: None
            }
            .check(&a),
            Peer::Browser
        );
        let other = Peers {
            exes: vec!["/x".into()],
            token: Some("t".into()),
        };
        assert_eq!(other.check(&a), Peer::Refused);
    }
}
