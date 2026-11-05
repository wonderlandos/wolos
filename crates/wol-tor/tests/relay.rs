// SPDX-License-Identifier: GPL-2.0-or-later
//! The relay end to end against a fake tor on a Unix socket (ADR 0053 D2, D3): credentials per
//! site, data both ways, and the kill switch (no tor, not bootstrapped, internal targets, UDP).
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;
use wol_tor::group::Group;
use wol_tor::isolation::Isolator;
use wol_tor::relay::{Ready, Relay};
use wol_tor::socks::{self, Target};

static N: AtomicUsize = AtomicUsize::new(0);

fn sock_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "wol-tor-it-{}-{}.sock",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ))
}

/// What the fake tor saw: ((username, password), target) per stream.
type Seen = Arc<Mutex<Vec<((String, String), Target)>>>;

/// A fake tor: SOCKS5 with username/password only, records (credential, target), echoes data.
fn fake_tor(path: &PathBuf) -> (Seen, mpsc::Receiver<()>) {
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let s2 = Arc::clone(&seen);
    thread::Builder::new()
        .name("fake-tor".into())
        .spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { return };
                c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let s3 = Arc::clone(&s2);
                let tx = tx.clone();
                thread::Builder::new()
                    .name("fake-tor-conn".into())
                    .spawn(move || {
                        assert_eq!(
                            socks::server_greeting(&mut c).unwrap(),
                            socks::USER_PASS,
                            "the relay always authenticates"
                        );
                        let cred = socks::server_userpass(&mut c).unwrap();
                        let t = socks::server_request(&mut c).unwrap();
                        s3.lock().unwrap().push((cred, t));
                        c.write_all(&socks::reply_bytes(socks::Reply::Succeeded))
                            .unwrap();
                        let _ = tx.send(());
                        let mut buf = [0u8; 64];
                        while let Ok(n) = c.read(&mut buf) {
                            if n == 0 || c.write_all(&buf[..n]).is_err() {
                                break;
                            }
                        }
                    })
                    .unwrap();
            }
        })
        .unwrap();
    (seen, rx)
}

fn relay(tor: PathBuf, ready: bool, wait: Duration) -> (SocketAddr, Arc<Ready>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let r = Arc::new(Ready::default());
    r.set(ready);
    let relay = Arc::new(Relay {
        tor_socks: tor,
        isolator: Arc::new(Isolator::new(Group::new("it").unwrap())),
        ready: Arc::clone(&r),
        ready_wait: wait,
    });
    thread::Builder::new()
        .name("relay".into())
        .spawn(move || relay.serve(l, |_| {}))
        .unwrap();
    (addr, r)
}

/// A relay whose first stream thread the system refuses.
fn relay_refusing_first(tor: PathBuf) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let r = Arc::new(Ready::default());
    r.set(true);
    let relay = Arc::new(Relay {
        tor_socks: tor,
        isolator: Arc::new(Isolator::new(Group::new("it").unwrap())),
        ready: r,
        ready_wait: Duration::from_millis(50),
    });
    let starts = AtomicUsize::new(0);
    thread::Builder::new()
        .name("relay".into())
        .spawn(move || {
            relay.serve_with(l, move |what, work| {
                if starts.fetch_add(1, Ordering::SeqCst) == 0 {
                    drop(work);
                    false
                } else {
                    shared::bounded::spawn_conn(what, &|_| {}, work)
                }
            })
        })
        .unwrap();
    addr
}

fn domain_request(cmd: u8, host: &str, port: u16) -> Vec<u8> {
    let mut v = vec![5, cmd, 0, 3, u8::try_from(host.len()).unwrap()];
    v.extend_from_slice(host.as_bytes());
    v.extend_from_slice(&port.to_be_bytes());
    v
}

/// Browser side: greeting (optionally user/pass), request; returns the reply code and the stream.
fn browser(addr: SocketAddr, cred: Option<(&str, &str)>, req: &[u8]) -> (u8, TcpStream) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    match cred {
        None => s.write_all(&[5, 1, 0]).unwrap(),
        Some(_) => s.write_all(&[5, 1, 2]).unwrap(),
    }
    let mut m = [0u8; 2];
    s.read_exact(&mut m).unwrap();
    if let Some((u, p)) = cred {
        assert_eq!(m, [5, 2]);
        let mut a = vec![1, u8::try_from(u.len()).unwrap()];
        a.extend_from_slice(u.as_bytes());
        a.push(u8::try_from(p.len()).unwrap());
        a.extend_from_slice(p.as_bytes());
        s.write_all(&a).unwrap();
        let mut ok = [0u8; 2];
        s.read_exact(&mut ok).unwrap();
        assert_eq!(ok, [1, 0]);
    } else {
        assert_eq!(m, [5, 0]);
    }
    s.write_all(req).unwrap();
    let mut r = [0u8; 10];
    s.read_exact(&mut r).unwrap();
    (r[1], s)
}

#[test]
fn streams_are_isolated_per_site_and_carry_data() {
    let path = sock_path();
    let (seen, got) = fake_tor(&path);
    let (addr, _) = relay(path.clone(), true, Duration::from_millis(50));
    let (code, mut s) = browser(addr, None, &domain_request(1, "www.example.org", 443));
    assert_eq!(code, 0);
    s.write_all(b"ping").unwrap();
    let mut back = [0u8; 4];
    s.read_exact(&mut back).unwrap();
    assert_eq!(&back, b"ping");
    let (code, _s2) = browser(addr, None, &domain_request(1, "tracker.example.net", 443));
    assert_eq!(code, 0);
    let (code, _s3) = browser(
        addr,
        Some(("news.example", "3")),
        &domain_request(1, "cdn.example.com", 443),
    );
    assert_eq!(code, 0);
    for _ in 0..3 {
        got.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    let creds: Vec<(String, String)> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(c, _)| c.clone())
        .collect();
    assert_eq!(creds[0], ("it/example.org".to_string(), "0".to_string()));
    assert_eq!(creds[1], ("it/example.net".to_string(), "0".to_string()));
    assert_eq!(
        creds[2],
        ("it/news.example".to_string(), "0.3".to_string()),
        "the browser's site wins"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn no_tor_means_no_connection_at_all() {
    let path = sock_path();
    let (addr, _) = relay(path, true, Duration::from_millis(50));
    let (code, _) = browser(addr, None, &domain_request(1, "example.org", 443));
    assert_eq!(
        code, 1,
        "general failure, and the relay has no other place to connect"
    );
}

#[test]
fn connects_wait_for_bootstrap_then_fail_closed() {
    let path = sock_path();
    let (seen, _got) = fake_tor(&path);
    let (addr, ready) = relay(path.clone(), false, Duration::from_millis(100));
    let (code, _) = browser(addr, None, &domain_request(1, "example.org", 443));
    assert_eq!(code, 1);
    assert!(
        seen.lock().unwrap().is_empty(),
        "tor is not asked before bootstrap"
    );
    ready.set(true);
    let (code, _s) = browser(addr, None, &domain_request(1, "example.org", 443));
    assert_eq!(code, 0);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn udp_and_internal_targets_are_refused_before_tor() {
    let path = sock_path();
    let (seen, _got) = fake_tor(&path);
    let (addr, _) = relay(path.clone(), true, Duration::from_millis(50));
    let (code, _) = browser(addr, None, &[5, 3, 0, 1, 0, 0, 0, 0, 0, 0]);
    assert_eq!(code, 7, "UDP ASSOCIATE: command not supported");
    let (code, _) = browser(addr, None, &domain_request(1, "printer.local", 631));
    assert_eq!(code, 2, "internal name: not allowed");
    let (code, _) = browser(addr, None, &[5, 1, 0, 1, 192, 168, 1, 1, 0, 80]);
    assert_eq!(code, 2, "private address: not allowed");
    assert!(seen.lock().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn tor_going_away_stops_new_streams() {
    let path = sock_path();
    let (_seen, _got) = fake_tor(&path);
    let (addr, _) = relay(path.clone(), true, Duration::from_millis(50));
    assert_eq!(
        browser(addr, None, &domain_request(1, "example.org", 443)).0,
        0
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        browser(addr, None, &domain_request(1, "example.org", 443)).0,
        1
    );
}

/// QA-A9-SEC-11: one refused stream thread drops that stream; the relay goes on serving.
#[test]
fn a_refused_stream_thread_does_not_end_the_relay() {
    let path = sock_path();
    let (_seen, _got) = fake_tor(&path);
    let addr = relay_refusing_first(path);
    let mut first = TcpStream::connect(addr).unwrap();
    first
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut b = [0u8; 1];
    assert!(
        matches!(first.read(&mut b), Ok(0) | Err(_)),
        "the refused stream is closed"
    );
    let (code, _s) = browser(addr, None, &domain_request(1, "www.example.org", 443));
    assert_eq!(code, 0, "the next stream is served");
}
