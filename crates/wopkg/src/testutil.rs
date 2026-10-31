//! Test helpers: temp dirs and a loopback HTTP/1.1 server with Range and ETag support.
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub fn temp(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "wopkg-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[derive(Clone)]
pub struct Served {
    pub body: Vec<u8>,
    pub etag: Option<String>,
}

impl Served {
    pub fn bytes(body: Vec<u8>) -> Self {
        Served { body, etag: None }
    }
    pub fn etag(mut self, e: &str) -> Self {
        self.etag = Some(e.into());
        self
    }
}

pub struct Server {
    pub port: u16,
    /// Request lines with their Range header, in order.
    pub log: Arc<Mutex<Vec<String>>>,
}

impl Server {
    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

pub fn serve(routes: Vec<(&str, Served)>) -> Server {
    serve_inner(None, routes)
}

fn serve_inner(port: Option<u16>, routes: Vec<(&str, Served)>) -> Server {
    let l = TcpListener::bind(("127.0.0.1", port.unwrap_or(0))).unwrap();
    let port = l.local_addr().unwrap().port();
    let routes: Vec<(String, Served)> =
        routes.into_iter().map(|(p, s)| (p.to_owned(), s)).collect();
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let routes = routes.clone();
            let log = log2.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(c.try_clone().unwrap());
                loop {
                    let mut first = String::new();
                    if r.read_line(&mut first).unwrap_or(0) == 0 {
                        return;
                    }
                    let (mut range, mut inm) = (None::<String>, None::<String>);
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                            break;
                        }
                        let (k, v) = h.split_once(':').unwrap_or(("", ""));
                        match k.to_ascii_lowercase().as_str() {
                            "range" => range = Some(v.trim().to_owned()),
                            "if-none-match" => inm = Some(v.trim().to_owned()),
                            _ => {}
                        }
                    }
                    let path = first.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    log.lock()
                        .unwrap()
                        .push(format!("{path} {}", range.clone().unwrap_or_default()));
                    let mut w = &c;
                    let Some((_, s)) = routes.iter().find(|(p, _)| *p == path) else {
                        let _ = w.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n");
                        continue;
                    };
                    if s.etag.is_some() && inm == s.etag {
                        let _ =
                            w.write_all(b"HTTP/1.1 304 Not Modified\r\ncontent-length: 0\r\n\r\n");
                        continue;
                    }
                    let tag = s
                        .etag
                        .as_ref()
                        .map(|e| format!("etag: {e}\r\n"))
                        .unwrap_or_default();
                    let parsed = range.as_deref().and_then(|r| {
                        let (a, b) = r.strip_prefix("bytes=")?.split_once('-')?;
                        let a: usize = a.parse().ok()?;
                        let b: usize = if b.is_empty() {
                            s.body.len().saturating_sub(1)
                        } else {
                            b.parse().ok()?
                        };
                        Some((a, b))
                    });
                    match parsed {
                        Some((a, _)) if a >= s.body.len() => {
                            let _ = w.write_all(
                                b"HTTP/1.1 416 Range Not Satisfiable\r\ncontent-length: 0\r\n\r\n",
                            );
                        }
                        Some((a, b)) => {
                            let b = b.min(s.body.len() - 1);
                            let part = &s.body[a..=b];
                            let _ = write!(
                                w,
                                "HTTP/1.1 206 Partial Content\r\n{tag}content-range: bytes {a}-{b}/{}\r\ncontent-length: {}\r\n\r\n",
                                s.body.len(),
                                part.len()
                            );
                            let _ = w.write_all(part);
                        }
                        None => {
                            let _ = write!(
                                w,
                                "HTTP/1.1 200 OK\r\n{tag}content-length: {}\r\n\r\n",
                                s.body.len()
                            );
                            let _ = w.write_all(&s.body);
                        }
                    }
                }
            });
        }
    });
    Server { port, log }
}

/// [`serve`] on a fixed port (the caller probed it free).
pub fn serve_on(port: u16, routes: Vec<(&str, Served)>) -> Server {
    serve_inner(Some(port), routes)
}
