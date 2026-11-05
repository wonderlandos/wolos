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

const NOT_BROWSER: &str =
    r#"{"reply":"error","id":0,"code":"not-browser","text":"Only Atlas can use this service."}"#;

fn hello_line() -> String {
    format!(
        r#"{{"op":"hello","version":2,"pid":{}}}"#,
        std::process::id()
    )
}

/// QA-A9-SEC-1 (a): the hello was written before the bridge accepted, by a process that then became
/// (or is) the browser. The bridge speaks first now, so bytes that precede its greeting are not an
/// echo of it.
#[test]
fn a_hello_written_before_the_greeting_is_refused() {
    let (sock, _wold, f) = setup("prewritten");
    let mut b = Browser::connect_raw(&sock);
    b.send(&hello_line());
    b.send(&verb("trash", 1, &f));
    // The bridge's greeting may come first; nothing may be answered as a hello or a verb. (It can
    // reset the connection: it closes with the pre-written bytes unread.)
    let said = b.read_all();
    assert!(
        said.iter()
            .all(|l| l.starts_with("hello ") || l == NOT_BROWSER),
        "{said:?}"
    );
    assert!(f.exists(), "nothing was trashed");
}

/// QA-A9-SEC-1 (b): the browser connects and hands the socket to a child of the same account (a
/// process holding the descriptor). The child sends a hello that names the browser's pid, then the
/// Trash, either at once or after echoing the greeting itself. The echo carries the child's pid,
/// not the pid the kernel recorded at connect; the hello's own `pid` field is only a claim.
fn a_child_holds_the_socket(name: &str, echo: bool) -> Vec<String> {
    use std::os::fd::OwnedFd;
    use std::process::{Command, Stdio};
    let (sock, wold, f) = setup(name);
    wold.answer("file.trash", r#"{"ok":true,"data":{"name":"report.pdf"}}"#);
    let b = Browser::connect_raw(&sock);
    let input = OwnedFd::from(b.w.try_clone().unwrap());
    let output = OwnedFd::from(b.w.try_clone().unwrap());
    let greeting = if echo {
        "read l; printf '%s\\n' \"$l\"; "
    } else {
        ""
    };
    let script = format!(
        "{greeting}printf '%s\\n' '{}' '{}'; sleep 1",
        hello_line(),
        verb("trash", 1, &f)
    );
    let mut child = Command::new("/bin/sh")
        .args(["-c", &script])
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output))
        .spawn()
        .unwrap();
    // The child holds the descriptor, and so it reads the greeting; the parent only listens.
    child.wait().unwrap();
    let mut b = b;
    let said = b.read_all();
    assert!(
        said.iter()
            .all(|l| l == NOT_BROWSER || l.starts_with("hello ")),
        "{said:?}"
    );
    assert!(said.iter().any(|l| l == NOT_BROWSER), "{said:?}");
    assert!(
        wold.asked.lock().unwrap().is_empty(),
        "wold was asked for the Trash in the browser's name"
    );
    said
}

#[test]
fn a_hello_from_another_process_than_the_connector_is_refused() {
    let said = a_child_holds_the_socket("child-holds-fd", false);
    assert!(said.iter().any(|l| l == NOT_BROWSER), "{said:?}");
}

#[test]
fn a_greeting_echoed_by_another_process_than_the_connector_is_refused() {
    let said = a_child_holds_the_socket("child-echoes", true);
    assert!(said.iter().any(|l| l == NOT_BROWSER), "{said:?}");
}

/// The greeting is part of the same deadline as the hello: a peer that never answers it is closed
/// at the hello wait.
#[test]
fn a_peer_that_never_answers_the_greeting_is_closed() {
    let (sock, _wold, _f) = setup("no-echo");
    let mut b = Browser::connect_raw(&sock);
    let t = Instant::now();
    let first = b.read().unwrap();
    assert!(first.starts_with("hello "), "{first}");
    assert_eq!(b.read().as_deref(), Some(NOT_BROWSER));
    assert_eq!(b.read(), None);
    assert!(t.elapsed() >= Duration::from_secs(4), "{:?}", t.elapsed());
    assert!(t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
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
    // Refused before the greeting: nothing to answer.
    let mut extra = Browser::connect_raw(&sock);
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
