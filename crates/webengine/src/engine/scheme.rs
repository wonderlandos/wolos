// SPDX-License-Identifier: BSD-2-Clause
//! WWE_1.2 (ADR 0052), the plain-Rust half: a URL scheme the host serves itself. The engine library
//! registers one scheme, [`SCHEME`] (Federate Network addresses), with the engine's protocol table
//! when the engine is made; the host fills the [`Slot`] with `wwe_engine_set_scheme`. A load runs
//! the host's blocking `load` on one of the slot's worker threads and the engine awaits a
//! [`Pending`], so no engine thread waits on the network. At most [`MAX_IN_FLIGHT`] loads run at
//! once per engine; the rest wait in order, at most [`MAX_QUEUED`], and a load beyond that is a 503
//! at once, so a page cannot make the process start a thread per subresource. Only std: `tests/engine_scheme.rs` includes this file and tests it
//! in the WolOS workspace, without the engine.
use std::collections::VecDeque;
use std::ffi::{c_char, c_void};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

/// The scheme the host serves.
pub const SCHEME: &str = "fed";
/// Room for a content type.
pub const MIME_CAP: usize = 256;
/// Room for a mapped URL.
pub const URL_CAP: usize = 4096;
/// Loads running at once per engine.
pub const MAX_IN_FLIGHT: usize = 8;
/// Loads waiting per engine; one more is refused.
pub const MAX_QUEUED: usize = 256;
/// `kind` of a load: a document (page or frame) or anything it loads (header: WWE_LOAD_*).
pub const LOAD_NAVIGATION: u32 = 0;
pub const LOAD_RESOURCE: u32 = 1;

// SAFETY: `wwe_body_fn`: the library's sink for the body, called by the host during `load` only.
pub type BodyFn = unsafe extern "C" fn(*mut c_void, *const u8, usize);
// SAFETY: `wwe_scheme_load_fn`: the host's answer for one URL; any thread, may block.
pub type LoadFn = unsafe extern "C" fn(
    *mut c_void,
    *const u8,
    usize,
    u32,
    *mut c_char,
    usize,
    BodyFn,
    *mut c_void,
) -> u32;
// SAFETY: `wwe_scheme_map_fn`: the host's `fed://` spelling of an http(s) URL; must not block.
pub type MapFn = unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut c_char, usize) -> isize;

/// What the host gave `wwe_engine_set_scheme`.
#[derive(Clone, Copy)]
pub struct Host {
    pub load: LoadFn,
    pub map: Option<MapFn>,
    /// The host's context, as an address (it crosses threads; the host promised that).
    pub ctx: usize,
}

/// One answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub mime: String,
    pub body: Vec<u8>,
}

impl Reply {
    fn text(status: u16, what: &str) -> Reply {
        Reply {
            status,
            mime: "text/plain; charset=utf-8".into(),
            body: what.as_bytes().to_vec(),
        }
    }
}

type Finish = Box<dyn FnOnce(Reply) + Send>;

struct Job {
    url: String,
    kind: u32,
    finish: Finish,
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<Job>,
    running: usize,
}

/// The host's scheme, shared by the protocol handler's loads; empty until the host sets it,
/// revoked when the engine is freed (a load in progress is waited for, none starts after).
pub struct Slot {
    host: RwLock<Option<Host>>,
    queue: Mutex<Queue>,
    max_in_flight: usize,
    max_queued: usize,
}

impl Default for Slot {
    fn default() -> Slot {
        Slot::with_limits(MAX_IN_FLIGHT, MAX_QUEUED)
    }
}

// SAFETY: the body sink: `sink` is the `Vec<u8>` `Slot::load` passed, alive for the call.
unsafe extern "C" fn put(sink: *mut c_void, bytes: *const u8, len: usize) {
    if sink.is_null() || bytes.is_null() || len == 0 {
        return;
    }
    // SAFETY: contract above; the host's bytes are readable for `len` during the call.
    #[allow(clippy::disallowed_methods)] // the boundary slice clippy.toml names
    let (out, b) = unsafe {
        (
            &mut *sink.cast::<Vec<u8>>(),
            std::slice::from_raw_parts(bytes, len),
        )
    };
    out.extend_from_slice(b);
}

fn c_text(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| c.to_ne_bytes()[0])
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

impl Slot {
    /// A slot running at most `in_flight` loads, with at most `queued` waiting.
    pub fn with_limits(in_flight: usize, queued: usize) -> Slot {
        Slot {
            host: RwLock::new(None),
            queue: Mutex::new(Queue::default()),
            max_in_flight: in_flight.max(1),
            max_queued: queued,
        }
    }

    pub fn set(&self, host: Host) {
        *self.host.write().unwrap_or_else(PoisonError::into_inner) = Some(host);
    }

    pub fn revoke(&self) {
        *self.host.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// The host's answer for `url`, a load of `kind` (blocks as long as the host does).
    pub fn load(&self, url: &str, kind: u32) -> Reply {
        let held = self.host.read().unwrap_or_else(PoisonError::into_inner);
        let Some(h) = *held else {
            return Reply::text(503, "This program does not serve fed:// addresses.");
        };
        let mut mime: [c_char; MIME_CAP] = [0; MIME_CAP];
        let mut body: Vec<u8> = Vec::new();
        // SAFETY: the host's contract (`wwe_scheme_load_fn`): url is readable for its length, mime is
        // writable for MIME_CAP, `put` with `body` is the sink, ctx is the host's.
        let status = unsafe {
            (h.load)(
                std::ptr::with_exposed_provenance_mut(h.ctx),
                url.as_ptr(),
                url.len(),
                kind,
                mime.as_mut_ptr(),
                MIME_CAP,
                put,
                (&raw mut body).cast(),
            )
        };
        drop(held);
        match u16::try_from(status) {
            Ok(s @ 100..=599) => Reply {
                status: s,
                mime: c_text(&mime),
                body,
            },
            _ => Reply::text(502, "The fed:// handler gave no answer."),
        }
    }

    /// The host's `fed://` spelling of an http(s) `url`, or `None` (stays on the web).
    pub fn map(&self, url: &str) -> Option<String> {
        // http and https only: the engine serialises schemes in lower case.
        if !url
            .split_once("://")
            .is_some_and(|(s, _)| s == "http" || s == "https")
        {
            return None;
        }
        let held = self.host.read().unwrap_or_else(PoisonError::into_inner);
        let map = (*held)?.map?;
        let ctx = (*held)?.ctx;
        let mut out: Vec<c_char> = vec![0; URL_CAP];
        // SAFETY: the host's contract (`wwe_scheme_map_fn`): url readable, out writable for URL_CAP.
        let n = unsafe {
            map(
                std::ptr::with_exposed_provenance_mut(ctx),
                url.as_ptr(),
                url.len(),
                out.as_mut_ptr(),
                URL_CAP,
            )
        };
        let n = usize::try_from(n).ok().filter(|n| *n < URL_CAP)?;
        let s = c_text(&out[..=n]);
        (s.len() == n && s.starts_with("fed://")).then_some(s)
    }
}

struct Shared {
    reply: Option<Reply>,
    waker: Option<Waker>,
}

/// A load in progress on its own thread: ready with the host's [`Reply`].
pub struct Pending(Arc<Mutex<Shared>>);

impl Future for Pending {
    type Output = Reply;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Reply> {
        let mut s = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match s.reply.take() {
            Some(r) => Poll::Ready(r),
            None => {
                s.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl Pending {
    /// `waker` is woken when the reply is in (at once when it already is). For the engine's spin,
    /// which has no task to poll: it wakes the host, whose next spin takes the reply ([`Outbox`]).
    pub fn watch(&self, waker: &Waker) {
        let mut s = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if s.reply.is_some() {
            waker.wake_by_ref();
        } else {
            s.waker = Some(waker.clone());
        }
    }

    /// The reply if the load is over (the reply is taken once), else `None`. For a caller that
    /// cannot await (the engine thread's spin: [`Outbox`]).
    pub fn take(&self) -> Option<Reply> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reply
            .take()
    }
}

/// Loads in progress that are answered from the engine's spin: each holds what must answer it
/// (`T`, the intercepted web load) beside its [`Pending`]. A fed-TLD host's http(s) subresource or
/// frame is held here, never sent to the network, until the `fed:` chain has answered; the engine
/// thread does not wait on it (see `lib.rs`, `load_web_resource`).
pub struct Outbox<T> {
    held: Vec<(T, Pending)>,
}

impl<T> Default for Outbox<T> {
    fn default() -> Outbox<T> {
        Outbox { held: Vec::new() }
    }
}

impl<T> Outbox<T> {
    pub fn add(&mut self, what: T, pending: Pending) {
        self.held.push((what, pending));
    }

    #[allow(dead_code)] // the library does not ask; the tests do
    pub fn len(&self) -> usize {
        self.held.len()
    }

    #[allow(dead_code)] // as `len`
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// The loads whose reply is in, with it; the others stay.
    pub fn ready(&mut self) -> Vec<(T, Reply)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.held.len() {
            match self.held[i].1.take() {
                Some(r) => out.push((self.held.swap_remove(i).0, r)),
                None => i += 1,
            }
        }
        out
    }

    /// Everything still held, without a reply: the engine is going away.
    pub fn abandon(&mut self) -> Vec<T> {
        self.held.drain(..).map(|(t, _)| t).collect()
    }
}

/// The `kind` of a load by the engine's request destination (its `Debug` name: `Document`,
/// `Frame`, `IFrame` are documents, anything else a subresource, which has the smaller cap).
pub fn load_kind(destination: &str) -> u32 {
    match destination {
        "Document" | "Frame" | "IFrame" => LOAD_NAVIGATION,
        _ => LOAD_RESOURCE,
    }
}

/// Whether a frame or subresource request to a Federate-TLD host (`http(s)://x.<fedTLD>`) may be
/// answered from the chain. A frame (`kind` is [`LOAD_NAVIGATION`]) always may: it becomes a
/// Federate document. Everything else (images, scripts, `fetch`, XHR, workers) only when the page
/// that asks is itself a Federate page, `asked_by_fed` ([`referrer_is_fed`]); a web or `file:` page
/// must not probe or fingerprint Federate names, and is refused.
pub fn fed_request_allowed(kind: u32, asked_by_fed: bool) -> bool {
    kind == LOAD_NAVIGATION || asked_by_fed
}

/// Whether `referrer` (the URL of the page that makes a request) is a Federate page: a `fed://`
/// URL, or the http(s) spelling of a Federate name (a frame the chain answered under that URL),
/// which `map` (the host's mapping) recognises. No referrer is no Federate page.
pub fn referrer_is_fed(referrer: Option<&str>, map: impl Fn(&str) -> Option<String>) -> bool {
    referrer.is_some_and(|r| r.starts_with("fed://") || map(r).is_some())
}

/// The page a refused Federate-host http(s) load becomes: it says where the address is.
pub fn refused_fed_body(url: &str) -> String {
    let url: String = url
        .chars()
        .filter(|c| c.is_ascii_graphic())
        .map(|c| match c {
            '<' | '>' | '&' | '"' | '\'' => '_',
            c => c,
        })
        .collect();
    format!(
        "<!doctype html><meta charset=utf-8><title>Federate</title><h1>Not opened</h1>\
         <p>{url} is a Federate name. On WolOS Federate names are fed:// only: they are never \
         fetched over http(s) and never looked up in DNS. This program may not open it.</p>"
    )
}

/// Queues `url`'s load (`kind` as the header's WWE_LOAD_*) on `slot`'s workers. The reply always
/// comes: a full queue is a 503 at once, a panic in the load a 502, no worker thread a 503.
pub fn spawn(slot: Arc<Slot>, url: String, kind: u32) -> Pending {
    let shared = Arc::new(Mutex::new(Shared {
        reply: None,
        waker: None,
    }));
    let done = shared.clone();
    let finish: Finish = Box::new(move |r: Reply| {
        let mut s = done.lock().unwrap_or_else(PoisonError::into_inner);
        s.reply = Some(r);
        if let Some(w) = s.waker.take() {
            w.wake();
        }
    });
    let start = {
        let mut q = slot.queue.lock().unwrap_or_else(PoisonError::into_inner);
        if q.waiting.len() >= slot.max_queued {
            drop(q);
            finish(Reply::text(
                503,
                "Too many fed:// loads are waiting; load the page again.",
            ));
            return Pending(shared);
        }
        q.waiting.push_back(Job { url, kind, finish });
        let start = q.running < slot.max_in_flight;
        if start {
            q.running += 1;
        }
        start
    };
    if start {
        let worker = slot.clone();
        let made = std::thread::Builder::new()
            .name("wwe-scheme".into())
            .spawn(move || work(&worker));
        if made.is_err() {
            // No worker: this one's place is given back, and with none running the waiting
            // loads are answered rather than left to wait forever.
            let orphans = {
                let mut q = slot.queue.lock().unwrap_or_else(PoisonError::into_inner);
                q.running -= 1;
                if q.running == 0 {
                    std::mem::take(&mut q.waiting)
                } else {
                    VecDeque::new()
                }
            };
            for j in orphans {
                (j.finish)(Reply::text(503, "No thread for the fed:// load."));
            }
        }
    }
    Pending(shared)
}

/// One worker: takes the waiting loads in order until there are none, then ends.
fn work(slot: &Slot) {
    loop {
        let job = {
            let mut q = slot.queue.lock().unwrap_or_else(PoisonError::into_inner);
            match q.waiting.pop_front() {
                Some(j) => j,
                None => {
                    q.running -= 1;
                    return;
                }
            }
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            slot.load(&job.url, job.kind)
        }))
        .unwrap_or_else(|_| Reply::text(502, "The fed:// handler failed."));
        (job.finish)(r);
    }
}

/// What a mapped page load does next ([`Retry::step`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Not yet time for another try.
    Wait,
    /// Ask the engine to load the page now.
    Try,
    /// Over: the `fed:` handler was asked for a document since the last try (the load took), or the
    /// tries ran out (the page stays as it was, as with any refused load).
    Done,
}

/// The retry of a mapped page load (`lib.rs`, `start_deferred`): the engine drops an embedder load
/// while the navigation the 204 ended is still pending, and nothing reports when that is over, so
/// the load is tried every `every` until a document load reaches the `fed:` handler, at most
/// `max_tries` times. The pace and the count are arguments so a test can use a short deadline.
#[derive(Debug, Clone)]
pub struct Retry {
    every: Duration,
    max_tries: u32,
    tries: u32,
    next: Instant,
    seen: u64,
}

impl Retry {
    /// A retry that may try at `now`; `docs` is the count of documents the handler was asked for.
    pub fn new(every: Duration, max_tries: u32, docs: u64, now: Instant) -> Retry {
        Retry {
            every,
            max_tries,
            tries: 0,
            next: now,
            seen: docs,
        }
    }

    /// The next move at `now`, given the handler's document count `docs`.
    pub fn step(&mut self, docs: u64, now: Instant) -> Step {
        if docs != self.seen || self.tries >= self.max_tries {
            return Step::Done;
        }
        if now < self.next {
            return Step::Wait;
        }
        self.tries += 1;
        self.seen = docs;
        self.next = now + self.every;
        Step::Try
    }
}
