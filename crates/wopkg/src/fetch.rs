// SPDX-License-Identifier: GPL-2.0-or-later
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

/// Most bytes a download without a known size may be (an AppImage from a bare URL).
pub const MAX_UNSIZED: u64 = 4 << 30;

/// What a person is told when the disk is full (QA-STORE-24).
pub const NO_ROOM: &str = "There is no room left on the disk. Free some space, then try again.";

/// Whether `e` is the disk being full.
pub fn is_full(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::StorageFull || e.raw_os_error() == Some(28)
}

/// A failed write as a sentence: the disk being full is named ([`NO_ROOM`]), anything else is
/// `plain`; never the path or the system's text (QA-STORE-24).
pub fn write_error(e: &std::io::Error, plain: &str) -> Error {
    Error::unavailable(if is_full(e) { NO_ROOM } else { plain })
}

/// The ETag a part was started under, beside it.
fn tag_of(part: &Path) -> PathBuf {
    let mut s = part.as_os_str().to_owned();
    s.push(".etag");
    PathBuf::from(s)
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
    let tag_file = tag_of(&part);
    // A part is resumed only when the rest can be proven to belong to it: a checksum to check
    // at the end, or the ETag it was started under (sent as If-Range, so a changed file comes
    // back whole). Otherwise it starts over (QA-STORE-19).
    let tag = std::fs::read_to_string(&tag_file)
        .ok()
        .filter(|t| !t.is_empty());
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > 0 && want.is_none() && tag.is_none() {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }
    let mut req = net.agent.get(&it.url);
    if have > 0 {
        req = req.header("Range", &format!("bytes={have}-"));
        if let Some(t) = &tag {
            req = req.header("If-Range", t);
        }
    }
    let resp = req
        .call()
        .map_err(|_| Error::unavailable(format!("{} could not be reached.", host(&it.url))))?;
    let status = resp.status().as_u16();
    let (append, resumed) = match status {
        206 if have > 0 => (true, true),
        200 => (false, false),
        // The part is already complete: only when the checksum or the size will prove it.
        416 if have > 0 && (want.is_some() || it.size == Some(have)) => (true, true),
        416 if have > 0 => {
            let _ = std::fs::remove_file(&part);
            let _ = std::fs::remove_file(&tag_file);
            return one_inner(net, it, tick);
        }
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
    if !append {
        // A fresh start keeps the ETag it was started under, for a resume later.
        match resp.headers().get("etag").and_then(|v| v.to_str().ok()) {
            Some(t) => {
                let _ = std::fs::write(&tag_file, t);
            }
            None => {
                let _ = std::fs::remove_file(&tag_file);
            }
        }
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&part)
        .map_err(|e| write_error(&e, "A download cannot be written."))?;
    let mut fetched = 0u64;
    if status != 416 {
        let mut body = resp.into_body().into_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = if append { have } else { 0 };
        // Without a size, a download is still bounded (QA-STORE-19).
        let limit = it.size.unwrap_or(MAX_UNSIZED);
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
                .map_err(|e| write_error(&e, "A download cannot be written."))?;
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
    std::fs::rename(&part, &it.dest).map_err(|e| write_error(&e, "A download cannot be saved."))?;
    let _ = std::fs::remove_file(&tag_file);
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

/// Largest index input a source may serve (a pacman sync db, an appstream file, the community package dump).
/// A longer body is refused and its part file deleted (audit C10-2: an endless body must not
/// fill the cache partition).
pub const MAX_INPUT: u64 = 512 << 20;

/// What a bounded conditional GET brought back ([`get_conditional`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Got {
    /// Not changed since the validator sent.
    NotModified,
    /// The new body and its validator (`etag <v>` / `date <v>`) for the next request.
    Body {
        bytes: Vec<u8>,
        validator: Option<String>,
    },
}

/// The validator file kept next to `dest`.
pub fn tag_path(dest: &Path) -> PathBuf {
    PathBuf::from(format!("{}.etag", dest.display()))
}

/// Sends the GET with `tag` (as stored by [`conditional`]) and checks the status. `None` is 304.
fn start(net: &Net, url: &str, tag: Option<&str>) -> Result<Option<(ureq::Body, Option<String>)>> {
    net.check_url(url)?;
    let mut req = net.agent.get(url);
    if let Some(t) = tag {
        req = match t.split_once(' ') {
            Some(("etag", v)) => req.header("If-None-Match", v),
            Some(("date", v)) => req.header("If-Modified-Since", v),
            _ => req,
        };
    }
    let resp = req
        .call()
        .map_err(|_| Error::unavailable(format!("{} could not be reached.", host(url))))?;
    match resp.status().as_u16() {
        304 => return Ok(None),
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
    Ok(Some((resp.into_body(), validator)))
}

/// Copies at most `max` bytes of `body` into `w`; one byte more is an error (never cut short).
fn copy_bounded(body: ureq::Body, w: &mut dyn Write, max: u64, url: &str) -> Result<u64> {
    let n = std::io::copy(&mut body.into_reader().take(max + 1), w).map_err(|e| {
        if is_full(&e) {
            Error::unavailable(NO_ROOM)
        } else {
            Error::unavailable(format!("The download from {} stopped.", host(url)))
        }
    })?;
    if n > max {
        return Err(Error::untrusted(format!(
            "The download from {} is larger than it should be.",
            host(url)
        )));
    }
    Ok(n)
}

/// Bounded conditional GET into memory: nothing is written, so the caller can verify the body
/// before it replaces a cached copy (the release feed and its signature, audit C10-2).
pub fn get_conditional(net: &Net, url: &str, tag: Option<&str>, max: u64) -> Result<Got> {
    let Some((body, validator)) = start(net, url, tag)? else {
        return Ok(Got::NotModified);
    };
    let mut bytes = Vec::new();
    copy_bounded(body, &mut bytes, max, url)?;
    Ok(Got::Body { bytes, validator })
}

/// Conditional GET of an index input into `dest`, at most `max` bytes: the validator (ETag or
/// Last-Modified) is kept in `<dest>.etag`, so an unchanged input is not downloaded again. The
/// body goes to `<dest>.part` first; a failed or oversized download deletes it and leaves `dest`
/// and its validator as they were.
pub fn conditional(net: &Net, url: &str, dest: &Path, max: u64) -> Result<Outcome> {
    let tag_path = tag_path(dest);
    let tag = std::fs::read_to_string(&tag_path)
        .ok()
        .filter(|_| dest.exists());
    let Some((body, validator)) = start(net, url, tag.as_deref())? else {
        return Ok(Outcome::NotModified);
    };
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)
            .map_err(|_| Error::unavailable("The index folder cannot be created."))?;
    }
    let part = part_of(dest);
    let mut f = std::fs::File::create(&part)
        .map_err(|e| write_error(&e, "The index cannot be written."))?;
    let n = match copy_bounded(body, &mut f, max, url) {
        Ok(n) => n,
        Err(e) => {
            drop(f);
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    drop(f);
    // The old validator goes first: a crash between the steps never pairs a new tag with the
    // old body (an old tag with the new body only costs one more download).
    let _ = std::fs::remove_file(&tag_path);
    std::fs::rename(&part, dest).map_err(|e| write_error(&e, "The index cannot be written."))?;
    if let Some(v) = validator {
        let _ = std::fs::write(&tag_path, v);
    }
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

    /// QA-STORE-19: a part with nothing to prove its rest (no checksum, no ETag) is not resumed,
    /// and a 416 for such a part is not taken as complete: the file comes whole; a part started
    /// under an ETag is resumed and the tag goes with the finished file.
    #[test]
    fn resume_restarts_on_mismatch() {
        let body: Vec<u8> = (0..50_000u32).map(|i| (i % 241) as u8).collect();
        let srv = serve(vec![
            ("/x", Served::bytes(body.clone())),
            ("/t", Served::bytes(body.clone()).etag("\"v1\"")),
        ]);
        let d = temp("fetch-resume");
        let net = Net::new(true);
        let item = |p: &str, n: &str| Item {
            url: srv.url(p),
            dest: d.join(n),
            sha256: None,
            size: None,
        };
        // A stale half of another file, then a stale part of the full length (416).
        for stale in [vec![9u8; 20_000], vec![9u8; body.len()]] {
            let _ = std::fs::remove_file(d.join("x"));
            std::fs::write(d.join("x.part"), &stale).unwrap();
            let o = one(&net, &item("/x", "x"), &mut |_, _| {});
            assert!(
                matches!(o, Outcome::Fetched { resumed: false, .. }),
                "{o:?}"
            );
            assert_eq!(std::fs::read(d.join("x")).unwrap(), body);
        }
        // Started under an ETag: resumed.
        std::fs::write(d.join("t.part"), &body[..10_000]).unwrap();
        std::fs::write(d.join("t.part.etag"), "\"v1\"").unwrap();
        let o = one(&net, &item("/t", "t"), &mut |_, _| {});
        assert_eq!(
            o,
            Outcome::Fetched {
                bytes: 40_000,
                resumed: true
            }
        );
        assert_eq!(std::fs::read(d.join("t")).unwrap(), body);
        assert!(!d.join("t.part.etag").exists());
        const { assert!(MAX_UNSIZED == 4 << 30) };
    }

    /// QA-STORE-24: a full disk is named, without a path or the system's words, in a download
    /// (the `.part` is /dev/full, which answers every write with ENOSPC) and in a conditional
    /// index fetch; the web library's text is not in "could not be reached".
    #[test]
    fn a_full_disk_is_named_and_library_text_stays_out() {
        let body = vec![7u8; 5000];
        let srv = serve(vec![("/x", Served::bytes(body.clone()))]);
        let d = temp("fetch-full");
        let net = Net::new(true);
        std::os::unix::fs::symlink("/dev/full", d.join("x.part")).unwrap();
        let it = Item {
            url: srv.url("/x"),
            dest: d.join("x"),
            sha256: None,
            size: None,
        };
        let failed = |it: &Item| match one(&net, it, &mut |_, _| {}) {
            Outcome::Failed { message } => message,
            o => panic!("{o:?}"),
        };
        let msg = failed(&it);
        assert_eq!(msg, NO_ROOM);
        assert!(!msg.contains(d.to_str().unwrap()), "{msg}");
        for (kind, raw) in [
            (std::io::ErrorKind::StorageFull, None),
            (std::io::ErrorKind::Other, Some(28)),
        ] {
            let e = match raw {
                Some(n) => std::io::Error::from_raw_os_error(n),
                None => std::io::Error::from(kind),
            };
            assert_eq!(write_error(&e, "x").message, NO_ROOM);
        }
        let other = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            write_error(&other, "A download cannot be written.").message,
            "A download cannot be written."
        );
        let down = Item {
            url: "http://127.0.0.1:1/x".into(),
            dest: d.join("y"),
            sha256: None,
            size: None,
        };
        assert_eq!(failed(&down), "127.0.0.1:1 could not be reached.");
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
            conditional(&net, &srv.url("/core.db"), &dest, MAX_INPUT).unwrap(),
            Outcome::Fetched { bytes: 2, .. }
        ));
        assert_eq!(
            conditional(&net, &srv.url("/core.db"), &dest, MAX_INPUT).unwrap(),
            Outcome::NotModified
        );
    }

    /// C10-2: a body one byte past `max` is an error, its part file is deleted, and an earlier
    /// copy and its validator stay as they were.
    #[test]
    fn an_oversized_input_is_refused_and_the_last_copy_kept() {
        let srv = serve(vec![
            ("/ok", Served::bytes(b"good".to_vec()).etag("\"v1\"")),
            ("/big", Served::bytes(vec![b'x'; 9]).etag("\"v2\"")),
        ]);
        let d = temp("cond-max");
        let net = Net::new(true);
        let dest = d.join("feed");
        let err = conditional(&net, &srv.url("/big"), &dest, 8).unwrap_err();
        assert!(err.message.contains("larger"), "{err}");
        assert!(!dest.exists() && !part_of(&dest).exists());
        assert!(!tag_path(&dest).exists());
        // Exactly `max` is fine.
        assert!(conditional(&net, &srv.url("/ok"), &dest, 4).is_ok());
        assert!(conditional(&net, &srv.url("/big"), &dest, 8).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"good");
        assert_eq!(
            std::fs::read_to_string(tag_path(&dest)).unwrap(),
            "etag \"v1\""
        );
        assert!(!part_of(&dest).exists());
        // In memory: the same bound, and a 304 for the stored validator.
        assert!(get_conditional(&net, &srv.url("/big"), None, 8).is_err());
        assert_eq!(
            get_conditional(&net, &srv.url("/ok"), Some("etag \"v1\""), 4).unwrap(),
            Got::NotModified
        );
        assert_eq!(
            get_conditional(&net, &srv.url("/ok"), None, 4).unwrap(),
            Got::Body {
                bytes: b"good".to_vec(),
                validator: Some("etag \"v1\"".into())
            }
        );
    }
}
