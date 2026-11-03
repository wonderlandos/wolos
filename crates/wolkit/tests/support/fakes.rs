// SPDX-License-Identifier: BSD-2-Clause
//! A fake Kit host: speaks the real `shared::view` wires on a socket, records every message from
//! the engine and sends the events a test asks for.
#![allow(dead_code)] // each test binary uses some of these helpers
use shared::view::{FromHost, ToHost};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct FakeKit {
    pub path: PathBuf,
    pub seen: Arc<Mutex<Vec<ToHost>>>,
    tx: Sender<FromHost>,
    raw: Sender<String>,
}

impl FakeKit {
    pub fn start(dir: &Path) -> FakeKit {
        let path = dir.join("kit.sock");
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = channel::<FromHost>();
        let (raw, raw_rx) = channel::<String>();
        let s2 = seen.clone();
        std::thread::spawn(move || {
            let Ok((conn, _)) = l.accept() else { return };
            let rd = conn.try_clone().unwrap();
            std::thread::spawn(move || {
                for line in BufReader::new(rd).lines().map_while(Result::ok) {
                    let m = ToHost::parse(&line)
                        .unwrap_or_else(|| panic!("the host dropped an invalid line: {line}"));
                    s2.lock().unwrap().push(m);
                }
            });
            let c2 = conn.try_clone().unwrap();
            std::thread::spawn(move || {
                while let Ok(t) = raw_rx.recv() {
                    if (&c2).write_all(t.as_bytes()).is_err() {
                        break;
                    }
                }
            });
            let mut w = &conn;
            while let Ok(e) = rx.recv() {
                if w.write_all(e.to_line().as_bytes()).is_err() {
                    break;
                }
            }
        });
        FakeKit {
            path,
            seen,
            tx,
            raw,
        }
    }

    /// Bytes as they are, with a newline: for the lines a real host would never send.
    pub fn send_raw(&self, text: &str) {
        self.raw.send(format!("{text}\n")).unwrap();
    }

    pub fn send(&self, e: FromHost) {
        self.tx.send(e).unwrap();
    }

    /// The messages seen so far, once at least `n` have arrived.
    pub fn wait(&self, n: usize) -> Vec<ToHost> {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let v = self.seen.lock().unwrap().clone();
            if v.len() >= n || Instant::now() > end {
                return v;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
