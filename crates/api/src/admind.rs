//! wol-admind request handling (IaraOS ADR 0026): allow-list only, judged by peer credentials.
use shared::admin::{AdminAction, AdminReply, decode};
use std::path::{Path, PathBuf};
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
    /// Accounts with a desktop session on this machine (a Hyprland socket in their runtime dir).
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
    fn remove_dir(&self, dir: &Path);
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
    match decide(
        known,
        admin || user_level,
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
        _ => execute(&req, os),
    };
    os.log(&format!(
        "{who}: {what}: {}",
        if ok { "ok" } else { "failed" }
    ));
    if ok {
        AdminReply::Ok
    } else {
        AdminReply::Failed
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
    let long = matches!(
        req.action,
        AdminAction::PkgUpgrade
            | AdminAction::PkgInstall
            | AdminAction::PkgRemove
            | AdminAction::FlatpakInstall
            | AdminAction::FlatpakRemove
    );
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
    let staged = dir.join("app.wol");
    os.copy_in(src, &staged, caller)?;
    let v = package::verify(&staged, &os.trusted_keys())?;
    if v.trust == Trust::Unsigned && !allow_unsigned {
        return Err("unsigned package; approve it as unsigned".into());
    }
    let payload = dir.join(shared::package::PAYLOAD);
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
        Some(u) => file_uid == u || mode & 0o004 != 0,
        None => false,
    }
}

/// Real OS: fixed argv with timeouts, atomic writes, PAM via the wol-authcheck helper, syslog via logger(1).
pub struct RealOs;

/// Helper contract: run as root with `--admin <user>`, password on stdin, exit 0 = authenticated.
pub const AUTHCHECK: &str = "/usr/local/libexec/wol-authcheck";

impl AdminOs for RealOs {
    fn passwd(&self) -> String {
        std::fs::read_to_string("/etc/passwd").unwrap_or_default()
    }
    fn group(&self) -> String {
        std::fs::read_to_string("/etc/group").unwrap_or_default()
    }
    fn authenticate(&self, user: &str, password: &[u8]) -> bool {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let Ok(mut child) = Command::new(AUTHCHECK)
            .args(["--admin", user])
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let wrote = child
            .stdin
            .take()
            .is_some_and(|mut s| s.write_all(password).is_ok());
        child.wait().is_ok_and(|s| s.success()) && wrote
    }
    fn run(&self, argv: &[String], long: bool) -> bool {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let t = std::time::Duration::from_secs(if long { 1800 } else { 30 });
        wol_core::system::run_with_timeout(&argv, t).is_some()
    }
    fn run_progress(&self, argv: &[String], kind: wol_core::progress::Kind, path: &str) -> bool {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        wol_core::progress::run_tracked(&argv, kind, path, std::time::Duration::from_secs(1800))
    }
    fn run_input(&self, argv: &[String], input: &str) -> bool {
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        wol_core::controls::run_with_input(&argv, input, std::time::Duration::from_secs(30))
            .is_some()
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
        wol_core::system::run_with_timeout(&argv, std::time::Duration::from_secs(30))
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
        let d = root.join(format!(
            "{}-{}",
            std::process::id(),
            shared::sys::monotonic_ns()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&d).ok()?;
        Some(d)
    }
    fn copy_in(&self, src: &str, dst: &Path, caller: Option<u32>) -> Result<(), String> {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(src)
            .map_err(|e| e.to_string())?;
        let m = f.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.len() > shared::package::MAX_SIZE {
            return Err("not a package file".into());
        }
        if !caller_may_read(m.uid(), m.mode(), caller) {
            return Err("caller cannot read this file".into());
        }
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dst)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut (&mut f).take(shared::package::MAX_SIZE), &mut out)
            .map_err(|e| e.to_string())?;
        out.sync_all().map_err(|e| e.to_string())
    }
    fn trusted_keys(&self) -> Vec<[u8; 32]> {
        package::read_keys(Path::new(shared::package::KEYS_DIR))
    }
    fn remove_dir(&self, dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
    fn log(&self, text: &str) {
        eprintln!("wol-admind: {text}");
        let _ = wol_core::system::run_with_timeout(
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

/// temp file + fsync + rename, then mode. Parent dir must exist.
pub fn atomic_write(path: &std::path::Path, mode: u32, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let tmp = path.with_extension(format!("wol-tmp-{}", std::process::id()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, path)
}

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
        /// (`pacman -Q` command line suffix, stdout); unmatched queries fail.
        pkg: Vec<(&'static str, String)>,
        /// (path, text) for `read`.
        files: Vec<(String, String)>,
        stage: RefCell<Option<std::path::PathBuf>>,
        /// Progress files of tracked runs.
        progress: RefCell<Vec<String>>,
    }

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
        fn remove_dir(&self, d: &std::path::Path) {
            let _ = std::fs::remove_dir_all(d);
        }
        fn wall_now(&self) -> u64 {
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
        ] {
            assert_eq!(
                caller_may_read(owner, mode, caller),
                ok,
                "{owner} {mode:o} {caller:?}"
            );
        }
    }
}
