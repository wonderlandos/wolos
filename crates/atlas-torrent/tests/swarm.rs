// SPDX-License-Identifier: BSD-2-Clause
//! A real swarm on 127.0.0.1, nothing from the internet: a generated 20 MB file, a tracker in this
//! test, the helper's own seeder (`atlas-torrent seed`, upload limited so the download lasts), and
//! the helper as the browser runs it (`atlas-torrent serve`, sharing off, no DHT). The test talks
//! ATT/1 like the browser: adds the .torrent, kills the helper with SIGKILL in the middle, starts
//! it again, and checks that the same job comes back with the pieces it had and finishes with the
//! same bytes, in state `done` (not shared).
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// 20 MB (decimal): not a multiple of the piece size, so the last piece is short (librqbit 9.1.0's
// `create_torrent` adds one hash too many when the length is an exact multiple; the check refuses that).
const SIZE: usize = 20_000_000;
const BIN: &str = env!("CARGO_BIN_EXE_atlas-torrent");

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-torrent-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut o = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            o.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            o.push(b[i]);
            i += 1;
        }
    }
    o
}

/// A tracker: remembers who announced with a non-zero port, answers every announce with them.
fn tracker() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let peers: Arc<Mutex<HashMap<Vec<u8>, Vec<u16>>>> = Arc::default();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let peers = Arc::clone(&peers);
            std::thread::spawn(move || announce(s, &peers));
        }
    });
    port
}

fn announce(mut s: TcpStream, peers: &Mutex<HashMap<Vec<u8>, Vec<u16>>>) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    if r.read_line(&mut line).is_err() {
        return;
    }
    let path = line.split(' ').nth(1).unwrap_or("").to_string();
    let q = path.split_once('?').map(|x| x.1).unwrap_or("");
    let mut hash = Vec::new();
    let mut port = 0u16;
    for kv in q.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            match k {
                "info_hash" => hash = unescape(v),
                "port" => port = v.parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    let mut compact = Vec::new();
    {
        let mut p = peers.lock().unwrap();
        let list = p.entry(hash).or_default();
        if port != 0 && !list.contains(&port) {
            list.push(port);
        }
        for pt in list.iter() {
            compact.extend_from_slice(&[127, 0, 0, 1]);
            compact.extend_from_slice(&pt.to_be_bytes());
        }
    }
    let mut body = format!("d8:intervali5e5:peers{}:", compact.len()).into_bytes();
    body.extend_from_slice(&compact);
    body.push(b'e');
    let _ = write!(
        s,
        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(&body);
}

fn data() -> Vec<u8> {
    // deterministic, not compressible
    let mut x: u64 = 0x9e3779b97f4a7c15;
    (0..SIZE)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

struct Helper {
    child: Child,
}

// A helper process of the test never outlives it, also when an assertion fails half-way.
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(dl: &Path, state: &Path, sock: &Path, tracker: u16) -> Helper {
    let me = std::env::current_exe().unwrap();
    let child = Command::new(BIN)
        .args([
            "serve",
            "--unconfined",
            "--local-peers",
            "--no-dht",
            "--downloads",
        ])
        .arg(dl)
        .arg("--state")
        .arg(state)
        .arg("--socket")
        .arg(sock)
        .arg("--peer-exe")
        .arg(&me)
        .args(["--tracker", &format!("http://127.0.0.1:{tracker}/announce")])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let t = Instant::now();
    while UnixStream::connect(sock).is_err() {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "helper did not open its socket"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    Helper { child }
}

struct Conn {
    w: UnixStream,
    r: BufReader<UnixStream>,
}

impl Conn {
    fn open(sock: &Path) -> Conn {
        let s = UnixStream::connect(sock).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        let mut c = Conn {
            w: s.try_clone().unwrap(),
            r: BufReader::new(s),
        };
        c.send(r#"{"op":"hello","version":1}"#);
        let h = c.next();
        assert_eq!(h["ev"], "hello", "{h}");
        assert_eq!(h["sharing"], false);
        c
    }
    fn send(&mut self, l: &str) {
        self.w.write_all(l.as_bytes()).unwrap();
        self.w.write_all(b"\n").unwrap();
    }
    fn next(&mut self) -> serde_json::Value {
        let mut l = String::new();
        self.r.read_line(&mut l).unwrap();
        if std::env::var_os("SWARM_DEBUG").is_some() {
            eprint!("EV {l}");
        }
        serde_json::from_str(&l).unwrap_or_else(|_| panic!("bad line {l:?}"))
    }
    /// Read rows until `f` says yes; returns that row.
    fn until(
        &mut self,
        limit: Duration,
        f: impl Fn(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let t = Instant::now();
        loop {
            assert!(t.elapsed() < limit, "timed out");
            let v = self.next();
            if v["ev"] == "job" && f(&v) {
                return v;
            }
        }
    }
}

/// Established TCP connections whose far end is 127.0.0.1:`port` (from /proc/net/tcp).
fn tcp_to(port: u16) -> usize {
    let want = format!(":{port:04X}");
    ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .map(|t| {
            t.lines()
                .skip(1)
                .filter(|l| {
                    let c: Vec<&str> = l.split_whitespace().collect();
                    c.len() > 3 && c[2].ends_with(&want) && c[3] == "01"
                })
                .count()
        })
        .sum()
}

/// Bytes allocated under `dir` (what a download wrote so far).
fn disk(dir: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let mut n = 0;
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let m = e.metadata().unwrap();
            if m.is_dir() {
                todo.push(e.path());
            } else {
                n += m.blocks() * 512;
            }
        }
    }
    n
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
fn kill_in_the_middle_then_resume_to_the_same_bytes() {
    let d = tmp("swarm");
    let (seed_dir, dl, st, seed_st) = (
        d.join("seed"),
        d.join("Downloads"),
        d.join("state"),
        d.join("seed-state"),
    );
    for p in [&seed_dir, &dl] {
        std::fs::create_dir_all(p).unwrap();
    }
    let bytes = data();
    std::fs::write(seed_dir.join("test-20mb.bin"), &bytes).unwrap();
    let tr = tracker();
    let torrent = d.join("t.torrent");
    let out = Command::new(BIN)
        .arg("make")
        .arg(seed_dir.join("test-20mb.bin"))
        .args([
            "--tracker",
            &format!("http://127.0.0.1:{tr}/announce"),
            "--out",
        ])
        .arg(&torrent)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(id.len(), 40);

    let seed_port = free_port();
    let _seeder = Helper {
        child: Command::new(BIN)
            .arg("seed")
            .arg(&torrent)
            .arg("--dir")
            .arg(&seed_dir)
            .arg("--state")
            .arg(&seed_st)
            .args([
                "--port",
                &seed_port.to_string(),
                "--rate",
                &(3 * 1024 * 1024).to_string(),
            ])
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    };
    // let the seeder check its copy and announce its port
    std::thread::sleep(Duration::from_secs(3));

    let sock = d.join("run").join("torrent.sock");
    let mut h = serve(&dl, &st, &sock, tr);
    let mut c = Conn::open(&sock);
    let mut t = Vec::new();
    std::fs::File::open(&torrent)
        .unwrap()
        .read_to_end(&mut t)
        .unwrap();
    c.send(&format!(
        r#"{{"op":"add_torrent","torrent_hex":"{}"}}"#,
        hex(&t)
    ));
    loop {
        let v = c.next();
        if v["ev"] == "added" {
            assert_eq!(v["id"], id.as_str());
            break;
        }
        assert_ne!(v["ev"], "error", "{v}");
    }
    let early = c.until(Duration::from_secs(60), |v| {
        v["state"] == "downloading" && v["have"].as_u64().unwrap() > (SIZE as u64) / 10
    });
    assert!(
        tcp_to(seed_port) >= 1,
        "downloading from the seeder: {early}"
    );
    // DL-SEC-08: the browser goes away -> every job pauses at once: no peer connection stays open
    // and none is opened while the helper waits for the browser
    drop(c);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        tcp_to(seed_port),
        0,
        "a peer connection outlived the browser"
    );
    let paused_size = disk(&dl);
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(
        tcp_to(seed_port),
        0,
        "a new peer connection while the browser was gone"
    );
    assert_eq!(
        disk(&dl),
        paused_size,
        "the download moved while the browser was gone"
    );
    // the browser is back in time: the job continues
    let mut c = Conn::open(&sock);
    let mid = c.until(Duration::from_secs(60), |v| {
        v["state"] == "downloading"
            && v["have"].as_u64().unwrap() > (SIZE as u64) * 3 / 10
            && v["peers"].as_u64().unwrap() >= 1
    });
    assert_eq!(mid["total"], SIZE as u64);
    assert!(mid["peers"].as_u64().unwrap() >= 1, "{mid}");
    let have_at_kill = mid["have"].as_u64().unwrap();
    // SIGKILL: no clean stop, like a crash or kill -9
    h.child.kill().unwrap();
    h.child.wait().unwrap();
    drop(c);
    let _ = std::fs::remove_file(&sock);

    let _h2 = serve(&dl, &st, &sock, tr);
    let mut c2 = Conn::open(&sock);
    // the same job is back and continues from the pieces on disk, not from zero
    let back = c2.until(Duration::from_secs(60), |v| {
        v["id"] == id.as_str() && v["state"] == "downloading"
    });
    let have_back = back["have"].as_u64().unwrap();
    eprintln!("have at kill {have_at_kill}, have after restart {have_back}");
    assert!(
        have_back >= have_at_kill / 2,
        "resumed from {have_back}, had {have_at_kill}"
    );
    let done = c2.until(Duration::from_secs(120), |v| {
        v["id"] == id.as_str() && v["state"] == "done"
    });
    assert_eq!(done["have"], SIZE as u64);
    assert_eq!(
        done["peers"], 0,
        "a finished job talks to no peer when sharing is off: {done}"
    );
    // a one-file torrent lands in its own folder (DL-SEC-05), named after the file
    assert_eq!(
        done["path"],
        dl.join("test-20mb")
            .join("test-20mb.bin")
            .display()
            .to_string()
    );
    let got = std::fs::read(dl.join("test-20mb").join("test-20mb.bin")).unwrap();
    assert!(got == bytes, "downloaded bytes differ");
    std::fs::write(
        d.join("result.txt"),
        format!("kill at {have_at_kill}, back at {have_back}, done {SIZE}\n"),
    )
    .unwrap();
}

#[test]
fn another_program_is_refused() {
    let d = tmp("refuse");
    std::fs::create_dir_all(&d).unwrap();
    let sock = d.join("torrent.sock");
    let child = Command::new(BIN)
        .args(["serve", "--unconfined", "--no-dht", "--downloads"])
        .arg(&d)
        .arg("--state")
        .arg(d.join("st"))
        .arg("--socket")
        .arg(&sock)
        .args(["--peer-exe", "/nonexistent/browser"])
        .spawn()
        .unwrap();
    let _h = Helper { child };
    let t = Instant::now();
    let s = loop {
        if let Ok(s) = UnixStream::connect(&sock) {
            break s;
        }
        assert!(t.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(50));
    };
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut w = s.try_clone().unwrap();
    let _ = w.write_all(b"{\"op\":\"hello\",\"version\":1}\n");
    let mut l = String::new();
    let n = BufReader::new(s).read_line(&mut l).unwrap_or(0);
    assert_eq!(
        n, 0,
        "the helper answered a program that is not the browser: {l}"
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&d).unwrap().permissions().mode() & 0o777,
        0o700
    );
}
