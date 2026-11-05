// SPDX-License-Identifier: BSD-2-Clause
//! The helper end to end on a real Unix socket, with the network's own data captured by the
//! federate crate (crates/federate/tests/vectors, 2026-11-04) as the distributor: the page the
//! browser gets, the page for a manifest whose signature was changed, the limits.
use atlas_fed::{Handler, Limits, Overlay, read_reply, request_line};
use federate::{Load, Reply, Resolver, Source};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

const ROOT: &[u8] = include_bytes!("../../federate/tests/vectors/root-zone.json");
const MANIFEST: &[u8] = include_bytes!("../../federate/tests/vectors/manifest-home.fed.json");
const INDEX: &[u8] = include_bytes!("../../federate/tests/vectors/block-home.fed-index.html");
const HOME_MANIFEST: &str = "41220e29510791e1fbbc13dbf2ba610dc9598f28a0f102f488f20610780c4c6f";
const HOME_INDEX: &str = "ddfe588ac4fe64292a561ae36aee15d32250b1bbc2b62fd8f84e09b96e254066";

/// A distributor answering from a table.
struct Table(Mutex<HashMap<String, Vec<u8>>>);

impl Source for Table {
    fn get(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let b = self
            .0
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or(format!("no {path}"))?;
        if b.len() as u64 > max {
            return Err("too large".into());
        }
        Ok(b)
    }
}

fn captured() -> Table {
    let mut m = HashMap::new();
    m.insert("/v1/root".to_owned(), ROOT.to_vec());
    m.insert(format!("/v1/manifest/{HOME_MANIFEST}"), MANIFEST.to_vec());
    m.insert(format!("/v1/block/{HOME_INDEX}"), INDEX.to_vec());
    Table(Mutex::new(m))
}

/// A fresh directory under the target dir (never a fixed /tmp path).
fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("atlas-fed-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Binds a socket in `dir` and serves `handler` on a thread; returns the socket path and the
/// server's end (Ok when `run` returned).
fn serve(dir: &Path, handler: Handler, limits: Limits) -> (PathBuf, mpsc::Receiver<bool>) {
    let path = dir.join("run").join("atlas").join("fed.sock");
    let bound = atlas_fed::bind(&path).unwrap();
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("test-server".into())
        .spawn(move || {
            let lock_held = bound;
            let ok = atlas_fed::run(lock_held.listener, handler, limits).is_ok();
            let _ = tx.send(ok);
        })
        .unwrap();
    (path, rx)
}

fn ask(path: &Path, url: &str, kind: Load) -> Reply {
    let mut s = UnixStream::connect(path).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(request_line(url, kind).as_bytes()).unwrap();
    read_reply(&mut s, 64 << 20).unwrap()
}

fn resolver_handler(source: Box<dyn Source>) -> Handler {
    let r = Arc::new(Resolver::new(source, federate::ROOT_KEY));
    Arc::new(move |url, kind| r.serve_load(url, kind))
}

#[test]
fn the_networks_home_page_comes_back_verified_over_the_socket() {
    let dir = scratch("home");
    let (path, _end) = serve(
        &dir,
        resolver_handler(Box::new(captured())),
        Limits::default(),
    );
    let r = ask(&path, "fed://home.fed", Load::Document);
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.mime, "text/html; charset=utf-8");
    assert_eq!(r.body, INDEX);
    // The http spelling of a known TLD is the same page.
    let h = ask(&path, "http://home.fed/", Load::Document);
    assert_eq!((h.status, h.body.as_slice()), (200, INDEX));
    // A name the zone does not have.
    assert_eq!(
        ask(&path, "fed://nobody-here.fed", Load::Document).status,
        404
    );
}

#[test]
fn a_manifest_with_a_changed_signature_gets_the_refusal_page_and_nothing_else() {
    let dir = scratch("tamper");
    // The overlay is the tampering distributor: the network's manifest with one signature digit changed.
    let m = String::from_utf8(MANIFEST.to_vec()).unwrap();
    let at = m.find("\"signature\":\"").unwrap() + "\"signature\":\"".len();
    let mut bytes = m.into_bytes();
    bytes[at] = if bytes[at] == b'0' { b'1' } else { b'0' };
    let mdir = dir.join("overlay/v1/manifest");
    std::fs::create_dir_all(&mdir).unwrap();
    std::fs::write(mdir.join(HOME_MANIFEST), &bytes).unwrap();
    let source = Overlay::new(dir.join("overlay"), Box::new(captured()));
    let (path, _end) = serve(&dir, resolver_handler(Box::new(source)), Limits::default());
    let r = ask(&path, "fed://home.fed", Load::Document);
    assert_eq!(r.status, 502);
    assert!(r.mime.starts_with("text/html"));
    let page = String::from_utf8(r.body).unwrap();
    assert!(page.contains("did not pass its checks"), "{page}");
    assert!(
        page.contains("the site&#39;s manifest (signed by the name&#39;s owner)"),
        "{page}"
    );
    assert!(
        !page.contains("lang-select"),
        "no byte of the page is shown"
    );
}

#[test]
fn a_malformed_request_is_answered_400_and_a_silent_one_is_dropped() {
    let dir = scratch("bad");
    let (path, _end) = serve(
        &dir,
        resolver_handler(Box::new(captured())),
        Limits::default(),
    );
    let mut s = UnixStream::connect(&path).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(b"GET / HTTP/1.1\n").unwrap();
    let r = read_reply(&mut s, 1 << 20).unwrap();
    assert_eq!(r.status, 400);
    // A peer that sends a line longer than allowed is cut off with a 400 too.
    let mut s = UnixStream::connect(&path).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let long = format!("FED/1 D fed://home.fed/{}", "a".repeat(9000));
    let _ = s.write_all(long.as_bytes());
    let r = read_reply(&mut s, 1 << 20).unwrap();
    assert_eq!(r.status, 400);
}

#[test]
fn loads_past_the_workers_and_the_queue_get_503_at_once() {
    let dir = scratch("busy");
    // One worker that holds its load until released, a queue of one.
    let gate = Arc::new(Barrier::new(2));
    let g = gate.clone();
    let handler: Handler = Arc::new(move |_url, _kind| {
        g.wait();
        Reply {
            status: 200,
            mime: "text/plain".into(),
            body: b"ok".to_vec(),
        }
    });
    let limits = Limits {
        workers: 1,
        queue: 1,
        idle: None,
    };
    let (path, _end) = serve(&dir, handler, limits);
    let send = |p: &Path| {
        let mut s = UnixStream::connect(p).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        s.write_all(request_line("fed://home.fed", Load::Subresource).as_bytes())
            .unwrap();
        s
    };
    let mut first = send(&path); // taken by the worker, which waits at the gate
    // Make sure the worker holds it before the queue is filled: its reply is still pending.
    first
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let mut probe = [0u8; 1];
    assert!(
        first.read(&mut probe).is_err(),
        "the worker is holding the first load"
    );
    first
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut second = send(&path); // waits in the queue
    let mut third = send(&path); // finds the queue full
    let r3 = read_reply(&mut third, 1 << 20).unwrap();
    assert_eq!(r3.status, 503);
    gate.wait(); // releases the first
    assert_eq!(read_reply(&mut first, 1 << 20).unwrap().status, 200);
    gate.wait(); // and the second
    assert_eq!(read_reply(&mut second, 1 << 20).unwrap().status, 200);
}

#[test]
fn an_idle_helper_returns_and_a_second_one_is_refused_while_it_runs() {
    let dir = scratch("idle");
    let limits = Limits {
        idle: Some(Duration::from_millis(400)),
        ..Limits::default()
    };
    let (path, end) = serve(&dir, resolver_handler(Box::new(captured())), limits);
    // While the first serves, a second cannot take the socket.
    let e = match atlas_fed::bind(&path) {
        Ok(_) => panic!("a second helper bound the socket"),
        Err(e) => e,
    };
    assert_eq!(e.kind(), std::io::ErrorKind::AddrInUse);
    assert_eq!(ask(&path, "fed://home.fed", Load::Document).status, 200);
    assert_eq!(end.recv_timeout(Duration::from_secs(20)), Ok(true));
    // Gone and its lock released: a new helper binds the same path (the stale socket replaced).
    assert!(atlas_fed::bind(&path).is_ok());
}

#[test]
fn the_socket_and_its_directory_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("mode");
    let path = dir.join("rt").join("atlas").join("fed.sock");
    let b = atlas_fed::bind(&path).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&b.path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
}
