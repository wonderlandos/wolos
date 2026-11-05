// SPDX-License-Identifier: GPL-2.0-or-later
//! The socket side: one thread per connection blocked in `read`, one thread per verb blocked in
//! wold's answer (so a share sheet that waits for the person never holds back the next request),
//! and the caller of [`serve`] blocked on a condition variable. Nothing polls and nothing ticks:
//! while the browser holds an idle connection no thread of the bridge runs. With no connection and
//! no verb running, [`serve`] returns after [`Config::idle`]; systemd starts the bridge again at the
//! next connection.
//!
//! A connection accepted in the instant the bridge decides to leave is lost: its peer reads the
//! end of the stream before any hello and must connect again (docs/modules/atlas.md).
use crate::peer::Identity;
use crate::verbs;
use crate::wold::Wold;
use shared::atlas::{
    ErrorCode, MAX_IN_FLIGHT, MAX_LINE, Reply, Request, TEXT_BUSY, TEXT_INVALID, TEXT_NOT_BROWSER,
    TEXT_TOO_MANY, VERSION,
};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long the bridge stays with nothing to do.
pub const IDLE: Duration = Duration::from_secs(30);
/// How long a new connection has to say hello, counted from accept (not from each read). The only
/// timer while connected, and only until the hello.
pub const HELLO_WAIT: Duration = Duration::from_secs(5);
/// Connections open at once; the next is refused with [`ErrorCode::Busy`].
pub const MAX_CONNECTIONS: usize = 16;
/// A pause after a failed accept (out of descriptors or memory) instead of spinning.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// A browser that does not read its replies is not waited for longer.
const WRITE_WAIT: Duration = Duration::from_secs(5);
/// Connection and verb threads only parse a line and wait; a small stack keeps the bridge small.
const STACK: usize = 256 * 1024;

/// Where the bridge writes what the person does not read (the journal, in the service).
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// What a bridge needs to run.
pub struct Config {
    /// The browser's executable, canonical ([`shared::atlas::BROWSER`] in the service).
    pub browser: PathBuf,
    /// Where portal calls go.
    pub wold: Arc<dyn Wold>,
    /// The fallback opener ([`verbs::OPENER`] in the service).
    pub opener: Vec<String>,
    /// How long it stays with nothing to do.
    pub idle: Duration,
    /// Reasons that belong in the journal.
    pub log: Log,
}

/// Connections and verbs running, and a count of every start (so a start and an end inside one
/// idle wait still count as activity).
#[derive(Default)]
struct Live {
    busy: usize,
    starts: u64,
    /// Connections open (a part of `busy`).
    conns: usize,
}

type Shared = Arc<(Mutex<Live>, Condvar)>;

/// Marks a connection or a verb as running for as long as it lives.
struct Hold(Shared, bool);

impl Hold {
    /// A verb running.
    fn new(live: &Shared) -> Self {
        let mut l = live.0.lock().unwrap_or_else(PoisonError::into_inner);
        l.busy += 1;
        l.starts += 1;
        // Ends `serve`'s idle timer now, so it does not wake once more when that timer runs out.
        live.1.notify_all();
        Self(live.clone(), false)
    }

    /// A connection open, or None when [`MAX_CONNECTIONS`] are.
    fn connection(live: &Shared) -> Option<Self> {
        let mut l = live.0.lock().unwrap_or_else(PoisonError::into_inner);
        if l.conns >= MAX_CONNECTIONS {
            return None;
        }
        l.conns += 1;
        l.busy += 1;
        l.starts += 1;
        live.1.notify_all();
        Some(Self(live.clone(), true))
    }
}

// Releases the running mark and wakes `serve`.
impl Drop for Hold {
    fn drop(&mut self) {
        let mut l = self.0.0.lock().unwrap_or_else(PoisonError::into_inner);
        l.busy -= 1;
        if self.1 {
            l.conns -= 1;
        }
        self.0.1.notify_all();
    }
}

/// Serve `listener` until the bridge has been idle for `config.idle`. Fails only if the accept
/// thread cannot start.
pub fn serve(listener: UnixListener, config: Config) -> std::io::Result<()> {
    let config = Arc::new(config);
    let live: Shared = Arc::default();
    {
        let (live, config) = (live.clone(), config.clone());
        std::thread::Builder::new()
            .name("accept".into())
            .stack_size(STACK)
            .spawn(move || accept(&listener, &config, &live))?;
    }
    let (m, cv) = &*live;
    let mut l = m.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if l.busy > 0 {
            l = cv
                .wait_while(l, |l| l.busy > 0)
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        let starts = l.starts;
        let (g, waited) = cv
            .wait_timeout_while(l, config.idle, |l| l.busy == 0 && l.starts == starts)
            .unwrap_or_else(PoisonError::into_inner);
        l = g;
        if waited.timed_out() && l.busy == 0 && l.starts == starts {
            return Ok(());
        }
    }
}

fn accept(listener: &UnixListener, config: &Arc<Config>, live: &Shared) {
    for s in listener.incoming() {
        let Ok(s) = s else {
            std::thread::sleep(ACCEPT_BACKOFF);
            continue;
        };
        let Some(hold) = Hold::connection(live) else {
            let _ = s.set_write_timeout(Some(WRITE_WAIT));
            let _ = (&s).write_all(
                Reply::error(0, ErrorCode::Busy, TEXT_TOO_MANY)
                    .encode()
                    .as_bytes(),
            );
            continue;
        };
        let (config, live) = (config.clone(), live.clone());
        let _ = std::thread::Builder::new()
            .name("connection".into())
            .stack_size(STACK)
            .spawn(move || {
                connection(s, &config, &live);
                drop(hold);
            });
    }
}

enum Line {
    End,
    TooLong,
    Got(Vec<u8>),
}

fn read_line(r: &mut impl BufRead) -> Line {
    let mut buf = Vec::new();
    match r.take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf) {
        Ok(0) | Err(_) => Line::End,
        Ok(_) if buf.len() > MAX_LINE => Line::TooLong,
        Ok(_) => Line::Got(buf),
    }
}

/// The first line, which must be complete before `deadline` (counted from accept): the wait is
/// cut down before each read, so a peer that sends a byte now and then does not extend it.
fn read_hello(r: &mut BufReader<UnixStream>, deadline: Instant) -> Line {
    let mut buf = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || r.get_ref().set_read_timeout(Some(left)).is_err() {
            return Line::End;
        }
        let chunk = match r.fill_buf() {
            Ok(c) if !c.is_empty() => c,
            _ => return Line::End,
        };
        let (take, whole) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        buf.extend_from_slice(&chunk[..take]);
        r.consume(take);
        if buf.len() > MAX_LINE {
            return Line::TooLong;
        }
        if whole {
            return Line::Got(buf);
        }
    }
}

type Writer = Arc<Mutex<UnixStream>>;

/// Write one reply. A write that fails or times out (the browser does not read) closes the
/// connection: a half-written line must not be followed by another.
fn send(w: &Writer, r: &Reply) {
    let mut s = w.lock().unwrap_or_else(PoisonError::into_inner);
    if s.write_all(r.encode().as_bytes()).is_err() {
        let _ = s.shutdown(Shutdown::Both);
    }
}

/// Answer, then close: the peer is not the browser, or the stream cannot be read on.
fn end(w: &Writer, r: &Reply) {
    send(w, r);
    let _ = w
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .shutdown(Shutdown::Both);
}

fn not_browser() -> Reply {
    Reply::error(0, ErrorCode::NotBrowser, TEXT_NOT_BROWSER)
}

fn connection(s: UnixStream, config: &Arc<Config>, live: &Shared) {
    let Some(who) = Identity::capture(&s) else {
        return;
    };
    let Ok(w) = s.try_clone() else {
        return;
    };
    let _ = w.set_write_timeout(Some(WRITE_WAIT));
    let w: Writer = Arc::new(Mutex::new(w));
    let mut r = BufReader::new(s);
    let hello = match read_hello(&mut r, Instant::now() + HELLO_WAIT) {
        Line::Got(l) => Request::decode(&l).ok(),
        _ => None,
    };
    let version = match hello {
        Some(Request::Hello { version, pid }) if pid == who.pid && who.is(&config.browser) => {
            version.min(VERSION)
        }
        _ => return end(&w, &not_browser()),
    };
    send(&w, &Reply::Hello { version });
    // From here every wait is a plain blocking read.
    let _ = r.get_ref().set_read_timeout(None);
    let waiting: Arc<Mutex<HashSet<u32>>> = Arc::default();
    loop {
        let line = match read_line(&mut r) {
            Line::End => return,
            Line::TooLong => {
                return end(
                    &w,
                    &Reply::error(0, ErrorCode::InvalidRequest, TEXT_INVALID),
                );
            }
            Line::Got(l) => l,
        };
        let req = match Request::decode(&line) {
            Ok(r) => r,
            Err(reply) => {
                send(&w, &reply);
                continue;
            }
        };
        let (Some(id), Some((verb, path))) = (req.id(), req.verb()) else {
            // A second hello.
            send(
                &w,
                &Reply::error(0, ErrorCode::InvalidRequest, TEXT_INVALID),
            );
            continue;
        };
        // A browser that exec()ed into another program since the hello is not the browser.
        if !who.still() {
            return end(&w, &not_browser());
        }
        {
            let mut set = waiting.lock().unwrap_or_else(PoisonError::into_inner);
            if set.contains(&id) {
                send(
                    &w,
                    &Reply::error(id, ErrorCode::InvalidRequest, TEXT_INVALID),
                );
                continue;
            }
            if set.len() >= MAX_IN_FLIGHT {
                send(&w, &Reply::error(id, ErrorCode::Busy, TEXT_BUSY));
                continue;
            }
            set.insert(id);
        }
        let hold = Hold::new(live);
        let (config, w2, waiting2) = (config.clone(), w.clone(), waiting.clone());
        let path = path.to_owned();
        let started = std::thread::Builder::new()
            .name("verb".into())
            .stack_size(STACK)
            .spawn(move || {
                let reply =
                    verbs::run(id, verb, &path, &*config.wold, &config.opener, &*config.log);
                waiting2
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id);
                send(&w2, &reply);
                drop(hold);
            });
        if started.is_err() {
            waiting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            send(&w, &Reply::error(id, ErrorCode::Busy, TEXT_BUSY));
        }
    }
}
