//! Downloads (ADR 0021): HTTPS only (loopback HTTP when a test allows it), several at once, resumed
//! from `<dest>.part` with `Range`, checked against size and SHA-256 before the rename. A failed
//! check deletes the part: nothing unverified ever lands at `dest`.
use crate::ffi::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Item {
    pub url: String,
    pub dest: PathBuf,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Request {
    pub items: Vec<Item>,
    #[serde(default)]
    pub jobs: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Downloaded now (bytes fetched over the network).
    Fetched {
        bytes: u64,
        resumed: bool,
    },
    /// Already at `dest` with the right hash.
    Cached,
    /// Not changed since the stored ETag / Last-Modified (index inputs).
    NotModified,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Done {
    pub url: String,
    pub dest: PathBuf,
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// Network policy for one context.
#[derive(Clone, Debug)]
pub struct Net {
    pub allow_loopback_http: bool,
    pub agent: ureq::Agent,
}

impl Net {
    pub fn new(allow_loopback_http: bool) -> Self {
        use ureq::tls::{RootCerts, TlsConfig};
        let agent = ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .https_only(!allow_loopback_http)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .max_redirects(8)
            .user_agent(concat!("wopkg/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Net {
            allow_loopback_http,
            agent,
        }
    }

    /// TLS only; plain HTTP only to loopback when allowed.
    pub fn check_url(&self, url: &str) -> Result<()> {
        if url.starts_with("https://") {
            return Ok(());
        }
        let host = url
            .strip_prefix("http://")
            .and_then(|r| r.split(['/', ':']).next())
            .unwrap_or("");
        if self.allow_loopback_http
            && (host == "127.0.0.1" || host == "localhost" || host == "[::1]")
        {
            return Ok(());
        }
        Err(Error::untrusted(format!(
            "WolOS downloads only over HTTPS, and {url} is not."
        )))
    }
}

pub fn sha256_file(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex(&h.finalize()))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn part_of(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Progress events from a worker.
pub enum Event {
    Bytes {
        url: String,
        done: u64,
        total: Option<u64>,
    },
}

/// One item, on the calling thread; `tick` gets byte counts.
pub fn one(net: &Net, it: &Item, tick: &mut dyn FnMut(u64, Option<u64>)) -> Outcome {
    match one_inner(net, it, tick) {
        Ok(o) => o,
        Err(e) => Outcome::Failed { message: e.message },
    }
}

fn one_inner(net: &Net, it: &Item, tick: &mut dyn FnMut(u64, Option<u64>)) -> Result<Outcome> {
    net.check_url(&it.url)?;
    let want = it.sha256.as_deref().map(str::to_ascii_lowercase);
    if let Some(w) = &want
        && sha256_file(&it.dest).as_deref() == Some(w.as_str())
    {
        return Ok(Outcome::Cached);
    }
    if let Some(d) = it.dest.parent() {
        std::fs::create_dir_all(d)
            .map_err(|_| Error::unavailable("The download folder cannot be created."))?;
    }
    let part = part_of(&it.dest);
    let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let mut req = net.agent.get(&it.url);
    if have > 0 {
        req = req.header("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| {
        Error::unavailable(format!("{} could not be reached ({e}).", host(&it.url)))
    })?;
    let status = resp.status().as_u16();
    let (append, resumed) = match status {
        206 if have > 0 => (true, true),
        200 => (false, false),
        416 if have > 0 => (true, true), // the part is already complete
        404 => {
            return Err(Error::new(
                crate::ffi::Status::NotFound,
                format!("{} is not on the server.", it.url),
            ));
        }
        s => {
            return Err(Error::unavailable(format!(
                "{} answered {s}.",
                host(&it.url)
            )));
        }
    };
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(|n| if append { n + have } else { n })
        .or(it.size);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&part)
        .map_err(|_| Error::unavailable("A download cannot be written."))?;
    let mut fetched = 0u64;
    if status != 416 {
        let mut body = resp.into_body().into_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = if append { have } else { 0 };
        let limit = it.size.unwrap_or(u64::MAX);
        loop {
            let n = body.read(&mut buf).map_err(|_| {
                Error::unavailable(format!(
                    "The download from {} stopped. Try again to resume it.",
                    host(&it.url)
                ))
            })?;
            if n == 0 {
                break;
            }
            f.write_all(&buf[..n])
                .map_err(|_| Error::unavailable("A download cannot be written."))?;
            done += n as u64;
            fetched += n as u64;
            if done > limit {
                drop(f);
                let _ = std::fs::remove_file(&part);
                return Err(Error::untrusted(format!(
                    "{} sent more than expected.",
                    it.url
                )));
            }
            tick(done, total);
        }
    }
    f.sync_all().ok();
    drop(f);
    let len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if let Some(s) = it.size
        && len != s
    {
        if len > s {
            let _ = std::fs::remove_file(&part);
        }
        return Err(Error::untrusted(format!("{} has the wrong size.", it.url)));
    }
    if let Some(w) = &want
        && sha256_file(&part).as_deref() != Some(w.as_str())
    {
        let _ = std::fs::remove_file(&part);
        return Err(Error::untrusted(format!(
            "{} does not match its checksum, so it was deleted.",
            it.url
        )));
    }
    std::fs::rename(&part, &it.dest)
        .map_err(|_| Error::unavailable("A download cannot be saved."))?;
    Ok(Outcome::Fetched {
        bytes: fetched,
        resumed,
    })
}

fn host(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or(url)
}

/// Every item, `jobs` at a time on worker threads; progress reaches `progress` on this thread.
pub fn many(net: &Net, req: &Request, progress: &mut dyn FnMut(&serde_json::Value)) -> Vec<Done> {
    let jobs = req
        .jobs
        .unwrap_or(8)
        .clamp(1, 32)
        .min(req.items.len().max(1));
    let (work_tx, work_rx) = mpsc::channel::<(usize, Item)>();
    let work_rx = std::sync::Arc::new(std::sync::Mutex::new(work_rx));
    let (ev_tx, ev_rx) = mpsc::channel::<std::result::Result<(usize, Done), Event>>();
    for (i, it) in req.items.iter().cloned().enumerate() {
        let _ = work_tx.send((i, it));
    }
    drop(work_tx);
    let mut out: Vec<Option<Done>> = vec![None; req.items.len()];
    std::thread::scope(|s| {
        for _ in 0..jobs {
            let (rx, tx, net) = (work_rx.clone(), ev_tx.clone(), net.clone());
            s.spawn(move || {
                loop {
                    let next = rx.lock().ok().and_then(|r| r.recv().ok());
                    let Some((i, it)) = next else { break };
                    let mut last = std::time::Instant::now();
                    let url = it.url.clone();
                    let outcome = one(&net, &it, &mut |done, total| {
                        if last.elapsed() > Duration::from_millis(100) {
                            last = std::time::Instant::now();
                            let _ = tx.send(Err(Event::Bytes {
                                url: url.clone(),
                                done,
                                total,
                            }));
                        }
                    });
                    let _ = tx.send(Ok((
                        i,
                        Done {
                            url: it.url,
                            dest: it.dest,
                            outcome,
                        },
                    )));
                }
            });
        }
        drop(ev_tx);
        for ev in ev_rx {
            match ev {
                Err(Event::Bytes { url, done, total }) => progress(
                    &serde_json::json!({"event":"bytes","url":url,"done":done,"total":total}),
                ),
                Ok((i, d)) => {
                    progress(&serde_json::json!({"event":"done","url":d.url,"outcome":d.outcome}));
                    out[i] = Some(d);
                }
            }
        }
    });
    out.into_iter().flatten().collect()
}

/// Conditional GET of an index input into `dest`: the validator (ETag or Last-Modified) is kept in
/// `<dest>.etag`, so an unchanged input is not downloaded again.
pub fn conditional(net: &Net, url: &str, dest: &Path) -> Result<Outcome> {
    net.check_url(url)?;
    let tag_path = PathBuf::from(format!("{}.etag", dest.display()));
    let tag = std::fs::read_to_string(&tag_path)
        .ok()
        .filter(|_| dest.exists());
    let mut req = net.agent.get(url);
    if let Some(t) = &tag {
        req = match t.split_once(' ') {
            Some(("etag", v)) => req.header("If-None-Match", v),
            Some(("date", v)) => req.header("If-Modified-Since", v),
            _ => req,
        };
    }
    let resp = req
        .call()
        .map_err(|e| Error::unavailable(format!("{} could not be reached ({e}).", host(url))))?;
    match resp.status().as_u16() {
        304 => return Ok(Outcome::NotModified),
        200 => {}
        404 => {
            return Err(Error::new(
                crate::ffi::Status::NotFound,
                format!("{url} is not on the server."),
            ));
        }
        s => return Err(Error::unavailable(format!("{} answered {s}.", host(url)))),
    }
    let h = |k: &str| {
        resp.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let validator = h("etag")
        .map(|e| format!("etag {e}"))
        .or_else(|| h("last-modified").map(|d| format!("date {d}")));
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)
            .map_err(|_| Error::unavailable("The index folder cannot be created."))?;
    }
    let part = part_of(dest);
    let mut f = std::fs::File::create(&part)
        .map_err(|_| Error::unavailable("The index cannot be written."))?;
    let n = std::io::copy(&mut resp.into_body().into_reader(), &mut f)
        .map_err(|_| Error::unavailable(format!("The download from {} stopped.", host(url))))?;
    drop(f);
    std::fs::rename(&part, dest).map_err(|_| Error::unavailable("The index cannot be written."))?;
    match validator {
        Some(v) => std::fs::write(&tag_path, v).ok(),
        None => std::fs::remove_file(&tag_path).ok(),
    };
    Ok(Outcome::Fetched {
        bytes: n,
        resumed: false,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::testutil::{Served, serve, temp};

    #[test]
    fn https_only() {
        let strict = Net::new(false);
        assert!(strict.check_url("https://example.org/x").is_ok());
        assert!(strict.check_url("http://example.org/x").is_err());
        assert!(strict.check_url("http://127.0.0.1:1/x").is_err());
        let test = Net::new(true);
        assert!(test.check_url("http://127.0.0.1:1/x").is_ok());
        assert!(test.check_url("http://example.org/x").is_err());
        assert!(test.check_url("ftp://127.0.0.1/x").is_err());
    }

    #[test]
    fn parallel_verified_and_resumed() {
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let sum = hex(&Sha256::digest(&body));
        let srv = serve(vec![
            ("/a", Served::bytes(body.clone())),
            ("/b", Served::bytes(b"bee".to_vec())),
        ]);
        let d = temp("fetch");
        let net = Net::new(true);
        // A half-finished earlier download is resumed with a Range request.
        std::fs::write(d.join("a.part"), &body[..100_000]).unwrap();
        let req = Request {
            items: vec![
                Item {
                    url: srv.url("/a"),
                    dest: d.join("a"),
                    sha256: Some(sum.clone()),
                    size: Some(body.len() as u64),
                },
                Item {
                    url: srv.url("/b"),
                    dest: d.join("b"),
                    sha256: Some("00".repeat(32)),
                    size: None,
                },
                Item {
                    url: srv.url("/missing"),
                    dest: d.join("c"),
                    sha256: None,
                    size: None,
                },
            ],
            jobs: Some(3),
        };
        let mut events = 0;
        let done = many(&net, &req, &mut |_| events += 1);
        assert_eq!(
            done[0].outcome,
            Outcome::Fetched {
                bytes: 200_000,
                resumed: true
            }
        );
        assert_eq!(std::fs::read(d.join("a")).unwrap(), body);
        assert!(
            matches!(done[1].outcome, Outcome::Failed { .. }),
            "bad hash refused"
        );
        assert!(!d.join("b").exists() && !d.join("b.part").exists());
        assert!(matches!(done[2].outcome, Outcome::Failed { .. }));
        assert!(events >= 3);
        let again = many(
            &net,
            &Request {
                items: req.items[..1].to_vec(),
                jobs: None,
            },
            &mut |_| {},
        );
        assert_eq!(again[0].outcome, Outcome::Cached);
    }

    #[test]
    fn conditional_get_skips_unchanged_inputs() {
        let srv = serve(vec![(
            "/core.db",
            Served::bytes(b"db".to_vec()).etag("\"v1\""),
        )]);
        let d = temp("cond");
        let net = Net::new(true);
        let dest = d.join("core.db");
        assert!(matches!(
            conditional(&net, &srv.url("/core.db"), &dest).unwrap(),
            Outcome::Fetched { bytes: 2, .. }
        ));
        assert_eq!(
            conditional(&net, &srv.url("/core.db"), &dest).unwrap(),
            Outcome::NotModified
        );
    }
}
