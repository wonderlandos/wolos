// SPDX-License-Identifier: BSD-2-Clause
//! The engine library's `fed:` bridge (`src/engine/scheme.rs`, WWE_1.2, ADR 0052), without the
//! engine: no host is a 503 page, the host's status, type and body come through whole, a load on
//! its own thread wakes its task with the reply, only http(s) URLs are offered to `map`, a bad
//! status is a 502, and nothing reaches the host once the slot is revoked. (A host must not
//! unwind: across `extern "C"` that aborts, so the WolOS host catches its own panics, `fed::serve`.)
#[path = "../src/engine/scheme.rs"]
mod scheme;

use std::ffi::{c_char, c_void};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

fn write_c(s: &str, out: *mut c_char, cap: usize) {
    let n = s.len().min(cap - 1);
    // SAFETY: the library's buffer has cap bytes; n + 1 <= cap.
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), out.cast::<u8>(), n);
        out.add(n).write(0);
    }
}

// SAFETY: as `wwe_scheme_load_fn`; this stand-in host answers by the URL.
unsafe extern "C" fn host_load(
    _ctx: *mut c_void,
    url: *const u8,
    len: usize,
    kind: u32,
    mime: *mut c_char,
    cap: usize,
    body: scheme::BodyFn,
    sink: *mut c_void,
) -> u32 {
    #[allow(clippy::disallowed_methods)] // the host's side of the boundary, as in src/fed.rs
    // SAFETY: the library passes len readable bytes.
    let url = std::str::from_utf8(unsafe { std::slice::from_raw_parts(url, len) }).unwrap();
    match url {
        "fed://odd.fed/" => 7,
        "fed://kind.fed/" => {
            let k = kind.to_string();
            // SAFETY: the library's sink, during this call.
            unsafe { body(sink, k.as_ptr(), k.len()) };
            200
        }
        u if u.starts_with("fed://slow.fed/") => {
            slow(u);
            200
        }
        "fed://who.fed/" => {
            let name = std::thread::current().name().unwrap_or("").to_owned();
            // SAFETY: the library's sink, during this call.
            unsafe { body(sink, name.as_ptr(), name.len()) };
            200
        }
        _ => {
            write_c("text/html; charset=utf-8", mime, cap);
            // SAFETY: the library's sink, during this call; twice, as a host may.
            unsafe {
                body(sink, b"<p>".as_ptr(), 3);
                body(sink, url.as_ptr(), url.len());
            }
            200
        }
    }
}

// SAFETY: as `wwe_scheme_map_fn`.
unsafe extern "C" fn host_map(
    _ctx: *mut c_void,
    url: *const u8,
    len: usize,
    out: *mut c_char,
    cap: usize,
) -> isize {
    #[allow(clippy::disallowed_methods)] // as in host_load
    // SAFETY: as above.
    let url = std::str::from_utf8(unsafe { std::slice::from_raw_parts(url, len) }).unwrap();
    if url != "http://home.fed/" {
        return -1;
    }
    write_c("fed://home.fed", out, cap);
    14
}

trait LoadNav {
    fn load_nav(&self, url: &str) -> scheme::Reply;
}
impl LoadNav for scheme::Slot {
    fn load_nav(&self, url: &str) -> scheme::Reply {
        self.load(url, scheme::LOAD_NAVIGATION)
    }
}

/// The slow loads: how many run now, the most seen at once, the order they started in, and a
/// gate that holds them until opened.
struct Slow {
    now: usize,
    most: usize,
    started: Vec<String>,
    open: bool,
}
static SLOW: Mutex<Slow> = Mutex::new(Slow {
    now: 0,
    most: 0,
    started: Vec::new(),
    open: false,
});
static SLOW_CV: std::sync::Condvar = std::sync::Condvar::new();

fn slow(url: &str) {
    let mut s = SLOW.lock().unwrap();
    s.now += 1;
    s.most = s.most.max(s.now);
    s.started.push(url.to_owned());
    SLOW_CV.notify_all();
    while !s.open {
        s = SLOW_CV.wait(s).unwrap();
    }
    s.now -= 1;
}

fn wait_running(n: usize) {
    let mut s = SLOW.lock().unwrap();
    while s.now < n {
        s = SLOW_CV.wait(s).unwrap();
    }
}

fn host() -> scheme::Host {
    scheme::Host {
        load: host_load,
        map: Some(host_map),
        ctx: 0,
    }
}

struct Flag(Mutex<bool>, std::sync::Condvar);
impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
}

/// Polls `f` until ready, sleeping between wakes (what the engine's runtime does).
fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let flag = Arc::new(Flag(Mutex::new(false), std::sync::Condvar::new()));
    let waker = Waker::from(flag.clone());
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        let mut woken = flag.0.lock().unwrap();
        while !*woken {
            woken = flag.1.wait(woken).unwrap();
        }
        *woken = false;
    }
}

#[test]
fn without_a_host_a_load_is_an_honest_503() {
    let slot = scheme::Slot::default();
    let r = slot.load_nav("fed://home.fed/");
    assert_eq!(r.status, 503);
    assert!(String::from_utf8_lossy(&r.body).contains("does not serve fed://"));
    assert_eq!(slot.map("http://home.fed/"), None);
}

#[test]
fn the_hosts_answer_comes_through_whole_and_off_the_engine_thread() {
    let slot = Arc::new(scheme::Slot::default());
    slot.set(host());
    let r = slot.load_nav("fed://home.fed/");
    assert_eq!(
        (r.status, r.mime.as_str()),
        (200, "text/html; charset=utf-8")
    );
    assert_eq!(r.body, b"<p>fed://home.fed/");
    let r = block_on(scheme::spawn(slot.clone(), "fed://home.fed/a".into(), 0));
    assert_eq!(r.body, b"<p>fed://home.fed/a");
    let r = block_on(scheme::spawn(slot.clone(), "fed://who.fed/".into(), 0));
    assert_eq!(
        r.body, b"wwe-scheme",
        "the host ran on the load's own thread"
    );
    // Several at once.
    let all: Vec<_> = (0..8)
        .map(|i| scheme::spawn(slot.clone(), format!("fed://home.fed/{i}"), 1))
        .collect();
    for (i, p) in all.into_iter().enumerate() {
        assert_eq!(
            block_on(p).body,
            format!("<p>fed://home.fed/{i}").into_bytes()
        );
    }
}

#[test]
fn a_status_outside_http_is_a_502() {
    assert_eq!(scheme::SCHEME, "fed");
    let slot = Arc::new(scheme::Slot::default());
    slot.set(host());
    assert_eq!(slot.load_nav("fed://odd.fed/").status, 502);
    assert_eq!(
        block_on(scheme::spawn(slot.clone(), "fed://odd.fed/".into(), 0)).status,
        502
    );
}

#[test]
fn only_http_urls_are_offered_for_mapping() {
    let slot = scheme::Slot::default();
    slot.set(host());
    assert_eq!(
        slot.map("http://home.fed/").as_deref(),
        Some("fed://home.fed")
    );
    assert_eq!(slot.map("http://example.com/"), None);
    assert_eq!(slot.map("file:///home.fed/"), None);
    slot.set(scheme::Host {
        map: None,
        ..host()
    });
    assert_eq!(slot.map("http://home.fed/"), None, "a host without map");
}

#[test]
fn a_revoked_slot_calls_the_host_no_more() {
    let slot = scheme::Slot::default();
    slot.set(host());
    assert_eq!(slot.load_nav("fed://home.fed/").status, 200);
    slot.revoke();
    assert_eq!(slot.load_nav("fed://home.fed/").status, 503);
    assert_eq!(slot.map("http://home.fed/"), None);
}

#[test]
fn the_kind_of_load_reaches_the_host() {
    let slot = Arc::new(scheme::Slot::default());
    slot.set(host());
    assert_eq!(
        block_on(scheme::spawn(
            slot.clone(),
            "fed://kind.fed/".into(),
            scheme::LOAD_RESOURCE
        ))
        .body,
        b"1"
    );
    assert_eq!(
        slot.load("fed://kind.fed/", scheme::LOAD_NAVIGATION).body,
        b"0"
    );
}

/// One test drives the slow loads (they share one gate): at most the cap run at once, the rest
/// start in the order they came, and a load beyond the queue is a 503 without reaching the host.
#[test]
fn loads_are_capped_queued_in_order_and_refused_beyond_the_queue() {
    assert_eq!((scheme::MAX_IN_FLIGHT, scheme::MAX_QUEUED), (8, 256));
    let slot = Arc::new(scheme::Slot::with_limits(2, 3));
    slot.set(host());
    let spawn = |i: usize| {
        scheme::spawn(
            slot.clone(),
            format!("fed://slow.fed/{i}"),
            scheme::LOAD_RESOURCE,
        )
    };
    let mut all: Vec<_> = (0..2).map(spawn).collect();
    wait_running(2);
    all.extend((2..5).map(spawn));
    // Two run, three wait: the queue is full, so the next is answered at once.
    // Answered at once, not queued: one poll is enough (a queued load would be Pending).
    let mut over = std::pin::pin!(scheme::spawn(
        slot.clone(),
        "fed://slow.fed/over".into(),
        scheme::LOAD_RESOURCE,
    ));
    let Poll::Ready(over) = over.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
        panic!("a load beyond the queue was queued");
    };
    assert_eq!(over.status, 503);
    {
        let mut s = SLOW.lock().unwrap();
        assert_eq!(s.now, 2);
        s.open = true;
        SLOW_CV.notify_all();
    }
    for p in all.drain(..) {
        assert_eq!(block_on(p).status, 200);
    }
    let s = SLOW.lock().unwrap();
    assert_eq!(s.most, 2, "never more than the cap at once");
    assert!(
        !s.started.iter().any(|u| u.ends_with("over")),
        "the refused load never ran"
    );
    let waited: Vec<&str> = s.started[2..].iter().map(String::as_str).collect();
    assert_eq!(
        waited,
        ["fed://slow.fed/2", "fed://slow.fed/3", "fed://slow.fed/4"],
        "in order"
    );
}
