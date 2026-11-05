// SPDX-License-Identifier: GPL-2.0-or-later
//! A fake wold on a real socket (its hello, one call per connection, scripted answers per call
//! name), a fake browser (this test binary: its executable is "the browser"), and a bridge served
//! on a socket in a scratch directory.
#![allow(dead_code)] // each test binary uses a different part of this module
use atlas::server::{Config, serve};
use atlas::wold::Socket;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub fn scratch(name: &str) -> PathBuf {
    let d =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("atlas-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

/// One scripted answer; `hold` makes the fake wait (the person still choosing) until it gets a
/// message or its sender is dropped.
pub struct Scripted {
    pub answer: String,
    pub hold: Option<Receiver<()>>,
}

#[derive(Clone, Default)]
pub struct FakeWold {
    pub answers: Arc<Mutex<HashMap<String, VecDeque<Scripted>>>>,
    /// Every call line received, in order of arrival (the JSON after `call `).
    pub asked: Arc<Mutex<Vec<String>>>,
    /// Rung (with the `asked` lock) when a call arrives.
    bell: Arc<Condvar>,
}

impl FakeWold {
    pub fn answer(&self, call: &str, answer: &str) {
        self.push(call, answer, None);
    }

    pub fn push(&self, call: &str, answer: &str, hold: Option<Receiver<()>>) {
        self.answers
            .lock()
            .unwrap()
            .entry(call.into())
            .or_default()
            .push_back(Scripted {
                answer: answer.into(),
                hold,
            });
    }

    /// Block until `n` calls have arrived (the person is then looking at the sheet).
    pub fn wait_asked(&self, n: usize) {
        let g = self.asked.lock().unwrap();
        let (g, t) = self
            .bell
            .wait_timeout_while(g, Duration::from_secs(20), |a| a.len() < n)
            .unwrap();
        assert!(!t.timed_out(), "wold was asked {} of {n} calls", g.len());
    }

    /// Listen on `path` in a thread; each connection is served in its own thread.
    pub fn serve(&self, path: &Path) {
        let l = UnixListener::bind(path).unwrap();
        let me = self.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let me = me.clone();
                std::thread::spawn(move || me.one(s));
            }
        });
    }

    fn one(&self, mut s: UnixStream) {
        s.write_all(b"hello abc123\n").unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut echo = String::new();
        r.read_line(&mut echo).unwrap();
        assert_eq!(echo, "hello abc123\n", "the client echoes wold's hello");
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let json = line.strip_prefix("call ").expect("a call line").trim_end();
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let name = v["call"].as_str().unwrap().to_owned();
        self.asked.lock().unwrap().push(json.to_owned());
        self.bell.notify_all();
        let next = self
            .answers
            .lock()
            .unwrap()
            .get_mut(&name)
            .and_then(VecDeque::pop_front);
        let Some(next) = next else {
            panic!("the fake wold has no answer left for {name}");
        };
        if let Some(h) = next.hold {
            let _ = h.recv();
        }
        let _ = s.write_all(format!("{}\n", next.answer).as_bytes());
    }
}

/// This test binary, canonical: the browser of these tests.
pub fn me() -> PathBuf {
    std::env::current_exe().unwrap().canonicalize().unwrap()
}

/// A bridge on `dir/bridge.sock` asking the wold at `wold` (which may not exist), with `opener` as
/// the fallback opener. Returns the socket path.
pub fn bridge(
    dir: &Path,
    wold: &Path,
    browser: PathBuf,
    opener: Vec<String>,
    idle: Duration,
) -> PathBuf {
    let sock = dir.join("bridge.sock");
    let l = atlas::activation::bind(&sock).unwrap();
    let config = Config {
        browser,
        wold: Arc::new(Socket::at(wold)),
        opener,
        idle,
        log: Arc::new(|_| {}),
    };
    std::thread::spawn(move || serve(l, config));
    sock
}

/// The browser end of a connection.
pub struct Browser {
    pub r: BufReader<UnixStream>,
    pub w: UnixStream,
}

impl Browser {
    /// Connect and answer the bridge's greeting, as the browser does (ADR 0033).
    pub fn connect(sock: &Path) -> Self {
        let b = Self::connect_raw(sock);
        shared::jobs::answer_hello(&b.w).unwrap();
        b
    }

    /// Connect and leave the greeting unread.
    pub fn connect_raw(sock: &Path) -> Self {
        let s = UnixStream::connect(sock).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        Self {
            w: s.try_clone().unwrap(),
            r: BufReader::new(s),
        }
    }

    pub fn send(&mut self, line: &str) {
        self.w.write_all(format!("{line}\n").as_bytes()).unwrap();
    }

    /// The next line without its newline; None at the end of the stream.
    pub fn read(&mut self) -> Option<String> {
        let mut l = String::new();
        match self.r.read_line(&mut l) {
            Ok(0) => None,
            Ok(_) => Some(l.strip_suffix('\n').expect("a whole line").to_owned()),
            Err(e) => panic!("no reply: {e}"),
        }
    }

    /// Every line until the bridge closes (or resets) the connection.
    pub fn read_all(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            let mut l = String::new();
            match self.r.read_line(&mut l) {
                Ok(n) if n > 0 => out.push(l.trim_end().to_owned()),
                _ => return out,
            }
        }
    }

    pub fn hello(&mut self) {
        self.send(&format!(
            r#"{{"op":"hello","version":1,"pid":{}}}"#,
            std::process::id()
        ));
        assert_eq!(
            self.read().as_deref(),
            Some(r#"{"reply":"hello","version":1}"#)
        );
    }
}

/// A shell script that writes its arguments to `log` and exits with `code`.
pub fn opener(dir: &Path, log: &Path, code: i32) -> Vec<String> {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("opener");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit {code}\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    vec![p.to_string_lossy().into_owned()]
}
