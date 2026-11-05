// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-admind request handling (IaraOS ADR 0026): allow-list only, judged by peer credentials.
use shared::admin::{AdminAction, AdminReply, decode};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use wol_core::admin::{
    AdminDecision, AdminStep, AuthCache, decide, describe, is_admin, needs_fresh_password,
    plan_with, validate,
};
use wol_core::package::{self, Trust};

/// Everything the daemon needs from the OS; faked in tests.
pub trait AdminOs {
    fn passwd(&self) -> String;
    fn group(&self) -> String;
    fn authenticate(&self, user: &str, password: &[u8]) -> bool;
    fn run(&self, argv: &[String], long: bool) -> bool;
    /// Fixed argv with `input` on stdin (secrets never in argv).
    fn run_input(&self, argv: &[String], input: &str) -> bool;
    /// Long install, its percentage published to `path` (`wol_core::progress`).
    fn run_progress(&self, argv: &[String], kind: wol_core::progress::Kind, path: &str) -> bool;
    fn write(&self, path: &str, mode: u32, text: &str) -> bool;
    fn read(&self, path: &str) -> Option<String>;
    /// Fixed delay before every refusal.
    fn refuse_delay(&self);
    fn now(&self) -> u64;
    fn log(&self, text: &str);
    /// Accounts with a desktop session on this machine (a compositor socket in their runtime dir).
    fn sessions(&self) -> Vec<u32>;
    /// Fixed argv, stdout; None if it failed.
    fn capture(&self, argv: &[String]) -> Option<String>;
    /// What the last failed `run`/`capture` said on stderr (its last lines), taken. For the log:
    /// "step failed" alone does not tell what to fix (live-4 F1).
    fn last_error(&self) -> String {
        String::new()
    }
    /// Another update holds [`UPDATE_LOCK`] (a `wol update apply`, a `sudo pacman -Syu` through
    /// the slot wrapper): Update Now must not start a second slot build beside it (QA-100 STORE-4).
    fn update_locked(&self) -> bool {
        false
    }
    /// Take [`UPDATE_LOCK`] for the whole of one Update Now (QA-100 STORE-4): wol-admind runs the
    /// slot builder's steps itself, so it is the builder that holds it. Ok(None): another holds it.
    fn hold_update_lock(&self) -> Result<Option<Box<dyn std::any::Any>>, String> {
        Ok(Some(Box::new(())))
    }
    /// The newer WolOS release the daily check found and its signature vouches for, as (version,
    /// snapshot date `YYYY/MM/DD`); None when there is none, it is already running, or it is not
    /// signed by a trusted key. Update Now installs it with the packages (QA-100 STORE-2).
    fn trusted_release(&self) -> Option<(String, String)> {
        None
    }
    /// Wall-clock seconds (restore point names).
    fn wall_now(&self) -> u64;
    /// Fresh root-only directory under [`shared::package::STAGING_DIR`] for one install.
    fn stage_dir(&self) -> Option<PathBuf>;
    /// Copy the caller's `.wol` into `dst`: a regular file the caller may read, at most
    /// [`shared::package::MAX_SIZE`].
    fn copy_in(&self, src: &str, dst: &Path, caller: Option<u32>) -> Result<(), String>;
    /// Ed25519 keys in [`shared::package::KEYS_DIR`].
    fn trusted_keys(&self) -> Vec<[u8; 32]>;
    fn remove_dir(&self, dir: &Path);
    /// Name, version and SHA-256 of wold's staged package copy (ADR 0034,
    /// [`crate::packages::inspect_staged`]); Err: why it cannot be read.
    fn package_facts(&self, staged: &str) -> Result<crate::packages::PkgFacts, String>;
}

/// (name, gid) of `uid` from passwd(5) text.
pub fn user_of(passwd: &str, uid: u32) -> Option<(String, u32)> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 4 && f[2].parse() == Ok(uid))
            .then(|| Some((f[0].to_owned(), f[3].parse().ok()?)))?
    })
}

/// Peer identity: uid and session id (getsid of the peer pid).
#[derive(Clone, Copy, Debug)]
pub struct Caller {
    pub uid: u32,
    pub session: i32,
}

/// Longest a client may take to send its whole request (audit S4-7): one deadline for the
/// connection, not per read, so a byte every few seconds cannot hold wol-admind. The hello
/// before it has its own bound (`shared::jobs::HELLO_TIMEOUT`).
pub const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);
/// Longest the password check may take (a stalled PAM stack, a faillock delay): past it the
/// helper is killed and the password counts as not accepted.
pub const AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Connections served at once, per account and in total (each on its own thread).
pub const CONN_PER_UID: usize = 4;
pub const CONN_TOTAL: usize = 32;

/// Read a request from `conn` until EOF, `buf` is full or `deadline` has passed since the call;
/// the byte count. The read timeout is what is left of the deadline before every read.
pub fn read_request(
    conn: &std::os::unix::net::UnixStream,
    buf: &mut [u8],
    deadline: std::time::Duration,
) -> usize {
    use std::io::Read;
    let start = std::time::Instant::now();
    let mut n = 0;
    while n < buf.len() {
        let Some(left) = deadline
            .checked_sub(start.elapsed())
            .filter(|d| !d.is_zero())
        else {
            break;
        };
        if conn.set_read_timeout(Some(left)).is_err() {
            break;
        }
        match (&*conn).read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(r) => n += r,
        }
    }
    n
}

/// What every connection thread shares (api-security 1+2). The authorization cache is locked
/// only to read or change it: the refusal delay, the password check and the commands run with
/// no lock other connections need, so one account looping bad requests (each one sleeping
/// [`wol_core::admin::REFUSAL_DELAY_MS`]) or a long install never holds another's request.
#[derive(Default)]
pub struct Shared {
    cache: Mutex<Cached>,
    /// One password check (and its refusal delay) at a time per account: guessing runs no
    /// faster with more connections, and other accounts never wait for it.
    auth: Mutex<HashMap<u32, Arc<Mutex<()>>>>,
    /// Commands still run one at a time per [`Lane`].
    system: Mutex<()>,
    settings: Mutex<()>,
}

#[derive(Default)]
struct Cached {
    cache: AuthCache,
    /// Sessions with a request under way ([`Watching`]): how many requests, and the revokes of
    /// that session since the first began. Per (uid, session), so one account's revokes never
    /// touch another's; an entry goes with its last request, so the table is never larger than
    /// the connections.
    watched: HashMap<(u32, i32), Watch>,
}

#[derive(Default)]
struct Watch {
    requests: u32,
    revokes: u64,
}

/// One request watching the revokes of its own session, from its decision to its end: a
/// password accepted, or a command that waited for its lane on the cached authorization, while
/// a revoke of that session went by does not count.
struct Watching<'a> {
    shared: &'a Shared,
    key: (u32, i32),
    seen: u64,
}

impl<'a> Watching<'a> {
    /// Registered under the cache lock `c`, at the moment the cache is read.
    fn new(shared: &'a Shared, c: &mut Cached, key: (u32, i32)) -> Self {
        let w = c.watched.entry(key).or_default();
        w.requests += 1;
        Self {
            shared,
            key,
            seen: w.revokes,
        }
    }
    /// Whether the session was revoked since this request's decision.
    fn revoked(&self, c: &Cached) -> bool {
        c.watched
            .get(&self.key)
            .is_none_or(|w| w.revokes != self.seen)
    }
}

// Unregisters the request: the session's entry goes with its last one.
impl Drop for Watching<'_> {
    fn drop(&mut self) {
        let mut c = self.shared.cached();
        if let Some(w) = c.watched.get_mut(&self.key) {
            w.requests -= 1;
            if w.requests == 0 {
                c.watched.remove(&self.key);
            }
        }
    }
}

/// Which commands may run side by side: the package manager, updates and boot entries are one
/// lane (one pacman lock, one boot loader); every other action (consent, Wi-Fi, power, ...) is
/// the other, so a 30-minute install does not hold a Wi-Fi join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    System,
    Settings,
}

/// The [`Lane`] of `a`.
pub fn lane(a: AdminAction) -> Lane {
    if wol_core::admin::LONG_ACTIONS.contains(&a)
        || matches!(
            a,
            AdminAction::PkgRefresh
                | AdminAction::SystemChannel
                | AdminAction::BootEnvironment
                | AdminAction::BootEnroll
        )
    {
        Lane::System
    } else {
        Lane::Settings
    }
}

/// `m` locked; a thread that panicked holding it leaves plain data, still usable.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Shared {
    fn cached(&self) -> MutexGuard<'_, Cached> {
        lock(&self.cache)
    }
    /// The per-account password lock of `uid`.
    fn auth_of(&self, uid: Option<u32>) -> Arc<Mutex<()>> {
        lock(&self.auth)
            .entry(uid.unwrap_or(u32::MAX))
            .or_default()
            .clone()
    }
    fn lane(&self, l: Lane) -> MutexGuard<'_, ()> {
        lock(match l {
            Lane::System => &self.system,
            Lane::Settings => &self.settings,
        })
    }
}

/// One connection, on its own thread: identity from the kernel at accept, the ADR 0033 hello,
/// the request within [`REQUEST_DEADLINE`], then [`handle`] (which takes the [`Shared`] locks
/// only as long as it needs them), the reply, and the buffer wiped.
pub fn serve_one(conn: std::os::unix::net::UnixStream, shared: &Shared, os: &dyn AdminOs) {
    use std::io::Write;
    let caller = shared::sys::peer(&conn).map(|p| Caller {
        uid: p.uid,
        session: session_of(p.pid),
    });
    // ADR 0033: greet first; a request written before accept, or an echo from another process
    // than the one that connected, is dropped without an answer.
    if shared::jobs::greet(&conn).is_err() {
        return;
    }
    let mut buf = vec![0u8; shared::admin::MAX_REQUEST + 1];
    let n = read_request(&conn, &mut buf, REQUEST_DEADLINE);
    let mut said = None;
    let reply = handle_said(&buf[..n], caller, shared, os, &mut said);
    buf.fill(0); // password never outlives the request
    let _ = conn.set_write_timeout(Some(REQUEST_DEADLINE));
    let _ = writeln!(&conn, "{}", reply_line(reply, said.as_deref()));
}

/// The reply line: a failure with a sentence for the person carries it (append-only wire).
pub fn reply_line(reply: AdminReply, said: Option<&str>) -> String {
    match (reply, said) {
        (AdminReply::Failed, Some(m)) => shared::admin::failed_line(m),
        _ => reply.line().to_owned(),
    }
}

/// What a connection over its quota is told, after the hello: the service is running, only busy
/// (QA-100 SHADOW-41; before, the client read a closed socket as "administrator service
/// unavailable").
pub const BUSY: &str = "wol-admind is busy with other requests. Try again in a moment.";
/// Connections over quota answered at once (each waits for its hello on its own thread).
const BUSY_REPLIES: usize = 8;
/// An `accept` error or an over-quota refusal is logged at most this often (seconds); the loop waits [`ACCEPT_BACKOFF`].
const ACCEPT_LOG_EVERY: u64 = 30;
/// Wait after an `accept` error (out of descriptors): never a busy loop.
pub const ACCEPT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

/// Says [`BUSY`] to a connection over its quota: greet, read what it sends (so the close does not
/// reset the reply), answer. On its own short-lived thread; past [`BUSY_REPLIES`] at once the
/// connection is just closed.
fn refuse_busy(conn: std::os::unix::net::UnixStream) {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ANSWERING: AtomicUsize = AtomicUsize::new(0);
    if ANSWERING.fetch_add(1, Ordering::SeqCst) >= BUSY_REPLIES {
        ANSWERING.fetch_sub(1, Ordering::SeqCst);
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("admind-busy".into())
        .spawn(move || {
            if shared::jobs::greet(&conn).is_ok() {
                let mut buf = vec![0u8; shared::admin::MAX_REQUEST + 1];
                let n = read_request(&conn, &mut buf, REQUEST_DEADLINE);
                buf[..n].fill(0);
                let _ = conn.set_write_timeout(Some(REQUEST_DEADLINE));
                let _ = writeln!(&conn, "{}", shared::admin::failed_line(BUSY));
            }
            ANSWERING.fetch_sub(1, Ordering::SeqCst);
        });
    if spawned.is_err() {
        ANSWERING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// wol-admind's accept loop (audit S4-7): each connection on its own thread, at most
/// [`CONN_PER_UID`] per account and [`CONN_TOTAL`] in all, so one idle or slow client never
/// holds the others; one over the quota is told [`BUSY`]. Never returns.
pub fn serve(listener: &std::os::unix::net::UnixListener, os: &'static (dyn AdminOs + Sync)) {
    serve_with(|| listener.accept().map(|c| c.0), os, || false);
}

/// [`serve`] over any source of connections; returns when `done` says so after an iteration.
/// An `accept` error (descriptors used up, say) is logged, once per [`ACCEPT_LOG_EVERY`] seconds,
/// and waited out for [`ACCEPT_BACKOFF`] (QA-100 SHADOW-20: the loop used to spin on it, silent).
pub fn serve_with(
    accept: impl FnMut() -> std::io::Result<std::os::unix::net::UnixStream>,
    os: &'static (dyn AdminOs + Sync),
    done: impl FnMut() -> bool,
) {
    serve_started(accept, os, done, |what, work| {
        shared::bounded::spawn_conn(what, &|l| os.log(l), work)
    });
}

/// [`serve_with`] with the thread start given (`start(what, work)` says whether the thread
/// began), so a test can refuse one. A connection whose thread the system refused is told
/// [`BUSY`] like one over its quota (it used to be closed before the hello, and `wol` said
/// "administrator service unavailable" while the daemon ran, QA-A9-SEC-7); the start waits
/// [`shared::bounded::SPAWN_BACKOFF`] and logs, so the loop does not spin on the shortage.
pub fn serve_started(
    mut accept: impl FnMut() -> std::io::Result<std::os::unix::net::UnixStream>,
    os: &'static (dyn AdminOs + Sync),
    mut done: impl FnMut() -> bool,
    start: impl Fn(&str, Box<dyn FnOnce() + Send>) -> bool,
) {
    let shared = Arc::new(Shared::default());
    let quota = shared::bounded::Quota::new(CONN_PER_UID, CONN_TOTAL);
    let mut logged: Option<std::time::Instant> = None;
    let mut over_logged: Option<std::time::Instant> = None;
    let mut failures = 0u64;
    while !done() {
        let conn = match accept() {
            Ok(c) => {
                failures = 0;
                c
            }
            Err(e) => {
                failures += 1;
                if logged.is_none_or(|t| t.elapsed().as_secs() >= ACCEPT_LOG_EVERY) {
                    logged = Some(std::time::Instant::now());
                    os.log(&format!(
                        "accept failed ({failures} in a row): {e}; waiting before the next try"
                    ));
                }
                std::thread::sleep(ACCEPT_BACKOFF);
                continue;
            }
        };
        let Some(peer) = shared::sys::peer(&conn) else {
            continue;
        };
        let Some(slot) = quota.admit(peer.uid) else {
            if over_logged.is_none_or(|t| t.elapsed().as_secs() >= ACCEPT_LOG_EVERY) {
                over_logged = Some(std::time::Instant::now());
                os.log(&format!(
                    "uid {}: over its connection limit; told busy",
                    peer.uid
                ));
            }
            refuse_busy(conn);
            continue;
        };
        let shared = shared.clone();
        let for_busy = conn.try_clone().ok();
        let began = start(
            "admind-conn",
            Box::new(move || {
                serve_one(conn, &shared, os);
                drop(slot);
            }),
        );
        if !began && let Some(c) = for_busy {
            refuse_busy(c);
        }
    }
}

/// Run a password helper with `password` on its standard input; true only when it exits 0
/// within `timeout`. Past it the helper is killed (audit S4-7: a stalled PAM stack used to
/// freeze wol-admind for everyone).
pub fn check_password(
    mut cmd: std::process::Command,
    password: &[u8],
    timeout: std::time::Duration,
) -> bool {
    use std::io::Write;
    use std::process::Stdio;
    let Ok(mut child) = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // Dropped at the end of the statement: the helper sees EOF after the password.
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut s| s.write_all(password).is_ok());
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success() && wrote,
            Ok(None) if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            // Past the timeout, or the child cannot be waited on: the password is not accepted.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Judge and run one request. Never logs the password or keybind JSON. Safe to call from many
/// threads at once: see [`Shared`].
pub fn handle(
    data: &[u8],
    caller: Option<Caller>,
    shared: &Shared,
    os: &dyn AdminOs,
) -> AdminReply {
    handle_said(data, caller, shared, os, &mut None)
}

/// [`handle`], and the sentence a failed request leaves for the person in `said` (the optional
/// message of the failed reply, `shared::admin::failed_line`; today Update Now's). Many
/// connections at once (QA-100 H1): the cache is locked only to read, grant or revoke; the
/// refusal delay, the password check and the steps run outside it, the steps in their lane.
pub fn handle_said(
    data: &[u8],
    caller: Option<Caller>,
    shared: &Shared,
    os: &dyn AdminOs,
    said: &mut Option<String>,
) -> AdminReply {
    let uid = caller.map(|c| c.uid);
    let session = caller.map_or(-1, |c| c.session);
    let passwd = os.passwd();
    // QA-100 SHADOW-20: an unreadable account list denies everyone; the log must say so, not
    // "not an administrator" for each.
    if passwd.trim().is_empty() {
        os.log("the account list (/etc/passwd) is unreadable or empty; every request is refused");
    }
    let user = uid.and_then(|u| user_of(&passwd, u));
    let who = match (&user, uid) {
        (Some((n, _)), Some(u)) => format!("{n}({u})"),
        (_, Some(u)) => format!("uid {u}"),
        _ => "unknown peer".into(),
    };
    let refuse = |r: AdminReply| {
        os.refuse_delay();
        r
    };
    let Some((req, pw)) = decode(data).filter(|(r, _)| validate(r).is_none()) else {
        os.log(&format!("{who}: invalid request"));
        return refuse(AdminReply::Invalid);
    };
    let what = describe(&req);
    if req.action == AdminAction::Revoke {
        if let Some(u) = uid {
            let mut c = shared.cached();
            c.cache.revoke(u, session);
            if let Some(w) = c.watched.get_mut(&(u, session)) {
                w.revokes += 1;
            }
        }
        return AdminReply::Ok;
    }
    let admin = user
        .as_ref()
        .is_some_and(|(n, g)| is_admin(n, *g, &os.group()));
    let known = if uid == Some(0) {
        uid
    } else {
        user.as_ref().and(uid)
    };
    let (cached, watch) = {
        let mut c = shared.cached();
        let valid = !needs_fresh_password(&req)
            && session > 0
            && uid.is_some_and(|u| c.cache.valid(u, session, os.now()));
        let watch = uid
            .filter(|_| session > 0)
            .map(|u| Watching::new(shared, &mut c, (u, session)));
        (valid, watch)
    };
    let user_level = known.is_some_and(|u| wol_core::admin::user_level_ok(&req, u, &os.sessions()));
    // Anyone may change their own password, with the current one (fresh: never the cache).
    let own_password = req.action == AdminAction::UserPassword
        && user.as_ref().is_some_and(|(n, _)| *n == req.args[0]);
    let decision = decide(
        known,
        admin || user_level || own_password,
        cached || user_level,
        !pw.is_empty(),
    );
    // Allowed on the session's cached authorization alone (not root, not a user-level action).
    let on_cache = decision == AdminDecision::Allow && cached && !user_level && known != Some(0);
    match decision {
        AdminDecision::NotAdmin => {
            log_not_admin(os, &req, known, &who, &what);
            return refuse(AdminReply::NotAdmin);
        }
        AdminDecision::NeedPassword => return AdminReply::NeedsAuth,
        AdminDecision::Authenticate => {
            let name = user.as_ref().map_or("", |(n, _)| n.as_str());
            let account = shared.auth_of(uid);
            let _one_at_a_time = lock(&account);
            if !os.authenticate(name, &data[pw]) {
                os.log(&format!("{who}: {what}: authentication failed"));
                return refuse(AdminReply::Denied);
            }
            if session > 0
                && let Some(u) = uid
            {
                let mut c = shared.cached();
                if !watch.as_ref().is_some_and(|w| w.revoked(&c)) {
                    c.cache.grant(u, session, os.now());
                }
            }
        }
        AdminDecision::Allow => {}
    }
    let _lane = shared.lane(lane(req.action));
    // A command may wait long for its lane (an install before it): on the cache, it runs only
    // if the authorization still holds now and its session was not revoked meanwhile.
    if on_cache {
        let c = shared.cached();
        let holds = uid.is_some_and(|u| c.cache.valid(u, session, os.now()))
            && !watch.as_ref().is_some_and(|w| w.revoked(&c));
        if !holds {
            drop(c);
            os.log(&format!("{who}: {what}: authorization ended while waiting"));
            return AdminReply::NeedsAuth;
        }
    }
    // A wrong Wi-Fi password must not leave the profile the join just made: asked before the
    // steps run, while the profiles are as they were (QA-100 NET2).
    let cleanup = wol_core::admin::wifi_join_cleanup(&req, &|argv: &[&str]| {
        os.capture(&argv.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    });
    let ok = match req.action {
        AdminAction::WolInstall => wol_install(os, &req.args[0], &req.args[1], uid),
        AdminAction::WolRemove => wol_remove(os, &req.args[0]),
        AdminAction::PkgInstallFile => pkg_install_file(os, &req, &who),
        AdminAction::PkgUpgrade => update_now(os, &req.args, &who)
            .map_err(|m| *said = Some(m))
            .is_ok(),
        _ => execute(&req, os),
    };
    finish(os, &req, ok, cleanup, (&who, &what), said)
}

/// The log line of a refused request; a restart or shut down from the session refused because
/// another account would lose its session (the lock screen's Restart, audit6 S6-5) names who is
/// signed in.
fn log_not_admin(
    os: &dyn AdminOs,
    req: &shared::admin::AdminRequest,
    known: Option<u32>,
    who: &str,
    what: &str,
) {
    let others = known
        .filter(|_| wol_core::admin::is_user_level(req.action))
        .map(|u| wol_core::admin::signed_in_others(req, u, &os.sessions()))
        .unwrap_or_default();
    if others.is_empty() {
        os.log(&format!("{who}: {what}: not an administrator"));
        return;
    }
    let passwd = os.passwd();
    let names: Vec<String> = others
        .iter()
        .map(|u| match user_of(&passwd, *u) {
            Some((n, _)) => format!("{n}({u})"),
            None => format!("uid {u}"),
        })
        .collect();
    os.log(&format!(
        "{who}: {what}: refused, signed in: {}",
        names.join(", ")
    ));
}

/// After the steps ran (`ok`): the reason, the Wi-Fi cleanup, the log line, the person's sentence
/// in `said` and the reply.
fn finish(
    os: &dyn AdminOs,
    req: &shared::admin::AdminRequest,
    ok: bool,
    cleanup: Option<Vec<String>>,
    (who, what): (&str, &str),
    said: &mut Option<String>,
) -> AdminReply {
    // Why it failed, taken before anything else runs a command: the log and the person get the
    // program's own last line, not "the change failed" alone (QA-100 LIVE3-19, DEV-4).
    let reason = if ok {
        None
    } else {
        reason_of(&os.last_error())
    };
    // After the reason is taken: the cleanup is a command too and would replace it.
    if let Some(argv) = cleanup.filter(|_| !ok) {
        os.run(&argv, false);
    }
    os.log(&format!(
        "{who}: {what}: {}",
        match (ok, &reason) {
            (true, _) => "ok".to_owned(),
            (false, Some(r)) => format!("failed: {r}"),
            (false, None) => "failed".to_owned(),
        }
    ));
    if !ok && said.is_none() {
        *said = reason.map(|r| format!("{}: {r}", shared::admin::AdminReply::Failed.message()));
    }
    if ok && req.action == AdminAction::PkgInstall {
        drop_gave_up(os, &req.args[0]);
    }
    if ok {
        AdminReply::Ok
    } else {
        AdminReply::Failed
    }
}

/// `package` is installed: modules-install no longer has to be asked to offer it again, so its
/// line goes from the gave-up list (ADR 0032, `wol_core::modules::GAVE_UP_FILE`). A missing or
/// unchanged list is left alone; the lines of other names stay.
fn drop_gave_up(os: &dyn AdminOs, package: &str) {
    let file = wol_core::modules::GAVE_UP_FILE;
    let Some(text) = os.read(file) else {
        return;
    };
    if let Some(kept) = wol_core::modules::without_gave_up(&text, package)
        && !os.write(file, 0o644, &kept)
    {
        os.log(&format!("{file}: could not drop {package}"));
    }
}

/// A locally built package (ADR 0034): only a staged copy that still has the SHA-256 the sheet
/// showed (`args[1]`) is installed; its name, version and digest go to the log first.
fn pkg_install_file(os: &dyn AdminOs, req: &shared::admin::AdminRequest, who: &str) -> bool {
    let (staged, shown) = (&req.args[0], &req.args[1]);
    match os.package_facts(staged) {
        Ok(f) if f.sha256 == *shown => {
            os.log(&format!(
                "{who}: pkg-install-file: {} {} sha256 {}",
                f.name, f.version, f.sha256
            ));
            execute(req, os)
        }
        Ok(f) => {
            os.log(&format!(
                "{who}: pkg-install-file: {staged} has sha256 {}, the sheet showed {shown}; not installed",
                f.sha256
            ));
            false
        }
        Err(e) => {
            os.log(&format!("{who}: pkg-install-file: {staged}: {e}"));
            false
        }
    }
}

/// The last non-empty line a failed program said on stderr, as one line for the person and the
/// log: control characters dropped, cut so that "The change failed: " and it fit one reply line
/// ([`shared::admin::MAX_FAILURE_MESSAGE`]); None when it said nothing.
pub fn reason_of(stderr: &str) -> Option<String> {
    let line = stderr.lines().rev().find(|l| !l.trim().is_empty())?;
    let line: String = line.chars().filter(|c| !c.is_control()).collect();
    let line = line.trim();
    let room = shared::admin::MAX_FAILURE_MESSAGE
        .saturating_sub(shared::admin::AdminReply::Failed.message().len() + 2);
    let mut cut = line.len().min(room);
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    (cut > 0).then(|| line[..cut].trim_end().to_owned())
}

/// The lock every slot update holds while it builds (`flock`, taken by the slot builder that
/// `wol update apply`, the pacman wrapper and Update Now share). wol-admind only asks whether it is
/// held; it never holds it itself, so the builder it starts can.
pub const UPDATE_LOCK: &str = wol_core::update::LOCK;

/// What Update Now answers when another update holds [`UPDATE_LOCK`]: nothing was started.
pub const UPDATE_RUNNING: &str = concat!(
    "Another update is running. Wait for it to finish, then try again. ",
    "Nothing was changed."
);

/// True when another process holds an exclusive `flock` on `path` right now. The file counts only
/// when root made it for this: a regular file owned by root with no group or other permission
/// bits (anyone else could create or hold one to block Update Now). Read from `/proc/locks`, so
/// the probe takes no lock of its own and a builder's `flock -n` never fails because of it.
pub fn lock_held(path: &Path) -> bool {
    lock_held_by_owner(path, 0)
}

/// [`lock_held`] for a lock file owned by `owner` (root in the daemon, the test's own account in
/// tests).
fn lock_held_by_owner(path: &Path, owner: u32) -> bool {
    shared::sys::flock_held_by(path, owner)
}

/// [`shared::sys::flock_write_held`] (the one implementation wol shares).
pub fn flock_write_held(locks: &str, dev: u64, ino: u64) -> bool {
    shared::sys::flock_write_held(locks, dev, ino)
}

/// `: <what the program said>` for a log line, one line, or nothing when it said nothing.
fn failure_detail(said: &str) -> String {
    let last = said.lines().rev().find(|l| !l.trim().is_empty());
    last.map_or_else(String::new, |l| format!(": {}", l.trim()))
}

/// Update Now (Settings > Software Update): the `[wolos]` fixes inside the pinned release, made in
/// the other root slot by the one slot builder the pacman wrapper and `wol update` share. The
/// running slot is the restore point and no step touches it; its name goes to the log before
/// anything runs. A failed step deletes what it half made; the restore point stays as it was.
/// One copy of WolOS (ADR 0023, `/etc/wolos/single-copy`): in place, nothing to go back to (the
/// page says so). Neither: refused, nothing runs. Err: the sentence for the page.
fn update_now(os: &dyn AdminOs, holds: &[String], who: &str) -> Result<(), String> {
    use wol_core::update::{self as up, Step, pkg};
    if os.update_locked() {
        os.log(&format!(
            "{who}: pkg-upgrade: another update holds {UPDATE_LOCK}; nothing was started"
        ));
        return Err(UPDATE_RUNNING.into());
    }
    // Held to the end of this function: every step below runs under it.
    let _lock = match os.hold_update_lock() {
        Ok(Some(l)) => l,
        Ok(None) => {
            os.log(&format!(
                "{who}: pkg-upgrade: another update holds {UPDATE_LOCK}; nothing was started"
            ));
            return Err(UPDATE_RUNNING.into());
        }
        Err(e) => {
            os.log(&format!("{who}: pkg-upgrade: {e}; nothing was started"));
            return Err(pkg::SLOT_FAILED.into());
        }
    };
    // The release the daily check found moves with the packages: the new slot's repos are set to
    // its snapshot date. With a package held back the release stays where it is (a held package
    // pins the repos it came from), and the log says so (QA-100 STORE-2).
    let release = os.trusted_release().filter(|_| holds.is_empty());
    if let Some((version, date)) = &release {
        os.log(&format!(
            "{who}: pkg-upgrade: WolOS {version} (snapshot {date}) comes with this update"
        ));
    } else if !holds.is_empty() && os.trusted_release().is_some() {
        os.log(&format!(
            "{who}: pkg-upgrade: packages are held back, so the new WolOS release stays for later"
        ));
    }
    let run = |s: &Step| match s {
        Step::Run(argv) => os.run(argv, true),
        Step::Write { path, text } => os.write(path, 0o644, text),
    };
    if os.read(up::SINGLE_COPY).is_some() {
        os.log(&format!(
            "{who}: pkg-upgrade: one copy of WolOS, no restore point; updating the running system"
        ));
        let steps = match (&release, holds.is_empty()) {
            (Some((_, date)), true) => {
                up::plan_in_place(Some(date)).unwrap_or_else(|| pkg::update_now_in_place(holds))
            }
            _ => pkg::update_now_in_place(holds),
        };
        for s in &steps {
            if !run(s) {
                os.log(&format!(
                    "{who}: pkg-upgrade: {s} failed{}",
                    failure_detail(&os.last_error())
                ));
                return Err(pkg::IN_PLACE_FAILED.into());
            }
        }
        return Ok(());
    }
    let cmdline = os.read("/proc/cmdline");
    let bootctl = os.capture(&up::be::LIST_ARGV.map(String::from));
    let args = pkg::update_now_args(holds);
    let Some(c) = pkg::slot_change(cmdline.as_deref(), bootctl.as_deref(), &args) else {
        os.log(&format!("{who}: pkg-upgrade: {}", pkg::NO_RESTORE_POINT));
        return Err(pkg::NO_RESTORE_POINT.into());
    };
    let (point, slot) = (c.restore_point, &c.slot);
    // Room first (QA-100 STORE-11, ACC-23): a slot build that runs the disk full fails late and
    // read as "check that you're online".
    let free = os
        .capture(&pkg::ROOM_ARGV.map(String::from))
        .and_then(|t| shared::update::free_from_df(&t));
    if let Some(said) = pkg::room_needed(free) {
        os.log(&format!("{who}: pkg-upgrade: {said}"));
        return Err(said);
    }
    // The same slot, built on the release's snapshot (its repos set to the date) when there is one.
    let steps = match &release {
        Some((_, date)) => up::plan_from(slot, c.from, Some(date), cmdline.as_deref(), holds)
            .unwrap_or_else(|| c.steps.clone()),
        None => c.steps.clone(),
    };
    os.log(&format!(
        "{who}: pkg-upgrade: restore point {point} (the running system) is kept; updating into {slot}"
    ));
    for (i, s) in steps.iter().enumerate() {
        if !run(s) {
            // Before the cleanup: its steps are commands too and would replace what this said.
            let why = os.last_error();
            let undo = up::rollback_at(slot, c.from, &steps, i);
            for r in &undo.argv {
                let _ = os.run(r, false);
            }
            if undo.done {
                os.log(&format!(
                    "{who}: pkg-upgrade: {s} failed{}; updated into {slot} all the same, restore point {point}",
                    failure_detail(&why)
                ));
                return Ok(());
            }
            let kept = if undo.kept {
                format!("{slot} is as it was")
            } else {
                format!("{slot} could not be kept and was removed")
            };
            os.log(&format!(
                "{who}: pkg-upgrade: {s} failed{}; {kept}, restore point {point} is unchanged",
                failure_detail(&why)
            ));
            return Err(pkg::SLOT_FAILED.into());
        }
    }
    os.log(&format!(
        "{who}: pkg-upgrade: updated into {slot}, it starts at the next restart; restore point {point}"
    ));
    Ok(())
}

fn execute(req: &shared::admin::AdminRequest, os: &dyn AdminOs) -> bool {
    let probe =
        |argv: &[&str]| os.capture(&argv.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>());
    let steps = match plan_with(req, &probe) {
        Ok(s) => s,
        Err(e) => {
            os.log(&e);
            return false;
        }
    };
    // S4-4: one table for wol-admind and the `wol` client that waits for it.
    let long = wol_core::admin::LONG_ACTIONS.contains(&req.action);
    // App Store installs publish their percentage for the tile ring.
    let progress = match req.action {
        AdminAction::PkgInstall => Some(wol_core::progress::Kind::Pacman),
        AdminAction::FlatpakInstall => Some(wol_core::progress::Kind::Flatpak),
        _ => None,
    }
    .zip(
        req.args
            .first()
            .and_then(|t| wol_core::progress::file_for(wol_core::progress::SYSTEM_DIR, t)),
    );
    steps.iter().all(|s| match s {
        AdminStep::Run(argv) => match &progress {
            Some((kind, path)) => os.run_progress(argv, *kind, path),
            None => os.run(argv, long),
        },
        AdminStep::RunInput { argv, input } => os.run_input(argv, input),
        AdminStep::Write {
            path,
            mode,
            content,
        } => os.write(path, *mode, &content.render(os.read(path).as_deref())),
    })
}

/// `.wol` install (ADR 0011): verify the daemon's own copy, check the payload is the package
/// Info.json names, then `wol pkg -- -U` (dependencies from the pinned repos). `trust` is what the caller
/// approved: an unsigned file needs `unsigned` (and so a fresh password).
fn wol_install(os: &dyn AdminOs, src: &str, trust: &str, caller: Option<u32>) -> bool {
    let Some(dir) = os.stage_dir() else {
        os.log("package staging directory unavailable");
        return false;
    };
    let r = stage_and_install(os, &dir, src, trust == "unsigned", caller);
    os.remove_dir(&dir);
    match r {
        Ok(()) => true,
        Err(e) => {
            os.log(&format!("{src}: {e}"));
            false
        }
    }
}

fn stage_and_install(
    os: &dyn AdminOs,
    dir: &Path,
    src: &str,
    allow_unsigned: bool,
    caller: Option<u32>,
) -> Result<(), String> {
    // Audit S15: names under the fresh staging directory go through safe_join, so a name that
    // already exists there as a symlink out of the directory is refused, not written through.
    let under = |name: &str| {
        shared::safe_path::safe_join_str(dir, name).map_err(|e| format!("staging path: {e}"))
    };
    let staged = under("app.wol")?;
    os.copy_in(src, &staged, caller)?;
    let v = package::verify(&staged, &os.trusted_keys())?;
    if v.trust == Trust::Unsigned && !allow_unsigned {
        return Err("unsigned package; approve it as unsigned".into());
    }
    let payload = under(shared::package::PAYLOAD)?;
    package::extract(&staged, &v.payload, &payload).map_err(|e| e.to_string())?;
    let p = payload.to_str().ok_or("staging path not UTF-8")?;
    let (info, query) = (&v.info, |argv: Vec<String>| os.capture(&argv));
    let nv =
        query(package::query_name_version_argv(p)).and_then(|t| package::parse_name_version(&t));
    if nv.as_ref() != Some(&(info.package.clone(), info.version.clone())) {
        return Err(format!(
            "payload {nv:?} does not match {} {}",
            info.package, info.version
        ));
    }
    let installed = query(package::query_installed_argv(&info.package))
        .and_then(|t| package::parse_name_version(&t))
        .map(|(_, v)| v);
    let steps = package::install_steps(p, &info.version, installed.as_deref())?;
    for s in &steps {
        if !os.run(s, true) {
            return Err(format!("{} failed", s.join(" ")));
        }
    }
    os.log(&format!(
        "installed {} {} ({})",
        info.id,
        info.version,
        if v.trust == Trust::Unsigned {
            "unsigned"
        } else {
            "signed"
        }
    ));
    Ok(())
}

/// Remove by id: the install record (owned by pkg) names the package.
fn wol_remove(os: &dyn AdminOs, id: &str) -> bool {
    let path = format!(
        "{}/{id}/{}",
        shared::package::RECORDS_DIR,
        shared::bundle::INFO_FILE
    );
    let steps = match os
        .read(&path)
        .ok_or_else(|| format!("{id} is not installed"))
    {
        Ok(text) => package::remove_steps(id, &text),
        Err(e) => Err(e),
    };
    let steps = match steps {
        Ok(s) => s,
        Err(e) => {
            os.log(&e);
            return false;
        }
    };
    steps.iter().all(|s| os.run(s, true))
}

/// The daemon reads a caller's file only if the caller could: owner, or world-readable. Root
/// may read anything; an unknown peer nothing.
pub fn caller_may_read(file_uid: u32, mode: u32, caller: Option<u32>) -> bool {
    match caller {
        Some(0) => true,
        Some(u) => (file_uid == u && mode & 0o400 != 0) || mode & 0o004 != 0,
        None => false,
    }
}

/// The caller could walk down to the file: every directory above it (uid, mode), from `/` to the
/// parent, is searchable by the caller as owner (`u+x`) or as anyone (`o+x`). Group access is not
/// granted (conservative: the daemon does not resolve the caller's groups). Audit A24: without
/// this a world-readable file inside a private directory of another account would be copied.
pub fn caller_may_reach(dirs: &[(u32, u32)], caller: Option<u32>) -> bool {
    match caller {
        Some(0) => true,
        Some(u) => dirs
            .iter()
            .all(|&(owner, mode)| (owner == u && mode & 0o100 != 0) || mode & 0o001 != 0),
        None => false,
    }
}

/// (uid, mode) of every directory above the opened file `f`, parent first, for
/// [`caller_may_reach`]: the directories of what was opened (the kernel's path of the
/// descriptor, symlinks in the parents resolved), not of the name the caller sent.
pub fn dirs_above(f: &std::fs::File) -> std::io::Result<Vec<(u32, u32)>> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    let real = std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd()))?;
    real.ancestors()
        .skip(1)
        .map(|d| std::fs::metadata(d).map(|m| (m.uid(), m.mode())))
        .collect()
}

/// A path the caller names is absolute, has no `..` or `.` parts and no empty parts.
pub fn is_plain_absolute(path: &str) -> bool {
    path.len() < 4096
        && path.strip_prefix('/').is_some_and(|rest| {
            rest.split('/')
                .all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains('\0'))
        })
}

/// Real OS: fixed argv with timeouts, atomic writes, PAM via the wol-authcheck helper, syslog via logger(1).
pub struct RealOs;

/// Helper contract: run as root with `--admin <user>`, password on stdin, exit 0 = authenticated.
pub const AUTHCHECK: &str = "/usr/lib/wol-authcheck";

impl AdminOs for RealOs {
    fn passwd(&self) -> String {
        std::fs::read_to_string("/etc/passwd").unwrap_or_default()
    }
    fn group(&self) -> String {
        std::fs::read_to_string("/etc/group").unwrap_or_default()
    }
    fn authenticate(&self, user: &str, password: &[u8]) -> bool {
        let mut cmd = crate::privileged::privileged_command(AUTHCHECK);
        cmd.args(["--admin", user]); // user checked by valid_name in authcheck
        check_password(cmd, password, AUTH_TIMEOUT)
    }
    fn run(&self, argv: &[String], long: bool) -> bool {
        let (secs, grace) = wol_core::admin::step_limit(argv, long);
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let t = std::time::Duration::from_secs(secs.into());
        // A long step (pacman) is interrupted at the deadline and given its grace to end the
        // package it is on; the group is killed after (QA-A9-CORE-3).
        let grace = match grace {
            0 => wol_core::system::KILL_GRACE,
            g => std::time::Duration::from_secs(g.into()),
        };
        crate::privileged::capture_grace(&argv, t, grace).is_some()
    }
    fn last_error(&self) -> String {
        wol_core::privileged::take_last_stderr()
    }
    fn update_locked(&self) -> bool {
        lock_held(Path::new(UPDATE_LOCK))
    }
    fn hold_update_lock(&self) -> Result<Option<Box<dyn std::any::Any>>, String> {
        shared::sys::try_flock(Path::new(UPDATE_LOCK))
            .map(|l| l.map(|f| Box::new(f) as Box<dyn std::any::Any>))
            .map_err(|e| format!("{UPDATE_LOCK}: {e}"))
    }
    fn trusted_release(&self) -> Option<(String, String)> {
        use wol_core::update::check::{DIR, RELEASE_FILE, Updates};
        let running = std::fs::read_to_string(RELEASE_FILE).ok();
        let u = Updates::read(Path::new(DIR), self.now())?.installed(running.as_deref());
        u.base.zip(u.snapshot)
    }
    fn run_progress(&self, argv: &[String], kind: wol_core::progress::Kind, path: &str) -> bool {
        let Some((prog, args)) = argv.split_first() else {
            return false;
        };
        // Audit S14: the same cleared environment as every other root child.
        let mut cmd = crate::privileged::privileged_command(prog);
        cmd.args(args);
        let t = std::time::Duration::from_secs(1800);
        wol_core::progress::run_tracked_cmd(cmd, kind, path, t)
    }
    fn run_input(&self, argv: &[String], input: &str) -> bool {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        crate::privileged::capture_input(&argv, input, std::time::Duration::from_secs(30)).is_some()
    }
    fn write(&self, path: &str, mode: u32, text: &str) -> bool {
        atomic_write(std::path::Path::new(path), mode, text.as_bytes()).is_ok()
    }
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
    fn refuse_delay(&self) {
        std::thread::sleep(std::time::Duration::from_millis(
            wol_core::admin::REFUSAL_DELAY_MS,
        ));
    }
    fn now(&self) -> u64 {
        shared::sys::monotonic_ns() / 1_000_000_000
    }
    fn capture(&self, argv: &[String]) -> Option<String> {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        crate::privileged::capture(&argv, std::time::Duration::from_secs(30))
    }
    fn wall_now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
    fn stage_dir(&self) -> Option<PathBuf> {
        use std::os::unix::fs::DirBuilderExt;
        let root = Path::new(shared::package::STAGING_DIR);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root)
            .ok()?;
        // Audit S16: one fresh, unguessable directory per install; mkdir fails (EEXIST) rather
        // than reuse a name, and copy_in creates the file inside with O_EXCL (create_new).
        let d = root.join(format!(
            "{}-{}-{}",
            std::process::id(),
            shared::sys::monotonic_ns(),
            shared::jobs::new_nonce().ok()?
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&d).ok()?;
        Some(d)
    }
    fn copy_in(&self, src: &str, dst: &Path, caller: Option<u32>) -> Result<(), String> {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        if !is_plain_absolute(src) {
            return Err("package path is not a plain absolute path".into());
        }
        // O_NOCTTY too (S4-2): a terminal the caller names never becomes wol-admind's.
        let mut f = shared::sys::open_regular(src).map_err(|e| e.to_string())?;
        let m = f.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.len() > shared::package::MAX_SIZE {
            return Err("not a package file".into());
        }
        if !caller_may_read(m.uid(), m.mode(), caller) {
            return Err("caller cannot read this file".into());
        }
        let dirs = dirs_above(&f).map_err(|e| e.to_string())?;
        if !caller_may_reach(&dirs, caller) {
            return Err("caller cannot reach this file".into());
        }
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dst)
            .map_err(|e| e.to_string())?;
        // One byte past the cap tells a file that grew after the check from one that fits.
        let n = std::io::copy(&mut (&mut f).take(shared::package::MAX_SIZE + 1), &mut out)
            .map_err(|e| e.to_string())?;
        if n > shared::package::MAX_SIZE {
            return Err("package file grew past the size limit while copying".into());
        }
        out.sync_all().map_err(|e| e.to_string())
    }
    fn trusted_keys(&self) -> Vec<[u8; 32]> {
        package::read_keys(Path::new(shared::package::KEYS_DIR))
    }
    fn remove_dir(&self, dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
    fn package_facts(&self, staged: &str) -> Result<crate::packages::PkgFacts, String> {
        crate::packages::inspect_staged(staged)
    }
    fn log(&self, text: &str) {
        eprintln!("wol-admind: {text}");
        let _ = crate::privileged::capture(
            &[
                "logger",
                "-p",
                "auth.notice",
                "-t",
                "wol-admind",
                "--",
                text,
            ],
            std::time::Duration::from_secs(2),
        );
    }
    fn sessions(&self) -> Vec<u32> {
        let list = |d: &str| -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(d)
                .map(|r| {
                    r.flatten()
                        .map(|e| e.path().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            v.sort();
            v
        };
        let mut uids: Vec<u32> = wol_core::power::policy::sessions(&list)
            .into_iter()
            .filter(|s| {
                use std::os::unix::fs::{FileTypeExt, MetadataExt};
                // The socket must be the account's own: nobody plants one for someone else.
                std::fs::symlink_metadata(&s.socket)
                    .is_ok_and(|m| m.file_type().is_socket() && m.uid() == s.uid)
            })
            .map(|s| s.uid)
            .collect();
        uids.dedup();
        uids
    }
}

/// temp file + fsync + rename, mode set at open (R2C-23, S4-10): one writer for every daemon,
/// [`shared::atomic::atomic_write`].
pub use shared::atomic::atomic_write;

/// Session id of `pid`, -1 if unknown.
pub fn session_of(pid: i32) -> i32 {
    // SAFETY: plain syscall.
    if pid > 0 {
        unsafe { libc::getsid(pid) }
    } else {
        -1
    }
}

#[cfg(test)]
mod tests;
