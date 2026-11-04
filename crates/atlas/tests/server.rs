// SPDX-License-Identifier: GPL-2.0-or-later
//! The bridge as the browser meets it: a sheet that waits does not hold back other verbs, at most
//! four wait at once, only the browser is answered, and the bridge leaves when idle.
mod support;
use atlas::server::{Config, serve};
use atlas::wold::Socket;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use support::{Browser, FakeWold, bridge, me, scratch};

const COPIED: &str = r#"{"ok":true,"data":{"done":"copied"}}"#;

fn setup(name: &str) -> (PathBuf, FakeWold, PathBuf) {
    let dir = scratch(name);
    let f = dir.join("report.pdf");
    std::fs::write(&f, b"x").unwrap();
    let wold = FakeWold::default();
    let ws = dir.join("wold.sock");
    wold.serve(&ws);
    let sock = bridge(&dir, &ws, me(), vec![], Duration::from_secs(60));
    (sock, wold, f)
}

fn verb(op: &str, id: u32, f: &Path) -> String {
    format!(r#"{{"op":"{op}","id":{id},"path":"{}"}}"#, f.display())
}

#[test]
fn a_waiting_share_sheet_does_not_hold_back_the_trash() {
    let (sock, wold, f) = setup("concurrent");
    let (person, chooses) = mpsc::channel();
    wold.push("share.file", COPIED, Some(chooses));
    wold.answer("file.trash", r#"{"ok":true,"data":{"name":"report.pdf"}}"#);
    let mut b = Browser::connect(&sock);
    b.hello();
    b.send(&verb("share", 1, &f));
    // The share sheet is on screen; the person has not chosen.
    wold.wait_asked(1);
    b.send(&verb("trash", 2, &f));
    assert_eq!(
        b.read().as_deref(),
        Some(r#"{"reply":"ok","id":2,"outcome":"trashed"}"#)
    );
    person.send(()).unwrap();
    assert_eq!(
        b.read().as_deref(),
        Some(r#"{"reply":"ok","id":1,"outcome":"copied"}"#)
    );
}

#[test]
fn four_wait_at_once_and_an_id_is_not_reused_while_it_waits() {
    let (sock, wold, f) = setup("in-flight");
    let mut people = Vec::new();
    for _ in 0..4 {
        let (tx, rx) = mpsc::channel();
        wold.push("share.file", COPIED, Some(rx));
        people.push(tx);
    }
    let mut b = Browser::connect(&sock);
    b.hello();
    for id in 1..=4 {
        b.send(&verb("share", id, &f));
    }
    wold.wait_asked(4);
    b.send(&verb("share", 3, &f));
    assert_eq!(
        b.read().as_deref(),
        Some(
            r#"{"reply":"error","id":3,"code":"invalid-request","text":"Atlas sent a request WolOS does not understand."}"#
        )
    );
    b.send(&verb("trash", 5, &f));
    assert_eq!(
        b.read().as_deref(),
        Some(
            r#"{"reply":"error","id":5,"code":"busy","text":"Something else is waiting for you to choose. Finish it, then try again."}"#
        )
    );
    drop(people);
    let mut done: Vec<String> = (0..4).map(|_| b.read().unwrap()).collect();
    done.sort();
    let want: Vec<String> = (1..=4)
        .map(|i| format!(r#"{{"reply":"ok","id":{i},"outcome":"copied"}}"#))
        .collect();
    assert_eq!(done, want);
}

#[test]
fn another_program_is_not_the_browser() {
    let dir = scratch("not-browser");
    let sock = bridge(
        &dir,
        &dir.join("wold.sock"),
        PathBuf::from(shared::atlas::BROWSER),
        vec![],
        Duration::from_secs(60),
    );
    let mut b = Browser::connect(&sock);
    b.send(&format!(
        r#"{{"op":"hello","version":1,"pid":{}}}"#,
        std::process::id()
    ));
    assert_eq!(
        b.read().as_deref(),
        Some(
            r#"{"reply":"error","id":0,"code":"not-browser","text":"Only Atlas can use this service."}"#
        )
    );
    assert_eq!(b.read(), None);
}

#[test]
fn a_connection_that_never_says_hello_is_closed() {
    let (sock, _wold, _f) = setup("silent");
    let mut b = Browser::connect(&sock);
    let t = Instant::now();
    assert_eq!(
        b.read().as_deref(),
        Some(
            r#"{"reply":"error","id":0,"code":"not-browser","text":"Only Atlas can use this service."}"#
        )
    );
    assert_eq!(b.read(), None);
    assert!(t.elapsed() >= Duration::from_secs(4), "{:?}", t.elapsed());
}

/// A pause that is a wait on a channel nobody sends on (tests do not sleep).
fn pause(d: Duration) {
    let (_keep, never) = mpsc::channel::<()>();
    let _ = never.recv_timeout(d);
}

#[test]
fn a_peer_that_trickles_bytes_does_not_extend_the_hello_wait() {
    use std::io::Write;
    let (sock, _wold, _f) = setup("trickle");
    let mut b = Browser::connect(&sock);
    let t = Instant::now();
    let mut closed = None;
    for _ in 0..12 {
        pause(Duration::from_millis(700));
        if b.w.write_all(b" ").is_err() {
            closed = Some(t.elapsed());
            break;
        }
    }
    let closed = closed.expect("the bridge closed the connection");
    assert!(closed >= Duration::from_secs(4), "{closed:?}");
    assert!(closed < Duration::from_secs(8), "{closed:?}");
}

#[test]
fn the_seventeenth_connection_is_refused() {
    let (sock, _wold, _f) = setup("cap");
    let held: Vec<Browser> = (0..atlas::server::MAX_CONNECTIONS)
        .map(|_| {
            let mut b = Browser::connect(&sock);
            b.hello();
            b
        })
        .collect();
    let mut extra = Browser::connect(&sock);
    assert_eq!(
        extra.read().as_deref(),
        Some(
            r#"{"reply":"error","id":0,"code":"busy","text":"Atlas has too many connections open to WolOS. Close some and try again."}"#
        )
    );
    assert_eq!(extra.read(), None);
    drop(held);
}

/// Runs `serve` with a short idle time; the receiver gets a message when it returns.
fn idle_bridge(name: &str, idle: Duration) -> (PathBuf, FakeWold, PathBuf, mpsc::Receiver<()>) {
    let dir = scratch(name);
    let f = dir.join("report.pdf");
    std::fs::write(&f, b"x").unwrap();
    let wold = FakeWold::default();
    let ws = dir.join("wold.sock");
    wold.serve(&ws);
    let sock = dir.join("bridge.sock");
    let l = atlas::activation::bind(&sock).unwrap();
    let config = Config {
        browser: me(),
        wold: Arc::new(Socket::at(ws)),
        opener: vec![],
        idle,
        log: Arc::new(|_| {}),
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = serve(l, config);
        let _ = tx.send(());
    });
    (sock, wold, f, rx)
}

#[test]
fn the_bridge_leaves_when_idle_and_not_before() {
    let idle = Duration::from_millis(300);
    let (_sock, _w, _f, gone) = idle_bridge("idle-alone", idle);
    gone.recv_timeout(Duration::from_secs(5))
        .expect("leaves with no connection");

    let (sock, _w, _f, gone) = idle_bridge("idle-held", idle);
    let mut b = Browser::connect(&sock);
    b.hello();
    assert!(
        gone.recv_timeout(Duration::from_millis(1200)).is_err(),
        "stays while the browser is connected"
    );
    drop(b);
    gone.recv_timeout(Duration::from_secs(5))
        .expect("leaves after the browser went");
}

#[test]
fn a_sheet_still_on_screen_keeps_the_bridge_after_the_browser_went() {
    let idle = Duration::from_millis(300);
    let (sock, wold, f, gone) = idle_bridge("idle-sheet", idle);
    let (person, chooses) = mpsc::channel();
    wold.push("share.file", COPIED, Some(chooses));
    let mut b = Browser::connect(&sock);
    b.hello();
    b.send(&verb("share", 1, &f));
    wold.wait_asked(1);
    drop(b);
    assert!(
        gone.recv_timeout(Duration::from_millis(1200)).is_err(),
        "the person is still choosing"
    );
    person.send(()).unwrap();
    gone.recv_timeout(Duration::from_secs(5))
        .expect("leaves once the sheet is answered");
}

#[test]
fn a_line_too_long_closes_the_connection() {
    let (sock, _w, _f) = setup("too-long");
    let mut b = Browser::connect(&sock);
    b.hello();
    b.send(&"x".repeat(shared::atlas::MAX_LINE + 1));
    assert_eq!(
        b.read().as_deref(),
        Some(
            r#"{"reply":"error","id":0,"code":"invalid-request","text":"Atlas sent a request WolOS does not understand."}"#
        )
    );
    assert_eq!(b.read(), None);
}
