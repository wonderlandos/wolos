// SPDX-License-Identifier: BSD-2-Clause
//! The state `wol-keyringd` serves (ADR 0054 sections 4, 5 and 8): one open keyring, the cache of
//! executable digests, the unlock back-off, the idle lock, and the fast-path socket. The Secret
//! Service on D-Bus ([`crate::secret_service`]) calls the same methods, so both doors enforce the
//! same access lists and write the same audit lines.
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

use crate::factor::Kind;
use crate::item::{AppId, Class, Meta};
use crate::proto::{self, Hit, Reply, Request};
use crate::service::Session;
use crate::{Error, Result, io, wire};

/// Most connections served at once on the fast path.
pub const MAX_CONNECTIONS: usize = 32;
/// Longest wait after wrong passwords, in seconds.
pub const MAX_BACKOFF: u64 = 30;

type FileKey = (u64, u64, i64, u64);

struct Inner {
    session: Option<Session>,
    fails: u32,
    retry_at: Option<std::time::Instant>,
}

/// The daemon's shared state. The digest cache has its own lock: identifying a caller must never
/// wait for the keyring's lock, which an unlock holds for the length of the KDF (ADR 0054
/// section 4, "the fork and exec window").
pub struct Daemon {
    name: String,
    inner: Mutex<Inner>,
    digests: Mutex<HashMap<FileKey, String>>,
}

/// A caller as the kernel showed it at one moment, with the process pinned so the moment can be
/// compared with a later one: the program, its file version and its scope.
pub struct Ident {
    pub app: AppId,
    pinned: io::Pinned,
    exe: String,
    key: (u64, u64, i64, u64),
    scope: Option<String>,
}

impl Ident {
    /// Whether the pinned process is still the program it was when identified. A process that
    /// executed another program, whose file was replaced, whose process ended, or which moved
    /// to another scope is not: its requests are refused.
    pub fn unchanged(&self) -> bool {
        self.pinned.exe().as_deref() == Some(self.exe.as_str())
            && self.pinned.exe_key() == Some(self.key)
            && self.pinned.app() == self.scope
    }
}

/// Seconds to wait before the next password try after `fails` wrong ones: none for the first
/// three, then 1, 2, 4 ... up to [`MAX_BACKOFF`].
pub fn backoff(fails: u32) -> u64 {
    if fails < 3 {
        0
    } else {
        (1u64 << (fails - 3).min(5)).min(MAX_BACKOFF)
    }
}

impl Daemon {
    pub fn new(name: &str) -> Daemon {
        Daemon {
            name: name.into(),
            inner: Mutex::new(Inner {
                session: None,
                fails: 0,
                retry_at: None,
            }),
            digests: Mutex::new(HashMap::new()),
        }
    }

    fn guard(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The open session after taking in what the command line wrote since the last request.
    fn session<'a>(&self, g: &'a mut Inner) -> Result<&'a mut Session> {
        if g.session.is_none() {
            g.session = Some(Session::open(&self.name)?);
        }
        let s = g.session.as_mut().ok_or(Error::NotFound)?;
        s.sync()?;
        Ok(s)
    }

    /// Identifies a pinned process: program, file version, scope, and the executable's digest
    /// (read once per file version). Takes no keyring lock. None when the program cannot be read
    /// or its file changed while it was hashed.
    pub fn identify(&self, pinned: io::Pinned) -> Option<Ident> {
        let exe = pinned.exe()?;
        let key = pinned.exe_key()?;
        let scope = pinned.app();
        let cached = self
            .digests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .cloned();
        let sha256 = match cached {
            Some(d) => d,
            None => {
                let d = io::sha256_file(&pinned.at("exe"))?;
                let mut c = self.digests.lock().unwrap_or_else(|p| p.into_inner());
                if c.len() > 4096 {
                    c.clear();
                }
                c.insert(key, d.clone());
                d
            }
        };
        let id = Ident {
            app: AppId {
                exe: exe.clone(),
                sha256: Some(sha256),
                app: scope.clone(),
                signer: None,
            },
            pinned,
            exe,
            key,
            scope,
        };
        id.unchanged().then_some(id)
    }

    /// Opens the keyring with the machine factors at sign-in (the TPM device key); a failure
    /// leaves it locked for the lock screen or `wol-login` to unlock with the password.
    pub fn unlock_at_start(&self) -> Result<()> {
        let mut g = self.guard();
        let s = self.session(&mut g)?;
        if !s
            .keyring
            .header()
            .factors
            .iter()
            .any(|f| f.kind == Kind::Tpm)
        {
            return Err(Error::Policy("this keyring has no device key"));
        }
        s.unlock(&[], true, "wol-keyringd").map(|_| ())
    }

    /// Whether the keyring is unlocked, and its item count.
    pub fn status(&self) -> Result<(bool, usize)> {
        let mut g = self.guard();
        let s = self.session(&mut g)?;
        Ok((s.keyring.unlocked(), s.keyring.len()))
    }

    /// Unlocks with the login password; wrong tries back off.
    pub fn unlock(&self, password: &str, app: &str) -> Result<()> {
        let mut g = self.guard();
        if g.retry_at.is_some_and(|t| std::time::Instant::now() < t) {
            return Err(Error::Policy("too many wrong passwords, wait a moment"));
        }
        let s = self.session(&mut g)?;
        let r = s.unlock(
            &[(Kind::Password, password), (Kind::Recovery, password)],
            false,
            app,
        );
        match r {
            Ok(_) => {
                g.fails = 0;
                g.retry_at = None;
                Ok(())
            }
            Err(e) => {
                g.fails += 1;
                // From the end of the try: the KDF's own time does not count against the wait.
                g.retry_at =
                    Some(std::time::Instant::now() + Duration::from_secs(backoff(g.fails)));
                Err(e)
            }
        }
    }

    /// Locks (screen lock, sleep, idle, a client's request).
    pub fn lock(&self, why: &str) -> Result<()> {
        let mut g = self.guard();
        match g.session.as_mut() {
            Some(s) => s.lock(why),
            None => Ok(()),
        }
    }

    /// The idle lock: called every few seconds.
    pub fn tick(&self) {
        let mut g = self.guard();
        if let Some(s) = g.session.as_mut() {
            let after = s.keyring.header().settings.lock_after_secs;
            if s.keyring.unlocked() && after > 0 && io::now().saturating_sub(s.last_use) >= after {
                let _ = s.lock("idle");
            } else {
                let _ = s.sync().and_then(|_| s.checkpoint_if_due());
            }
        }
    }

    /// Locks on sleep or the screen lock when the settings say so.
    pub fn lock_event(&self, sleep: bool) {
        let mut g = self.guard();
        if let Some(s) = g.session.as_mut() {
            let st = &s.keyring.header().settings;
            if (sleep && st.lock_on_sleep) || (!sleep && st.lock_with_screen) {
                let _ = s.lock(if sleep { "sleep" } else { "screen" });
            }
        }
    }

    /// Items whose attributes include `q`; needs the first unlock.
    pub fn search(&self, q: &BTreeMap<String, String>) -> Result<Vec<Meta>> {
        let mut g = self.guard();
        let s = self.session(&mut g)?;
        if !s.keyring.first_unlocked() {
            return Err(Error::Locked);
        }
        Ok(s.keyring.search(q).into_iter().cloned().collect())
    }

    /// One item's metadata.
    pub fn meta(&self, id: &str) -> Result<Meta> {
        let mut g = self.guard();
        let s = self.session(&mut g)?;
        if !s.keyring.first_unlocked() {
            return Err(Error::Locked);
        }
        s.keyring.meta(id).cloned().ok_or(Error::NotFound)
    }

    /// The secret of `id` for `caller` (an unknown caller gets nothing).
    pub fn get(&self, caller: Option<&AppId>, id: &str) -> Result<(Zeroizing<Vec<u8>>, String)> {
        let caller = caller.ok_or(Error::Denied)?;
        let mut g = self.guard();
        let s = self.session(&mut g)?;
        let ct = s
            .keyring
            .meta(id)
            .map(|m| m.content_type.clone())
            .unwrap_or_else(|| "text/plain".into());
        Ok((s.get(Some(caller), id)?, ct))
    }

    /// Stores a secret for `caller`.
    pub fn set(
        &self,
        caller: Option<&AppId>,
        label: &str,
        attrs: BTreeMap<String, String>,
        secret: &[u8],
        class: Class,
    ) -> Result<String> {
        let caller = caller.ok_or(Error::Denied)?;
        let mut g = self.guard();
        let s = self.session(&mut g)?;
        s.set(Some(caller), label, attrs, secret, class)
    }

    /// Replaces the secret of an existing item (the Secret Service's SetSecret).
    pub fn replace(&self, caller: Option<&AppId>, id: &str, secret: &[u8]) -> Result<()> {
        let m = self.meta(id)?;
        self.set(caller, &m.label, m.attrs, secret, m.class)
            .map(|_| ())
    }

    /// Deletes an item for `caller`.
    pub fn delete(&self, caller: Option<&AppId>, id: &str) -> Result<()> {
        let caller = caller.ok_or(Error::Denied)?;
        let mut g = self.guard();
        let s = self.session(&mut g)?;
        s.delete(Some(caller), id)
    }

    /// The last `limit` audit lines.
    pub fn audit_tail(&self, limit: usize) -> Result<Vec<String>> {
        let mut g = self.guard();
        let s = self.session(&mut g)?;
        let text = s.audit_text()?;
        let lines: Vec<&str> = text.lines().collect();
        let from = lines.len().saturating_sub(limit.min(1000));
        Ok(lines[from..].iter().map(|l| l.to_string()).collect())
    }

    /// Answers one request from a pinned caller: refused when the process is no longer the
    /// program that was identified, before the work and again after it (a reply is dropped, not
    /// sent, when the process changed while the work ran).
    pub fn handle_as(&self, who: Option<&Ident>, req: Request) -> Reply {
        if who.is_some_and(|i| !i.unchanged()) {
            return Reply::err(&Error::Denied);
        }
        let r = self.handle(who.map(|i| &i.app), req);
        if who.is_some_and(|i| !i.unchanged()) {
            return Reply::err(&Error::Denied);
        }
        r
    }

    /// Answers one fast-path request from `caller`.
    pub fn handle(&self, caller: Option<&AppId>, req: Request) -> Reply {
        match self.handle_inner(caller, req) {
            Ok(r) => r,
            Err(e) => Reply::err(&e),
        }
    }

    fn handle_inner(&self, caller: Option<&AppId>, req: Request) -> Result<Reply> {
        let who = caller
            .map(|c| c.display().to_string())
            .unwrap_or_else(|| "unknown".into());
        let mut r = Reply::ok();
        match req {
            Request::Status => {
                let (u, n) = self.status()?;
                r.unlocked = Some(u);
                r.items = Some(n);
            }
            Request::Unlock { password } => {
                let p = Zeroizing::new(password);
                self.unlock(&p, &who)?;
            }
            Request::Lock => self.lock("request")?,
            Request::Search { attrs } => {
                let mut hits: Vec<Hit> = self
                    .search(&attrs)?
                    .into_iter()
                    .take(proto::MAX_RESULTS)
                    .map(|m| Hit {
                        id: m.id,
                        label: m.label,
                        attrs: m.attrs,
                        modified: m.modified,
                    })
                    .collect();
                hits.sort_by(|a, b| a.id.cmp(&b.id));
                r.hits = Some(hits);
            }
            Request::Get { id } => {
                let (s, _) = self.get(caller, &id)?;
                r.secret = Some(wire::b64(&s));
            }
            Request::Set {
                label,
                attrs,
                secret,
                class,
            } => {
                let secret = Zeroizing::new(secret);
                let bytes = Zeroizing::new(wire::unb64(&secret, crate::item::MAX_SECRET)?);
                r.id = Some(self.set(caller, &label, attrs, &bytes, class)?);
            }
            Request::Delete { id } => self.delete(caller, &id)?,
            Request::Audit { limit } => r.lines = Some(self.audit_tail(limit)?),
        }
        Ok(r)
    }
}

/// Makes the fast-path socket: private folder, no second daemon, mode 0600.
pub fn bind(path: &std::path::Path) -> Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().ok_or(Error::Malformed("socket path"))?;
    std::fs::create_dir_all(dir).map_err(|e| Error::Io(format!("socket folder: {e}")))?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(Error::Io("wol-keyringd is already running".into()));
        }
        std::fs::remove_file(path).map_err(|e| Error::Io(format!("old socket: {e}")))?;
    }
    let l = UnixListener::bind(path).map_err(|e| Error::Io(format!("socket: {e}")))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| Error::Io(format!("socket mode: {e}")))?;
    Ok(l)
}

/// Serves the fast path until the process ends.
pub fn serve(d: Arc<Daemon>, l: UnixListener) {
    let live = Arc::new(AtomicUsize::new(0));
    for c in l.incoming() {
        let Ok(c) = c else { continue };
        if live.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            live.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let (d, live) = (d.clone(), live.clone());
        std::thread::spawn(move || {
            connection(&d, c);
            live.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

fn connection(d: &Daemon, c: UnixStream) {
    let t = Some(Duration::from_millis(proto::TIMEOUT_MS));
    let _ = c.set_read_timeout(t);
    let _ = c.set_write_timeout(t);
    // The caller is whoever the kernel says, pinned and identified now, before anything that can
    // wait; another user's process is refused. Every request re-checks that the process is still
    // the program that was identified.
    let me = shared::sys::euid();
    let who = io::Pinned::of(&c)
        .filter(|p| p.uid == me)
        .and_then(|p| d.identify(p));
    let mut rd = LineReader::default();
    loop {
        let (line, too_long) = match rd.next(&c) {
            Some(x) => x,
            None => return,
        };
        let mut reply = if too_long {
            Reply::err(&Error::Limit("request line"))
        } else {
            match serde_json::from_slice::<Request>(&line) {
                Ok(req) => d.handle_as(who.as_ref(), req),
                Err(_) => Reply::err(&Error::Malformed("request")),
            }
        };
        let mut out = Zeroizing::new(serde_json::to_vec(&reply).unwrap_or_default());
        if let Some(s) = reply.secret.as_mut() {
            s.zeroize();
        }
        out.push(b'\n');
        let changed = who.as_ref().is_some_and(|i| !i.unchanged());
        if (&c).write_all(&out).is_err() || too_long || changed {
            return;
        }
    }
}

/// Reads request lines from a socket into buffers that are wiped when dropped (the password of an
/// unlock and the secret of a set pass through here). A line over [`proto::MAX_LINE`] is reported
/// and ends the connection.
#[derive(Default)]
struct LineReader {
    pending: Zeroizing<Vec<u8>>,
}

impl LineReader {
    fn next(&mut self, mut c: &UnixStream) -> Option<(Zeroizing<Vec<u8>>, bool)> {
        loop {
            if let Some(i) = self.pending.iter().position(|b| *b == b'\n') {
                if i > proto::MAX_LINE {
                    return Some((Zeroizing::new(Vec::new()), true));
                }
                let rest = self.pending.split_off(i + 1);
                let mut line = std::mem::replace(&mut self.pending, Zeroizing::new(rest));
                line.pop();
                return Some((line, false));
            }
            if self.pending.len() > proto::MAX_LINE {
                return Some((Zeroizing::new(Vec::new()), true));
            }
            let mut chunk = Zeroizing::new([0u8; 4096]);
            match c.read(&mut chunk[..]) {
                Ok(0) | Err(_) => return None,
                Ok(n) => self.pending.extend_from_slice(&chunk[..n]),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff(0), 0);
        assert_eq!(backoff(2), 0);
        assert_eq!(backoff(3), 1);
        assert_eq!(backoff(4), 2);
        assert_eq!(backoff(5), 4);
        assert_eq!(backoff(100), MAX_BACKOFF);
    }
}
