// SPDX-License-Identifier: GPL-2.0-or-later
//! wol-admind request handling (IaraOS ADR 0026): allow-list only, judged by peer credentials.
use shared::admin::{AdminAction, AdminReply, decode};
use std::path::{Path, PathBuf};
use wol_core::admin::{
    AdminDecision, AdminStep, AuthCache, decide, describe, is_admin, needs_fresh_password,
    plan_with, validate,
};
use wol_core::package;
use wol_core::woltrust::Trust as woltrust_trust;

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
    /// Wall-clock seconds (restore point names).
    fn wall_now(&self) -> u64;
    /// Fresh root-only directory under [`shared::package::STAGING_DIR`] for one install.
    fn stage_dir(&self) -> Option<PathBuf>;
    /// Copy the caller's `.wol` into `dst`: a regular file the caller may read, at most
    /// [`shared::package::MAX_SIZE`].
    fn copy_in(&self, src: &str, dst: &Path, caller: Option<u32>) -> Result<(), String>;
    /// Ed25519 keys in [`shared::package::KEYS_DIR`].
    fn trusted_keys(&self) -> Vec<[u8; 32]>;
    /// The certificate trust store of ADR 0046 (`/usr/share/wolos/trust`, `/var/lib/wolos/trust`).
    fn trust_store(&self) -> wol_core::trust::TrustStore;
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

/// One connection, on its own thread: identity from the kernel at accept, the ADR 0033 hello,
/// the request within [`REQUEST_DEADLINE`], then [`handle`] under the one cache lock (requests
/// still run one at a time, as before), the reply, and the buffer wiped.
pub fn serve_one(
    conn: std::os::unix::net::UnixStream,
    cache: &std::sync::Mutex<AuthCache>,
    os: &dyn AdminOs,
) {
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
    let reply = {
        let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
        handle(&buf[..n], caller, &mut c, os)
    };
    buf.fill(0); // password never outlives the request
    let _ = conn.set_write_timeout(Some(REQUEST_DEADLINE));
    let _ = writeln!(&conn, "{}", reply.line());
}

/// wol-admind's accept loop (audit S4-7): each connection on its own thread, at most
/// [`CONN_PER_UID`] per account and [`CONN_TOTAL`] in all, so one idle or slow client never
/// holds the others. Never returns.
pub fn serve(listener: &std::os::unix::net::UnixListener, os: &'static (dyn AdminOs + Sync)) {
    let cache = std::sync::Arc::new(std::sync::Mutex::new(AuthCache::default()));
    let quota = shared::bounded::Quota::new(CONN_PER_UID, CONN_TOTAL);
    loop {
        let conn = match listener.accept() {
            Ok((c, _)) => c,
            // Out of descriptors: wait instead of spinning.
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
        };
        let Some(slot) = shared::sys::peer(&conn).and_then(|p| quota.admit(p.uid)) else {
            continue;
        };
        let cache = cache.clone();
        let _ = std::thread::Builder::new()
            .name("admind-conn".into())
            .spawn(move || {
                serve_one(conn, &cache, os);
                drop(slot);
            });
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

/// Judge and run one request. Never logs the password or keybind JSON.
pub fn handle(
    data: &[u8],
    caller: Option<Caller>,
    cache: &mut AuthCache,
    os: &dyn AdminOs,
) -> AdminReply {
    let uid = caller.map(|c| c.uid);
    let session = caller.map_or(-1, |c| c.session);
    let user = uid.and_then(|u| user_of(&os.passwd(), u));
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
            cache.revoke(u, session);
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
    let cached = !needs_fresh_password(&req)
        && session > 0
        && uid.is_some_and(|u| cache.valid(u, session, os.now()));
    let user_level = known.is_some_and(|u| wol_core::admin::user_level_ok(&req, u, &os.sessions()));
    // Anyone may change their own password, with the current one (fresh: never the cache).
    let own_password = req.action == AdminAction::UserPassword
        && user.as_ref().is_some_and(|(n, _)| *n == req.args[0]);
    match decide(
        known,
        admin || user_level || own_password,
        cached || user_level,
        !pw.is_empty(),
    ) {
        AdminDecision::NotAdmin => {
            os.log(&format!("{who}: {what}: not an administrator"));
            return refuse(AdminReply::NotAdmin);
        }
        AdminDecision::NeedPassword => return AdminReply::NeedsAuth,
        AdminDecision::Authenticate => {
            let name = user.as_ref().map_or("", |(n, _)| n.as_str());
            if !os.authenticate(name, &data[pw]) {
                os.log(&format!("{who}: {what}: authentication failed"));
                return refuse(AdminReply::Denied);
            }
            if session > 0
                && let Some(u) = uid
            {
                cache.grant(u, session, os.now());
            }
        }
        AdminDecision::Allow => {}
    }
    let ok = match req.action {
        AdminAction::WolInstall => wol_install(os, &req.args[0], &req.args[1], uid),
        AdminAction::WolRemove => wol_remove(os, &req.args[0]),
        AdminAction::PkgInstallFile => pkg_install_file(os, &req, &who),
        _ => execute(&req, os),
    };
    os.log(&format!(
        "{who}: {what}: {}",
        if ok { "ok" } else { "failed" }
    ));
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
/// Info.json names, then `pacman -U` (dependencies from the pinned repos). `trust` is what the caller
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
    // ADR 0046: the second check, as root on the staged copy, with the certificate store.
    let checked =
        os.trust_store()
            .verify_wol_at(&staged, &os.trusted_keys(), None, os.wall_now())?;
    let (v, outcome) = (checked.verified, checked.outcome);
    let sentence = outcome.code.sentence().unwrap_or_default();
    match outcome.trust {
        woltrust_trust::Invalid | woltrust_trust::Withdrawn => {
            return Err(format!("refused ({}): {sentence}", outcome.code.as_str()));
        }
        woltrust_trust::Community if !allow_unsigned => {
            return Err(format!(
                "unsigned package ({}); approve it as unsigned",
                outcome.code.as_str()
            ));
        }
        _ => {}
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
        match outcome.trust {
            woltrust_trust::Wolos => "made by WolOS",
            woltrust_trust::Verified => "verified author",
            _ => "unsigned",
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
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let secs = if long {
            wol_core::admin::LONG_STEP_SECS
        } else {
            wol_core::admin::STEP_SECS
        };
        let t = std::time::Duration::from_secs(secs.into());
        crate::privileged::capture(&argv, t).is_some()
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
        let mut f = shared::sys::open_named(src).map_err(|e| e.to_string())?;
        let m = f.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.len() > shared::package::MAX_SIZE {
            return Err("not a package file".into());
        }
        if !caller_may_read(m.uid(), m.mode(), caller) {
            return Err("caller cannot read this file".into());
        }
        // The directories of what was opened (the kernel's path of the descriptor, symlinks in
        // the parents resolved), not of the name the caller sent.
        use std::os::fd::AsRawFd;
        let real = std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd()))
            .map_err(|e| e.to_string())?;
        let mut dirs = Vec::new();
        for d in real.ancestors().skip(1) {
            let dm = std::fs::metadata(d).map_err(|e| e.to_string())?;
            dirs.push((dm.uid(), dm.mode()));
        }
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
    fn trust_store(&self) -> wol_core::trust::TrustStore {
        wol_core::trust::TrustStore::load(&crate::privileged::PrivilegedHost)
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
mod tests {
    use super::*;
    use shared::admin::{AdminRequest, encode};
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    struct Fake {
        log: RefCell<Vec<String>>,
        ran: RefCell<Vec<Vec<String>>>,
        stdin: RefCell<Vec<String>>,
        wrote: RefCell<Vec<(String, String)>>,
        delays: Cell<u32>,
        auths: Cell<u32>,
        fail_run: bool,
        captured: RefCell<Vec<Vec<String>>>,
        /// Probe answers by command line (`sysctl -n hw.ncpu` -> "4\n"); others fail.
        probes: Vec<(&'static str, &'static str)>,
        /// Accounts with a desktop session.
        sessions: Vec<u32>,

        /// Trusted package keys.
        keys: Vec<[u8; 32]>,
        /// The certificate store (empty unless a test fills it).
        trust: wol_core::trust::TrustStore,
        /// `wall_now` when not zero.
        wall: u64,
        /// (`pacman -Q` command line suffix, stdout); unmatched queries fail.
        pkg: Vec<(&'static str, String)>,
        /// (path, text) for `read`.
        files: Vec<(String, String)>,
        stage: RefCell<Option<std::path::PathBuf>>,
        /// Progress files of tracked runs.
        progress: RefCell<Vec<String>>,
        /// SHA-256 of every staged package copy (None: [`FAKE_SHA`]).
        staged_sha: Option<String>,
    }

    /// What the fake reads from a staged package copy unless `staged_sha` says otherwise.
    const FAKE_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    impl AdminOs for Fake {
        fn passwd(&self) -> String {
            "root:*:0:0::/root:/bin/sh\nann:*:1001:1001::/home/ann:/bin/sh\nbob:*:1002:1002::/home/bob:/bin/sh\n".into()
        }
        fn group(&self) -> String {
            "wheel:*:0:root,ann\n".into()
        }
        fn authenticate(&self, user: &str, pw: &[u8]) -> bool {
            self.auths.set(self.auths.get() + 1);
            user == "ann" && pw == b"good"
        }
        fn run(&self, argv: &[String], _: bool) -> bool {
            self.ran.borrow_mut().push(argv.to_vec());
            !self.fail_run
        }
        fn run_progress(&self, argv: &[String], _: wol_core::progress::Kind, path: &str) -> bool {
            self.progress.borrow_mut().push(path.into());
            self.run(argv, true)
        }
        fn capture(&self, argv: &[String]) -> Option<String> {
            self.captured.borrow_mut().push(argv.to_vec());
            let line = argv.join(" ");
            if argv[0] == "pacman" {
                return self
                    .pkg
                    .iter()
                    .find(|(k, _)| line.ends_with(k))
                    .map(|(_, v)| v.clone());
            }
            self.probes
                .iter()
                .find(|p| p.0 == line)
                .map(|p| p.1.to_string())
        }
        fn stage_dir(&self) -> Option<std::path::PathBuf> {
            let d = std::env::temp_dir().join(format!(
                "wol-admind-stage-{}-{:p}",
                std::process::id(),
                self
            ));
            std::fs::create_dir_all(&d).ok()?;
            *self.stage.borrow_mut() = Some(d.clone());
            Some(d)
        }
        fn copy_in(&self, src: &str, dst: &std::path::Path, _: Option<u32>) -> Result<(), String> {
            std::fs::copy(src, dst)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        fn trusted_keys(&self) -> Vec<[u8; 32]> {
            self.keys.clone()
        }
        fn trust_store(&self) -> wol_core::trust::TrustStore {
            self.trust.clone()
        }
        fn remove_dir(&self, d: &std::path::Path) {
            let _ = std::fs::remove_dir_all(d);
        }
        fn package_facts(&self, _: &str) -> Result<crate::packages::PkgFacts, String> {
            match self.staged_sha.as_deref() {
                Some("") => Err("it has no package information (.PKGINFO) at its start".into()),
                sha => Ok(crate::packages::PkgFacts {
                    name: "x".into(),
                    version: "1-1".into(),
                    sha256: sha.unwrap_or(FAKE_SHA).into(),
                }),
            }
        }
        fn wall_now(&self) -> u64 {
            if self.wall != 0 {
                return self.wall;
            }
            1_761_600_000
        }
        fn run_input(&self, argv: &[String], input: &str) -> bool {
            self.ran.borrow_mut().push(argv.to_vec());
            self.stdin.borrow_mut().push(input.into());
            !self.fail_run
        }
        fn write(&self, path: &str, _: u32, text: &str) -> bool {
            self.wrote.borrow_mut().push((path.into(), text.into()));
            true
        }
        fn read(&self, path: &str) -> Option<String> {
            self.files.iter().find(|f| f.0 == path).map(|f| f.1.clone())
        }
        fn refuse_delay(&self) {
            self.delays.set(self.delays.get() + 1);
        }
        fn now(&self) -> u64 {
            100
        }
        fn log(&self, t: &str) {
            self.log.borrow_mut().push(t.into());
        }
        fn sessions(&self) -> Vec<u32> {
            self.sessions.clone()
        }
    }

    fn req(a: AdminAction, args: &[&str]) -> AdminRequest {
        AdminRequest {
            action: a,
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    const ANN: Option<Caller> = Some(Caller {
        uid: 1001,
        session: 7,
    });
    const BOB: Option<Caller> = Some(Caller {
        uid: 1002,
        session: 8,
    });
    const ROOT: Option<Caller> = Some(Caller { uid: 0, session: 1 });

    #[test]
    fn decision_table() {
        let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
        let up_good = encode(&req(AdminAction::PkgUpgrade, &[]), b"good");
        let up_bad = encode(&req(AdminAction::PkgUpgrade, &[]), b"bad");
        let bad = encode(&req(AdminAction::Idle, &["x"]), b"");
        let cases: [(&[u8], Option<Caller>, AdminReply, u32); 8] = [
            (&up, ROOT, AdminReply::Ok, 0),
            (&up, ANN, AdminReply::NeedsAuth, 0),
            (&up_bad, ANN, AdminReply::Denied, 1),
            (&up_good, ANN, AdminReply::Ok, 0),
            (&up_good, BOB, AdminReply::NotAdmin, 1),
            (&up, None, AdminReply::NotAdmin, 1),
            (&bad, ROOT, AdminReply::Invalid, 1),
            (b"garbage", ROOT, AdminReply::Invalid, 1),
        ];
        for (data, caller, want, delays) in cases {
            let os = Fake::default();
            let got = handle(data, caller, &mut AuthCache::default(), &os);
            assert_eq!(
                (got, os.delays.get()),
                (want, delays),
                "{caller:?} {:?}",
                String::from_utf8_lossy(data)
            );
        }
    }

    /// Finish Secure Boot setup from System Settings: an administrator's password, then the one
    /// fixed command (`wol boot enroll`, which prints `enrolled` and changes nothing when the key
    /// is already in db, so asking twice is harmless).
    #[test]
    fn boot_enroll_needs_the_admin_password_and_runs_the_verb() {
        let os = Fake::default();
        let mut cache = AuthCache::default();
        let none = encode(&req(AdminAction::BootEnroll, &[]), b"");
        let good = encode(&req(AdminAction::BootEnroll, &[]), b"good");
        assert_eq!(handle(&none, ANN, &mut cache, &os), AdminReply::NeedsAuth);
        assert_eq!(handle(&good, BOB, &mut cache, &os), AdminReply::NotAdmin);
        assert_eq!(handle(&good, ANN, &mut cache, &os), AdminReply::Ok);
        assert_eq!(handle(&good, ANN, &mut cache, &os), AdminReply::Ok);
        let ran: Vec<String> = os.ran.borrow().iter().map(|v| v.join(" ")).collect();
        assert_eq!(ran, ["wol boot enroll", "wol boot enroll"]);
        let with_arg = encode(&req(AdminAction::BootEnroll, &["--force"]), b"good");
        assert_eq!(
            handle(&with_arg, ROOT, &mut cache, &Fake::default()),
            AdminReply::Invalid
        );
    }

    /// Audit C7-1 (ADR 0034): a locally built package, Undo Update and the update channel run as
    /// root only for an administrator, with a password typed for each (never the session cache).
    /// A standard account is refused whatever its own shell answered; a path outside wold's
    /// staging layout is refused before anything runs.
    #[test]
    fn root_package_changes_need_an_administrator_and_a_fresh_password() {
        let staged = format!(
            "{}/0123456789abcdef0123456789abcdef/x-1-1-x86_64.pkg.tar.zst",
            shared::package::STAGING_DIR
        );
        let os = Fake::default();
        let mut cache = AuthCache::default();
        for (action, args) in [
            (AdminAction::PkgInstallFile, vec![staged.as_str(), FAKE_SHA]),
            (AdminAction::SystemRollback, vec![]),
            (AdminAction::SystemChannel, vec!["beta"]),
        ] {
            let good = encode(&req(action, &args), b"good");
            let none = encode(&req(action, &args), b"");
            assert_eq!(handle(&good, BOB, &mut cache, &os), AdminReply::NotAdmin);
            assert_eq!(handle(&good, ANN, &mut cache, &os), AdminReply::Ok);
            assert_eq!(
                handle(&none, ANN, &mut cache, &os),
                AdminReply::NeedsAuth,
                "{action:?}: a warm cache is not enough"
            );
        }
        let ran: Vec<String> = os.ran.borrow().iter().map(|v| v.join(" ")).collect();
        assert_eq!(
            ran,
            [
                format!("wol pkg -- -U --noconfirm -- {staged}"),
                "wol update rollback".into()
            ]
        );
        assert_eq!(
            *os.wrote.borrow(),
            [(
                wol_core::admin::CHANNEL_FILE.to_owned(),
                "beta\n".to_owned()
            )]
        );
        for path in ["/home/bob/evil-1-1-any.pkg.tar.zst", "/tmp/x.pkg.tar.zst"] {
            let r = encode(
                &req(AdminAction::PkgInstallFile, &[path, FAKE_SHA]),
                b"good",
            );
            assert_eq!(handle(&r, ANN, &mut cache, &os), AdminReply::Invalid);
        }
        assert_eq!(os.ran.borrow().len(), 2, "nothing else ran");
        assert!(
            os.log
                .borrow()
                .iter()
                .any(|l| l == &format!("ann(1001): pkg-install-file: x 1-1 sha256 {FAKE_SHA}")),
            "{:?}",
            os.log.borrow()
        );
    }

    /// ADR 0034: wol-admind installs only the copy the sheet described. A staged copy whose
    /// SHA-256 is not the one shown, or that cannot be read, is not installed, and the log says
    /// why.
    #[test]
    fn a_staged_package_that_is_not_what_the_sheet_showed_is_not_installed() {
        let staged = format!(
            "{}/0123456789abcdef0123456789abcdef/x-1-1-x86_64.pkg.tar.zst",
            shared::package::STAGING_DIR
        );
        let mut cache = AuthCache::default();
        for (sha, why) in [
            ("f".repeat(64), "the sheet showed"),
            (String::new(), ".PKGINFO"),
        ] {
            let os = Fake {
                staged_sha: Some(sha),
                ..Default::default()
            };
            let r = encode(
                &req(AdminAction::PkgInstallFile, &[&staged, FAKE_SHA]),
                b"good",
            );
            assert_eq!(handle(&r, ANN, &mut cache, &os), AdminReply::Failed);
            assert!(os.ran.borrow().is_empty(), "nothing ran");
            assert!(
                os.log.borrow().iter().any(|l| l.contains(why)),
                "{:?}",
                os.log.borrow()
            );
        }
    }

    #[test]
    fn authorization_is_cached_per_session_and_revocable() {
        let os = Fake::default();
        let mut cache = AuthCache::default();
        let good = encode(&req(AdminAction::PkgUpgrade, &[]), b"good");
        let none = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
        assert_eq!(handle(&good, ANN, &mut cache, &os), AdminReply::Ok);
        assert_eq!(handle(&none, ANN, &mut cache, &os), AdminReply::Ok);
        assert_eq!(os.auths.get(), 1);
        let other_session = Some(Caller {
            uid: 1001,
            session: 9,
        });
        assert_eq!(
            handle(&none, other_session, &mut cache, &os),
            AdminReply::NeedsAuth
        );
        assert_eq!(
            handle(
                &encode(&req(AdminAction::Revoke, &[]), b""),
                ANN,
                &mut cache,
                &os
            ),
            AdminReply::Ok
        );
        assert_eq!(handle(&none, ANN, &mut cache, &os), AdminReply::NeedsAuth);
    }

    /// Settings > Modules Install sends two `PkgInstall` (the backend package, then the meta
    /// package): the person types the password for the first only; the second rides the session's
    /// authorization, so ONE password sheet rises (and a wrong password stops before either runs).
    #[test]
    fn a_module_install_of_two_packages_asks_one_password() {
        let os = Fake::default();
        let mut cache = AuthCache::default();
        let send = |name: &str, pw: &[u8], cache: &mut AuthCache| {
            handle(
                &encode(&req(AdminAction::PkgInstall, &[name]), pw),
                ANN,
                cache,
                &os,
            )
        };
        assert_eq!(send("ollama-rocm", b"", &mut cache), AdminReply::NeedsAuth);
        assert_eq!(send("ollama-rocm", b"bad", &mut cache), AdminReply::Denied);
        assert!(
            os.ran.borrow().is_empty(),
            "nothing ran without the password"
        );
        assert_eq!(send("ollama-rocm", b"good", &mut cache), AdminReply::Ok);
        assert_eq!(
            send("wolos-module-ai-local", b"", &mut cache),
            AdminReply::Ok,
            "the second package needs no second sheet"
        );
        assert_eq!(
            os.auths.get(),
            2,
            "one failed and one good password, no more"
        );
        assert_eq!(os.ran.borrow().len(), 2);
    }

    #[test]
    fn runs_planned_steps_and_logs_without_secrets() {
        let os = Fake::default();
        let r = req(AdminAction::WifiJoin, &["wlan0", "Home", "hunter22"]);
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        assert!(os.wrote.borrow().is_empty());
        assert_eq!(
            os.ran.borrow()[0].join(" "),
            "nmcli dev wifi connect Home passwd-file /dev/stdin"
        );
        assert!(os.log.borrow().iter().all(|l| !l.contains("hunter22")));
        let failing = Fake {
            fail_run: true,
            ..Fake::default()
        };
        let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
        assert_eq!(
            handle(&up, ROOT, &mut AuthCache::default(), &failing),
            AdminReply::Failed
        );
    }

    #[test]
    fn user_add_sets_password_on_stdin() {
        let os = Fake::default();
        let r = req(
            AdminAction::UserAdd,
            &["dinah", "Dinah", "no", "n3w-passw0rd"],
        );
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        let ran = os.ran.borrow();
        assert_eq!(ran[0][..2], ["useradd", "-m"]);
        assert_eq!(ran[1], ["chpasswd"]);
        assert_eq!(os.stdin.borrow()[0], "dinah:n3w-passw0rd\n");
        assert!(ran.iter().flatten().all(|a| !a.contains("n3w")));
        assert!(os.log.borrow().iter().all(|l| !l.contains("n3w")));
        let nopw = encode(&req(AdminAction::UserAdd, &["dinah", "Dinah", "no"]), b"");
        assert_eq!(
            handle(&nopw, ROOT, &mut AuthCache::default(), &os),
            AdminReply::Invalid
        );
    }

    /// Changing a password: anyone their own, always with the current one; someone else's only
    /// an administrator. The account and the disk slot get the new one on stdin.
    #[test]
    fn a_password_change_needs_the_current_password_every_time() {
        let os = Fake::default();
        let mut cache = AuthCache::default();
        let mine = req(AdminAction::UserPassword, &["ann", "n3w-passw0rd"]);
        assert_eq!(
            handle(&encode(&mine, b"good"), ANN, &mut cache, &os),
            AdminReply::Ok
        );
        assert_eq!(os.ran.borrow()[0], ["chpasswd"]);
        assert_eq!(os.ran.borrow()[1], ["wol", "encrypt", "password", "ann"]);
        assert_eq!(*os.stdin.borrow(), ["ann:n3w-passw0rd\n", "n3w-passw0rd\n"]);
        assert!(os.log.borrow().iter().all(|l| !l.contains("n3w")));
        assert_eq!(
            handle(&encode(&mine, b""), ANN, &mut cache, &os),
            AdminReply::NeedsAuth,
            "a password given a moment ago does not change it again"
        );
        let bobs = req(AdminAction::UserPassword, &["bob", "n3w-passw0rd"]);
        assert_eq!(
            handle(&encode(&bobs, b"wrong"), BOB, &mut cache, &os),
            AdminReply::Denied,
            "not an administrator, but his own: the password decides"
        );
        assert_eq!(
            handle(&encode(&mine, b"good"), BOB, &mut cache, &os),
            AdminReply::NotAdmin
        );
    }

    /// Update Now stays inside the pinned release: pacman, no restore point of its own (the
    /// previous root slot is the way back).
    #[test]
    fn package_upgrade_runs_pacman() {
        let os = Fake::default();
        let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
        assert_eq!(
            handle(&up, ROOT, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        let ran: Vec<String> = os.ran.borrow().iter().map(|a| a.join(" ")).collect();
        assert_eq!(ran, ["pacman -Syu --noconfirm"]);
        let wifi = Fake::default();
        let r = req(AdminAction::WifiPower, &["wlan0", "off"]);
        handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &wifi);
        assert!(wifi.captured.borrow().is_empty());
    }

    /// Low Power Mode probes the machine, saves the mode and switches power-profiles-daemon.
    #[test]
    fn low_power_mode_follows_the_probed_hardware() {
        let os = Fake {
            probes: vec![("nproc", "2\n"), ("powerprofilesctl get", "balanced\n")],
            ..Fake::default()
        };
        let low = encode(&req(AdminAction::PowerMode, &["low"]), b"");
        assert_eq!(
            handle(&low, ROOT, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        let ran: Vec<String> = os.ran.borrow().iter().map(|a| a.join(" ")).collect();
        assert_eq!(ran, ["powerprofilesctl set power-saver"]);
        assert_eq!(
            os.wrote.borrow()[0],
            (
                "/etc/wolos/power.json".to_string(),
                "{\"mode\":\"low\"}\n".to_string()
            )
        );
        let probed: Vec<String> = os.captured.borrow().iter().map(|a| a.join(" ")).collect();
        assert!(
            probed.contains(&"ls /sys/class/power_supply".to_string()),
            "{probed:?}"
        );
        // Nothing to steer (VM: no power-profiles-daemon): refused, nothing runs.
        let vm = Fake::default();
        assert_eq!(
            handle(&low, ROOT, &mut AuthCache::default(), &vm),
            AdminReply::Failed
        );
        assert!(vm.ran.borrow().is_empty() && vm.wrote.borrow().is_empty());
    }

    /// Low Power Mode, Wi-Fi, Bluetooth and Sleep are like on a Mac: the person at the computer
    /// (an account with a desktop session), no password. Accounts without a session, unknown peers
    /// and every system-wide action still need an administrator.
    #[test]
    fn everyday_changes_need_no_password_at_the_computer() {
        let os = Fake {
            probes: vec![("nproc", "1\n"), ("powerprofilesctl get", "balanced\n")],
            sessions: vec![1001, 1002],
            ..Fake::default()
        };
        let low = encode(&req(AdminAction::PowerMode, &["low"]), b"");
        for (caller, want) in [
            (ANN, AdminReply::Ok),
            (BOB, AdminReply::Ok),
            (None, AdminReply::NotAdmin),
        ] {
            assert_eq!(
                handle(&low, caller, &mut AuthCache::default(), &os),
                want,
                "{caller:?}"
            );
        }
        let sleep = encode(&req(AdminAction::SessionPower, &["sleep"]), b"");
        assert_eq!(
            handle(&sleep, BOB, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        assert_eq!(
            os.ran.borrow().last().unwrap().join(" "),
            "systemctl suspend"
        );
        assert_eq!(os.auths.get(), 0);
        assert!(
            os.log.borrow().iter().any(|l| l.contains("session-power")),
            "audited: {:?}",
            os.log.borrow()
        );
        let up = encode(&req(AdminAction::PkgUpgrade, &[]), b"");
        assert_eq!(
            handle(&up, BOB, &mut AuthCache::default(), &os),
            AdminReply::NotAdmin
        );
        // Ann shutting down would end Bob's session: her password, like an administrator change.
        let off = encode(&req(AdminAction::SessionPower, &["shutdown"]), b"");
        assert_eq!(
            handle(&off, ANN, &mut AuthCache::default(), &os),
            AdminReply::NeedsAuth
        );
        // No desktop session (a remote shell): no everyday shortcut.
        let away = Fake::default();
        let join = encode(&req(AdminAction::WifiPower, &["wlan0", "on"]), b"");
        assert_eq!(
            handle(&join, BOB, &mut AuthCache::default(), &away),
            AdminReply::NotAdmin
        );
    }

    #[test]
    fn passwd_lookup_and_atomic_write() {
        assert_eq!(
            user_of("ann:*:1001:20::/h:/bin/sh\n", 1001),
            Some(("ann".into(), 20))
        );
        assert_eq!(user_of("ann:*:1001:20::/h:/bin/sh\n", 5), None);
        let p = std::env::temp_dir().join(format!("wol-admind-{}.txt", std::process::id()));
        atomic_write(&p, 0o600, b"a").unwrap();
        atomic_write(&p, 0o600, b"b").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"b");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_file(p).unwrap();
    }

    const SEED: [u8; 32] = [7; 32];
    const INFO: &str = r#"{"id":"org.wolos.Fixture","version":"1.0","package":"wol-fixture","targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/fixture"}]}"#;

    /// A `.wol` file on disk, signed with `seed` when given.
    fn wol_file(name: &str, seed: Option<&[u8; 32]>, payload: &[u8]) -> String {
        use wol_core::package::{manifest, sign, ustar};
        let man = manifest(&[("Info.json", INFO.as_bytes()), ("payload.pkg", payload)]);
        let sig = seed.map(|s| sign(s, man.as_bytes()));
        let mut m: Vec<(&str, &[u8])> = vec![
            ("Info.json", INFO.as_bytes()),
            ("payload.pkg", payload),
            ("SHA256", man.as_bytes()),
        ];
        if let Some(sig) = &sig {
            m.push(("SHA256.sig", sig));
        }
        let p = std::env::temp_dir().join(format!("wol-admind-{}-{name}.wol", std::process::id()));
        std::fs::write(&p, ustar(&m)).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn pkg_os() -> Fake {
        Fake {
            keys: vec![wol_core::package::public_of(&SEED)],
            pkg: vec![("payload.pkg", "wol-fixture 1.0\n".into())],
            ..Fake::default()
        }
    }

    fn ran(os: &Fake) -> Vec<String> {
        let stage = os.stage.borrow().clone();
        let stage = stage.as_ref().map(|p| p.to_string_lossy().into_owned());
        os.ran
            .borrow()
            .iter()
            .map(|a| {
                let l = a.join(" ");
                stage
                    .as_ref()
                    .map_or(l.clone(), |s| l.replace(s, "<stage>"))
            })
            .collect()
    }

    /// App Store installs publish their percentage; other verbs run plainly.
    #[test]
    fn store_installs_publish_progress() {
        let os = Fake::default();
        for (a, t) in [
            (AdminAction::PkgInstall, "gimp"),
            (AdminAction::FlatpakInstall, "org.gimp.GIMP"),
            (AdminAction::PkgRemove, "gimp"),
        ] {
            let r = req(a, &[t]);
            assert_eq!(
                handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
                AdminReply::Ok
            );
        }
        assert_eq!(
            *os.progress.borrow(),
            [
                "/run/wolos/progress/gimp",
                "/run/wolos/progress/org.gimp.GIMP"
            ]
        );
        assert_eq!(os.ran.borrow().len(), 3);
    }

    /// A package that installed leaves modules-install's gave-up list (Settings > Modules Retry);
    /// a failed install, another name or no list leaves it alone.
    #[test]
    fn an_installed_package_leaves_the_gave_up_list() {
        let file = wol_core::modules::GAVE_UP_FILE;
        let list = "m dev\np gst-libav\nm ai-local\n";
        let os = Fake {
            files: vec![(file.into(), list.into())],
            ..Fake::default()
        };
        let go = |os: &Fake, name: &str| {
            let r = req(AdminAction::PkgInstall, &[name]);
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), os)
        };
        assert_eq!(go(&os, "wolos-module-dev"), AdminReply::Ok);
        assert_eq!(go(&os, "gst-libav"), AdminReply::Ok);
        assert_eq!(go(&os, "lame"), AdminReply::Ok);
        let wrote = os.wrote.borrow().clone();
        assert_eq!(
            wrote,
            [
                (file.to_string(), "p gst-libav\nm ai-local\n".to_string()),
                (file.to_string(), "m dev\nm ai-local\n".to_string()),
            ],
            "one write per name on the list; `lame` was not there"
        );
        let failing = Fake {
            fail_run: true,
            files: vec![(file.into(), list.into())],
            ..Fake::default()
        };
        assert_eq!(go(&failing, "wolos-module-dev"), AdminReply::Failed);
        assert!(
            failing.wrote.borrow().is_empty(),
            "a failed install keeps its line"
        );
        assert_eq!(go(&Fake::default(), "wolos-module-dev"), AdminReply::Ok);
    }

    /// Signed `.wol`: verified on the daemon's own copy, then `pacman -U` (deps from the repos).
    #[test]
    fn wol_install_signed() {
        let os = pkg_os();
        let f = wol_file("signed", Some(&SEED), b"payload");
        let r = req(AdminAction::WolInstall, &[&f, "signed"]);
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        assert_eq!(ran(&os), ["pacman -U --noconfirm <stage>/payload.pkg"]);
        let stage = os.stage.borrow().clone().unwrap();
        assert!(!stage.exists(), "staging removed");
        // Same version installed: nothing runs.
        let os = Fake {
            pkg: [
                pkg_os().pkg,
                vec![("-Q wol-fixture", "wol-fixture 1.0\n".into())],
            ]
            .concat(),
            ..pkg_os()
        };
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        assert!(ran(&os).is_empty());
    }

    /// A `.wol` signed with a TEST author certificate (`CHAIN` + `SHA256.wsig`).
    fn author_wol(name: &str, p: &wol_core::woltrust::pki::Pki) -> String {
        use wol_core::package::{manifest, ustar};
        use wol_core::woltrust::{Purpose, make, pki::Pki};
        let info = INFO.replace("org.wolos.Fixture", "alice.fixture");
        let man = manifest(&[("Info.json", info.as_bytes()), ("payload.pkg", b"payload")]);
        let chain = Pki::chain(&[&p.alice, &p.authors]);
        let w = make::wsig(
            &p.alice.body.serial,
            &p.alice.secret,
            Purpose::Package,
            man.as_bytes(),
        );
        let m: Vec<(&str, &[u8])> = vec![
            ("Info.json", info.as_bytes()),
            ("payload.pkg", b"payload"),
            ("SHA256", man.as_bytes()),
            ("CHAIN", &chain),
            ("SHA256.wsig", &w),
        ];
        let path =
            std::env::temp_dir().join(format!("wol-admind-{}-{name}.wol", std::process::id()));
        std::fs::write(&path, ustar(&m)).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn author_store(p: &wol_core::woltrust::pki::Pki, revoke: bool) -> wol_core::trust::TrustStore {
        use wol_core::woltrust::pki::{DAY, NOW};
        use wol_core::woltrust::{LIST_LIFETIME, Reason, Store, make, revoked};
        let entries = if revoke {
            vec![revoked::Entry {
                serial: p.alice.body.serial.clone(),
                at: NOW - DAY,
                reason: Reason::KeyCompromise,
            }]
        } else {
            vec![]
        };
        let l = revoked::ListText {
            format: 1,
            serial: 4,
            issued: NOW - DAY,
            expires: NOW - DAY + LIST_LIFETIME,
            entries,
        };
        let lists = [make::revocation_list(
            &l,
            &p.index.body.serial,
            &p.index.secret,
        )];
        wol_core::trust::TrustStore {
            store: Store::load(
                &[&p.root_a.file],
                &[&p.authors.file, &p.index.file],
                &lists,
                NOW,
                true,
            ),
        }
    }

    /// ADR 0046: wol-admind judges the staged copy with the certificate store, as root.
    #[test]
    fn wol_install_checks_the_author_certificate_again() {
        let p = wol_core::woltrust::pki::Pki::new();
        let f = author_wol("author", &p);
        let os = Fake {
            trust: author_store(&p, false),
            pkg: vec![("payload.pkg", "wol-fixture 1.0\n".into())],
            wall: wol_core::woltrust::pki::NOW,
            ..Fake::default()
        };
        let r = req(AdminAction::WolInstall, &[&f, "signed"]);
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        assert!(
            os.log
                .borrow()
                .iter()
                .any(|l| l.contains("verified author")),
            "{:?}",
            os.log.borrow()
        );
        let os = Fake {
            trust: author_store(&p, true),
            wall: wol_core::woltrust::pki::NOW,
            ..pkg_os()
        };
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Failed
        );
        assert!(os.ran.borrow().is_empty());
        assert!(
            os.log
                .borrow()
                .iter()
                .any(|l| l.contains("revoked-key-compromise")),
            "{:?}",
            os.log.borrow()
        );
        // With no store (no roots shipped yet) the chain ends nowhere: refused, never installed.
        let os = Fake {
            wall: wol_core::woltrust::pki::NOW,
            ..pkg_os()
        };
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Failed
        );
        assert!(
            os.log
                .borrow()
                .iter()
                .any(|l| l.contains("refused (issuer-unknown)"))
        );
    }

    /// Refusals: nothing runs, the log says why.
    #[test]
    fn wol_install_refusals() {
        let signed = wol_file("r-signed", Some(&SEED), b"payload");
        let unsigned = wol_file("r-unsigned", None, b"payload");
        let other_key = wol_file("r-other", Some(&[9; 32]), b"payload");
        let tampered = wol_file("r-tampered", Some(&SEED), b"payload");
        let mut b = std::fs::read(&tampered).unwrap();
        // The payload bytes (the header name is "payload.pkg\0").
        let at = b.windows(8).position(|w| w == b"payload\0").unwrap();
        b[at] = b'P';
        std::fs::write(&tampered, b).unwrap();
        let mismatch = Fake {
            pkg: vec![("payload.pkg", "other 1.0\n".into())],
            ..pkg_os()
        };
        let cases: [(&str, &str, Fake, &str); 4] = [
            (&unsigned, "signed", pkg_os(), "unsigned"),
            (&other_key, "signed", pkg_os(), "trusted key"),
            (&tampered, "unsigned", pkg_os(), "hash mismatch"),
            (&signed, "signed", mismatch, "does not match"),
        ];
        for (f, trust, os, why) in cases {
            let r = req(AdminAction::WolInstall, &[f, trust]);
            assert_eq!(
                handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
                AdminReply::Failed,
                "{why}"
            );
            assert!(os.ran.borrow().is_empty(), "{why}: {:?}", os.ran.borrow());
            assert!(
                os.log.borrow().iter().any(|l| l.contains(why)),
                "{why}: {:?}",
                os.log.borrow()
            );
        }
    }

    /// Unsigned needs the password every time, even with a cached authorization.
    #[test]
    fn opening_the_aur_always_checks_the_password() {
        let os = Fake::default();
        let mut cache = AuthCache::default();
        let ann = Some(Caller {
            uid: 1001,
            session: 7,
        });
        let open = |pw: &[u8]| {
            encode(
                &AdminRequest {
                    action: AdminAction::AurOpen,
                    args: vec![],
                },
                pw,
            )
        };
        assert_eq!(
            handle(&open(b""), ann, &mut cache, &os),
            AdminReply::NeedsAuth
        );
        assert_eq!(
            handle(&open(b"bad"), ann, &mut cache, &os),
            AdminReply::Denied
        );
        assert_eq!(handle(&open(b"good"), ann, &mut cache, &os), AdminReply::Ok);
        // A password given a moment ago does not open it again.
        assert_eq!(
            handle(&open(b""), ann, &mut cache, &os),
            AdminReply::NeedsAuth
        );
        let bob = Some(Caller {
            uid: 1002,
            session: 8,
        });
        assert_eq!(
            handle(&open(b"good"), bob, &mut cache, &os),
            AdminReply::NotAdmin
        );
        assert!(os.ran.borrow().is_empty());
    }

    #[test]
    fn wol_install_unsigned_needs_a_fresh_password() {
        let os = pkg_os();
        let mut cache = AuthCache::default();
        cache.grant(1001, 7, os.now());
        let f = wol_file("fresh", None, b"payload");
        let r = req(AdminAction::WolInstall, &[&f, "unsigned"]);
        assert_eq!(
            handle(&encode(&r, b""), ANN, &mut cache, &os),
            AdminReply::NeedsAuth
        );
        assert_eq!(
            handle(&encode(&r, b"good"), ANN, &mut cache, &os),
            AdminReply::Ok
        );
        assert_eq!(
            ran(&os).last().unwrap(),
            "pacman -U --noconfirm <stage>/payload.pkg"
        );
        // Signed installs use the cache as before.
        let s = wol_file("fresh-signed", Some(&SEED), b"payload");
        let r = req(AdminAction::WolInstall, &[&s, "signed"]);
        assert_eq!(
            handle(&encode(&r, b""), ANN, &mut cache, &os),
            AdminReply::Ok
        );
    }

    #[test]
    fn wol_remove_reads_the_install_record() {
        let os = Fake {
            files: vec![(
                "/usr/share/wolos/packages/org.wolos.Fixture/Info.json".into(),
                INFO.into(),
            )],
            ..Fake::default()
        };
        let r = req(AdminAction::WolRemove, &["org.wolos.Fixture"]);
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Ok
        );
        assert_eq!(ran(&os), ["pacman -Rns --noconfirm wol-fixture"]);
        let r = req(AdminAction::WolRemove, &["org.wolos.Missing"]);
        let os = Fake::default();
        assert_eq!(
            handle(&encode(&r, b""), ROOT, &mut AuthCache::default(), &os),
            AdminReply::Failed
        );
        assert!(os.ran.borrow().is_empty());
    }

    #[test]
    fn caller_may_read_table() {
        // (file uid, mode, caller uid, ok)
        for (owner, mode, caller, ok) in [
            (1001, 0o600, Some(1001), true),
            (1002, 0o600, Some(1001), false),
            (1002, 0o644, Some(1001), true),
            (0, 0o600, Some(0), true),
            (0, 0o600, None, false),
            (1001, 0o044, Some(1001), true),
            (1001, 0o040, Some(1001), false),
        ] {
            assert_eq!(
                caller_may_read(owner, mode, caller),
                ok,
                "{owner} {mode:o} {caller:?}"
            );
        }
    }

    #[test]
    fn a_file_inside_a_private_directory_is_not_reachable() {
        let root = (0, 0o755);
        let home = (0, 0o755);
        let mine = (1001, 0o700);
        let theirs = (1002, 0o700);
        assert!(caller_may_reach(&[mine, home, root], Some(1001)));
        assert!(!caller_may_reach(&[theirs, home, root], Some(1001)));
        assert!(!caller_may_reach(&[(1002, 0o750), home, root], Some(1001)));
        assert!(caller_may_reach(&[theirs, home, root], Some(0)));
        assert!(!caller_may_reach(&[root], None));
        for (p, ok) in [
            ("/home/a/x.wol", true),
            ("/home/a/../b/x.wol", false),
            ("home/a/x.wol", false),
            ("/home//a", false),
            ("/home/./a", false),
        ] {
            assert_eq!(is_plain_absolute(p), ok, "{p}");
        }
    }

    /// Audit S4-7: the deadline is for the whole request, not per read: a client that trickles
    /// a byte at a time is cut off at the deadline, an idle one too.
    #[test]
    fn a_trickling_client_is_cut_off_at_the_deadline() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let (srv, cli) = UnixStream::pair().unwrap();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let trickle = std::thread::spawn(move || {
            // One byte every 50 ms until told to stop (the channel's timeout is the pause).
            while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                stop_rx.recv_timeout(std::time::Duration::from_millis(50))
            {
                if (&cli).write_all(b"x").is_err() {
                    break;
                }
            }
        });
        let mut buf = vec![0u8; 1024];
        let t = std::time::Instant::now();
        let n = read_request(&srv, &mut buf, std::time::Duration::from_millis(300));
        let took = t.elapsed();
        assert!(took < std::time::Duration::from_millis(900), "{took:?}");
        assert!(n < 20, "{n} bytes");
        let _ = stop_tx.send(());
        trickle.join().unwrap();
        let (srv, _idle) = UnixStream::pair().unwrap();
        let t = std::time::Instant::now();
        assert_eq!(
            read_request(&srv, &mut buf, std::time::Duration::from_millis(200)),
            0
        );
        assert!(t.elapsed() < std::time::Duration::from_millis(800));
    }

    /// Audit S4-7: a password check that stalls (PAM) is killed at its timeout and counts as
    /// not accepted; the daemon is not frozen.
    #[test]
    fn a_stalled_password_check_is_a_no_not_a_hang() {
        let mut slow = std::process::Command::new("sleep");
        slow.arg("10");
        let t = std::time::Instant::now();
        assert!(!check_password(
            slow,
            b"pw",
            std::time::Duration::from_millis(300)
        ));
        assert!(
            t.elapsed() < std::time::Duration::from_secs(3),
            "{:?}",
            t.elapsed()
        );
        let ok = std::process::Command::new("true");
        assert!(check_password(ok, b"pw", AUTH_TIMEOUT));
        let no = std::process::Command::new("false");
        assert!(!check_password(no, b"pw", AUTH_TIMEOUT));
    }

    /// Audit S4-7 and ADR 0033 on wol-admind: an idle client does not hold the daemon (a second
    /// client's `revoke` is answered within a second), and a request sent without answering the
    /// hello gets no answer at all.
    #[test]
    fn an_idle_client_does_not_hold_wol_admind() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        static OS: RealOs = RealOs;
        let dir = std::env::temp_dir().join(format!("wol-admind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("admin.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || serve(&listener, &OS));
        let _idle = UnixStream::connect(&path).unwrap();
        let req = shared::admin::AdminRequest {
            action: AdminAction::Revoke,
            args: Vec::new(),
        };
        let t = std::time::Instant::now();
        let mut s = shared::jobs::connect(&path).unwrap();
        s.write_all(&shared::admin::encode(&req, b"")).unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        s.read_to_string(&mut line).unwrap();
        assert_eq!(AdminReply::parse(&line), AdminReply::Ok, "{line}");
        assert!(
            t.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            t.elapsed()
        );
        // No hello answered: the request is never read and nothing comes back.
        let mut raw = UnixStream::connect(&path).unwrap();
        raw.write_all(&shared::admin::encode(&req, b"")).unwrap();
        raw.shutdown(std::net::Shutdown::Write).unwrap();
        raw.set_read_timeout(Some(std::time::Duration::from_secs(8)))
            .unwrap();
        let mut got = String::new();
        let _ = raw.read_to_string(&mut got);
        assert!(!got.contains("ok"), "{got}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
