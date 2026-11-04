// SPDX-License-Identifier: GPL-2.0-or-later
//! `webapp-call <method> [<params JSON>]` (ADR 0048): one call of a system web app, asked by its
//! host process (`/usr/bin/wol-webhost`). wold is the authority; the host's own check is only early.
//!
//! What wold believes, and from where. The caller is the host only when the kernel says its
//! executable is [`HOST`] (`peer.rs`, unchanged since accept). The app id is the app scope the
//! kernel puts that process in (`app-wol-<app id>-<pid>-<n>.scope`, `wol_core::sysapp::app_of_cgroup`),
//! never a word of the request. The permissions are read from the app's own `app.toml` in the sealed
//! image (`<system>/<slug>/app.toml`) or, for an app of others, from `<opt>/<slug>/app.toml`. The
//! method must be in the catalog (`wol_core::sysapp`), its permission declared there, and held:
//! a system app holds what it declares; another app holds a permission only through a consent grant
//! whose app key is its folder (`/etc/wolos/consent.json`, Settings > Privacy). A permission with
//! no consent capability is for system apps only. Then the verb runs as the caller's account
//! through `wol webapp-call`, the same parser and handlers as `wol mcp`, so wold adds no verb.
use crate::portal::{Caller, Grants};
use serde_json::{Value, json};
use shared::portal::{Answer, Code};
use std::path::PathBuf;
use wol_core::sysapp;

/// The one executable that may ask.
pub const HOST: &str = "/usr/bin/wol-webhost";
/// What runs the verb.
pub const WOL: &str = "/usr/bin/wol";
/// Longest a call may run, and the most of its reply wold keeps.
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(sysapp::CALL_SECS);

/// How many of wol-admind's long steps a call of `method` may need, from what wold can know before
/// the call runs: Update Now is one request. A Doctor fix names its own steps only to the process
/// that plans it, so wold allows the most one fix may send
/// ([`shared::shell::MAX_DOCTOR_STEPS`]) until it is told (a bound, never more).
pub fn steps_of(method: &str) -> usize {
    match method {
        "update.install" => 1,
        _ => shared::shell::MAX_DOCTOR_STEPS,
    }
}

/// How long a call of `method` may run when it needs `steps` of wol-admind's long steps:
/// [`DEADLINE`], or for an administrator method the sheet's wait, `steps` long steps and the reply
/// margin on top of it (`wol_core::sysapp::call_deadline_secs`, the one table the web app host
/// waits by too, QA-100 H9-03). The most is [`shared::shell::WEBAPP_CALL_DEADLINE_SECS`].
pub fn deadline(method: &str, steps: usize) -> std::time::Duration {
    std::time::Duration::from_secs(sysapp::call_deadline_secs(method, steps))
}
pub const MAX_REPLY: u64 = 1 << 20;

/// Where the apps' folders are (tests name their own).
pub struct Dirs {
    pub system: PathBuf,
    pub opt: PathBuf,
}

impl Default for Dirs {
    fn default() -> Dirs {
        Dirs {
            system: sysapp::SYSTEM_DIR.into(),
            opt: sysapp::OPT_DIR.into(),
        }
    }
}

/// Runs a verb for an account: the reply of `wol webapp-call` (`{"ok":..}` or `{"err":{..}}`).
pub trait Verbs {
    /// `caller` is the app's host (its uid is the account the verb runs as). `gone` says the host
    /// hung up (its window closed): the verb is stopped, with whatever it started, and nothing
    /// more is asked on screen (QA-100 H9-05).
    fn run(
        &self,
        caller: &Caller,
        app: &str,
        method: &str,
        params: &str,
        gone: &dyn Fn() -> bool,
    ) -> Result<Value, String>;
}

/// What wold read off the host's connection besides who it is.
pub struct Conn<'a> {
    /// The host's command line (its second word is the manifest it runs), read from the process
    /// the kernel pinned at accept; None when it cannot be read or changed.
    pub args: Option<&'a [String]>,
    /// True once the host is gone: its process ended (its window was closed) or is no longer the
    /// program it was at accept (`PeerIdentity::still_same`). A host that only finished writing
    /// its request has not hung up; one that is gone has no one to show a sheet to or answer.
    pub gone: &'a dyn Fn() -> bool,
}

/// Administrator calls waiting or running, one per (account, app): a web app that asks again
/// while its sheet is up or its fix runs is told it is busy, so one app never parks more than one
/// wold thread for the long deadline (QA-100 L5).
static ADMIN_CALLS: std::sync::Mutex<Vec<(u32, String)>> = std::sync::Mutex::new(Vec::new());

struct AdminSlot(u32, String);

impl AdminSlot {
    fn take(uid: u32, app: &str) -> Option<AdminSlot> {
        let mut t = ADMIN_CALLS.lock().unwrap_or_else(|p| p.into_inner());
        if t.iter().any(|(u, a)| *u == uid && a == app) {
            return None;
        }
        t.push((uid, app.to_owned()));
        Some(AdminSlot(uid, app.to_owned()))
    }
}

// Gives the place back when the call ends, however it ended.
impl Drop for AdminSlot {
    fn drop(&mut self) {
        let mut t = ADMIN_CALLS.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(i) = t.iter().position(|(u, a)| *u == self.0 && *a == self.1) {
            t.remove(i);
        }
    }
}

/// The host runs the manifest of the folder wold resolved: its first argument is that folder's
/// `app.toml`, absolute (a relative one would be read against a directory wold does not know).
/// A program that starts the installed host on a manifest of its own, whose id names a system app,
/// is not that app (QA-100 H9-08).
fn runs_manifest(args: Option<&[String]>, dir: &std::path::Path) -> bool {
    let Some(arg) = args.and_then(|a| a.get(1)) else {
        return false;
    };
    let path = std::path::Path::new(arg);
    path.is_absolute() && std::fs::canonicalize(path).is_ok_and(|p| p == dir.join(MANIFEST))
}

/// The file in an app's folder (`webengine::bridge::manifest::FILE`).
const MANIFEST: &str = "app.toml";

fn denied(message: &str) -> Answer {
    Answer::err(Code::PermissionDenied, message)
}

/// The app's folder and whether it is a system app: the sealed image's, then the packages'. The
/// folder is canonical (a symlink does not move an app out of its tree). None when the slug exists
/// in both trees.
fn folder(dirs: &Dirs, slug: &str) -> Option<(PathBuf, bool)> {
    // A slug in both trees names no app: a package under /opt must not be judged as the system
    // app of the same name (its permissions, its name on the sheet; QA-100 M1, ADR 0048).
    if [&dirs.system, &dirs.opt]
        .iter()
        .all(|root| root.join(slug).symlink_metadata().is_ok())
    {
        return None;
    }
    [(&dirs.system, true), (&dirs.opt, false)]
        .into_iter()
        .find_map(|(root, system)| {
            let dir = std::fs::canonicalize(root.join(slug)).ok()?;
            (dir.starts_with(std::fs::canonicalize(root).ok()?) && dir.join(MANIFEST).is_file())
                .then_some((dir, system))
        })
}

/// One `webapp-call` request. `rest` is the line after the word; `scope_app` is the app id of the
/// caller's scope, if it has one.
pub fn handle(
    rest: &str,
    caller: Option<&Caller>,
    scope_app: Option<&str>,
    dirs: &Dirs,
    grants: &dyn Grants,
    verbs: &dyn Verbs,
    conn: &Conn,
) -> Answer {
    let Some(c) = caller.filter(|c| c.app == HOST) else {
        return denied("Only an app's own host may ask for this.");
    };
    let Some(app) = scope_app else {
        return denied("WolOS could not tell which app asked, so it did nothing.");
    };
    let rest = rest.trim();
    let (method, params) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let params = params.trim();
    if params.len() > shared::shell::WEBAPP_MAX_PARAMS {
        return Answer::err(Code::InvalidArgument, "The request was too large.");
    }
    if method.is_empty()
        || serde_json::from_str::<Value>(if params.is_empty() { "{}" } else { params })
            .ok()
            .is_none_or(|v| !v.is_object())
    {
        return Answer::err(Code::InvalidArgument, "The request was not understood.");
    }
    let Some(perm) = sysapp::permission_of(method) else {
        return Answer::err(
            Code::InvalidArgument,
            "This app asked for something WolOS does not offer.",
        );
    };
    let Some(slug) = sysapp::slug_of(app) else {
        return denied("That is not a web app.");
    };
    let Some((dir, system)) = folder(dirs, slug) else {
        return denied("WolOS has no record of that app, so it did nothing.");
    };
    if !runs_manifest(conn.args, &dir) {
        return denied("WolOS could not tell which app asked, so it did nothing.");
    }
    let Ok(decl) = std::fs::read_to_string(dir.join(MANIFEST))
        .map_err(|e| e.to_string())
        .and_then(|t| sysapp::declared(&t))
    else {
        return denied("That app's description could not be read, so it did nothing.");
    };
    if decl.id != app || !decl.permissions.iter().any(|p| p == perm.name) {
        return denied("This app did not ask for that permission.");
    }
    if !system {
        // An app of others holds only what the person turned on for it.
        let key = dir.to_string_lossy();
        if !perm.consent.is_some_and(|cap| grants.granted(&key, cap)) {
            return denied("This app is not allowed to do that. Turn it on in Settings > Privacy.");
        }
    }
    // An administrator method waits for a sheet and then for wol-admind: one at a time per app.
    let _slot = if perm.admin {
        match AdminSlot::take(c.uid, app) {
            Some(slot) => Some(slot),
            None => {
                return Answer::err(
                    Code::Unavailable,
                    "This app already has a change waiting. Finish or cancel it first.",
                );
            }
        }
    } else {
        None
    };
    match verbs.run(c, app, method, params, conn.gone) {
        Ok(reply) => Answer::with(reply),
        Err(why) => Answer::err(Code::Unavailable, why),
    }
}

/// The real [`Verbs`]: `wol webapp-call` as the account, with nothing of wold's environment.
pub struct WolVerbs;

/// The account's session directory, from the host's own environment (`XDG_RUNTIME_DIR`): the
/// session put it there, wold does not guess a path from a uid. Only an absolute path is taken.
fn runtime_dir(pid: i32) -> Option<String> {
    use std::io::Read;
    let mut env = Vec::new();
    std::fs::File::open(format!("/proc/{pid}/environ"))
        .ok()?
        .take(1 << 20)
        .read_to_end(&mut env)
        .ok()?;
    env.split(|b| *b == 0)
        .find_map(|e| e.strip_prefix(b"XDG_RUNTIME_DIR="))
        .and_then(|v| String::from_utf8(v.to_vec()).ok())
        .filter(|v| v.starts_with('/') && !v.contains(['\n', '\0']))
}

impl Verbs for WolVerbs {
    fn run(
        &self,
        caller: &Caller,
        app: &str,
        method: &str,
        params: &str,
        gone: &dyn Fn() -> bool,
    ) -> Result<Value, String> {
        use std::io::Read;
        use std::os::unix::process::CommandExt;
        let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| e.to_string())?;
        let who =
            crate::services::identity(&passwd, caller.uid).ok_or("That account is not known.")?;
        let (mut reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
        // A cleared environment and a fixed PATH, as every program wold starts; then the account's own.
        let mut cmd = crate::privileged::privileged_command(WOL);
        cmd.args(["webapp-call", app, method, params])
            .uid(caller.uid)
            .gid(who.gid)
            .env("HOME", &who.home)
            .env("USER", &who.name)
            .env("LOGNAME", &who.name)
            .current_dir(&who.home)
            .stderr(std::process::Stdio::null())
            .stdout(writer);
        if let Some(dir) = runtime_dir(caller.pid) {
            cmd.env("XDG_RUNTIME_DIR", dir);
        }
        let reading = std::thread::Builder::new()
            .name("webapp-reply".into())
            .spawn(move || {
                let mut out = Vec::new();
                let _ = reader.by_ref().take(MAX_REPLY).read_to_end(&mut out);
                // Past the cap: drain so the child never blocks on a full pipe.
                let _ = std::io::copy(&mut reader, &mut std::io::sink());
                out
            })
            .map_err(|e| e.to_string())?;
        let code = crate::reaper::run_until(&mut cmd, deadline(method, steps_of(method)), gone);
        drop(cmd);
        let out = reading
            .join()
            .map_err(|_| "The answer could not be read.")?;
        if code == crate::reaper::TIMED_OUT {
            return Err("It took too long, so WolOS stopped it.".into());
        }
        if code == crate::reaper::CANCELLED {
            return Err("The app closed, so WolOS stopped it.".into());
        }
        let last = String::from_utf8_lossy(&out);
        serde_json::from_str::<Value>(last.lines().last().unwrap_or("").trim())
            .ok()
            .filter(|v| v.get("ok").is_some() || v.get("err").is_some())
            .ok_or_else(|| "WolOS could not run that.".to_string())
    }
}

/// An error reply for tests and the host's tooling: `{"err":{code,message,hint}}`.
pub fn err_reply(code: &str, message: &str, hint: &str) -> Value {
    json!({"err": {"code": code, "message": message, "hint": hint}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Held(Vec<(&'static str, &'static str)>);
    impl Grants for Held {
        fn granted(&self, app: &str, cap: &str) -> bool {
            self.0.iter().any(|(a, c)| *a == app && *c == cap)
        }
        fn shows_notifications(&self, _: &str) -> bool {
            false
        }
    }
    #[derive(Default)]
    struct Fake(RefCell<Vec<(u32, String, String, String)>>);
    impl Verbs for Fake {
        fn run(
            &self,
            c: &Caller,
            app: &str,
            method: &str,
            params: &str,
            _: &dyn Fn() -> bool,
        ) -> Result<Value, String> {
            self.0
                .borrow_mut()
                .push((c.uid, app.into(), method.into(), params.into()));
            Ok(json!({"ok": {"method": method}}))
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("wold-webapp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A system app `doctor` (system.read, audio.read) and an app of others `notes` (audio.read,
    /// system.read, audio.control).
    fn world(tag: &str) -> Dirs {
        let root = tmp(tag);
        let (sys, opt) = (root.join("usr"), root.join("opt"));
        for (base, slug, perms) in [
            (&sys, "doctor", "'system.read','audio.read'"),
            (&opt, "notes", "'audio.read','system.read','audio.control'"),
        ] {
            std::fs::create_dir_all(base.join(slug)).unwrap();
            std::fs::write(
                base.join(slug).join("app.toml"),
                format!("id='org.wolos.webapp.{slug}'\nname='X'\nversion='1.0.0'\nicon='i.svg'\npermissions=[{perms}]\n"),
            )
            .unwrap();
        }
        Dirs { system: sys, opt }
    }

    /// What the host of `app` was started with: its manifest, in the tree that has the app.
    fn host_args(d: &Dirs, app: Option<&str>) -> Option<Vec<String>> {
        let slug = sysapp::slug_of(app?)?;
        [&d.system, &d.opt].into_iter().find_map(|root| {
            let toml = std::fs::canonicalize(root.join(slug).join(MANIFEST)).ok()?;
            Some(vec![HOST.to_owned(), toml.to_string_lossy().into_owned()])
        })
    }

    /// [`handle`] for a host started on the app's own manifest, still connected.
    fn ask(
        rest: &str,
        caller: Option<&Caller>,
        app: Option<&str>,
        d: &Dirs,
        g: &dyn Grants,
        v: &dyn Verbs,
    ) -> Answer {
        let args = host_args(d, app);
        let conn = Conn {
            args: args.as_deref(),
            gone: &|| false,
        };
        handle(rest, caller, app, d, g, v, &conn)
    }

    fn host(uid: u32) -> Caller {
        Caller {
            uid,
            pid: 77,
            app: HOST.into(),
        }
    }

    fn code(a: &Answer) -> Option<Code> {
        a.code
    }

    /// QA-100 L5, H9-03: the deadline follows the step count: a plain call is bounded at once,
    /// an administrator call by the sheet and as many long steps as the fix has, never past the
    /// constant the web app host waits by.
    #[test]
    fn deadline_from_step_count() {
        let secs = |m, n| deadline(m, n).as_secs();
        assert_eq!(deadline("about", 0), DEADLINE);
        assert_eq!(
            deadline("about", 8),
            DEADLINE,
            "steps do not lengthen a plain call"
        );
        for m in ["update.install", "doctor.fix"] {
            assert!(secs(m, 1) > 1800, "{m}: one long step");
            assert_eq!(
                secs(m, 2) - secs(m, 1),
                1800,
                "{m}: one more step, 30 minutes more"
            );
            assert!(secs(m, 1) < secs(m, 8), "{m}");
            assert_eq!(secs(m, 0), secs(m, 1), "{m}: at least one step");
            assert_eq!(
                secs(m, 99),
                shared::shell::WEBAPP_CALL_DEADLINE_SECS,
                "{m}: never past the most"
            );
        }
        assert_eq!(steps_of("update.install"), 1, "Update Now is one request");
        assert!(secs("update.install", steps_of("update.install")) < 3 * 3600);
        assert_eq!(steps_of("doctor.fix"), shared::shell::MAX_DOCTOR_STEPS);
    }

    #[test]
    fn a_system_app_holds_what_it_declares_and_the_verb_runs_as_its_account() {
        let (d, v) = (world("sys"), Fake::default());
        let a = ask(
            "audio.get {\"x\":1}",
            Some(&host(1000)),
            Some("org.wolos.webapp.doctor"),
            &d,
            &Held(vec![]),
            &v,
        );
        assert!(a.ok, "{a:?}");
        assert_eq!(a.data, Some(json!({"ok": {"method": "audio.get"}})));
        assert_eq!(
            v.0.borrow()[0],
            (
                1000,
                "org.wolos.webapp.doctor".into(),
                "audio.get".into(),
                "{\"x\":1}".into()
            )
        );
        let a = ask(
            "about",
            Some(&host(1000)),
            Some("org.wolos.webapp.doctor"),
            &d,
            &Held(vec![]),
            &v,
        );
        assert!(a.ok);
        assert_eq!(v.0.borrow()[1].3, "", "no params is an empty string");
    }

    #[test]
    fn the_app_comes_from_the_scope_and_a_permission_must_be_declared() {
        let (d, v) = (world("decl"), Fake::default());
        let who = host(1000);
        let g = Held(vec![]);
        // doctor never declared games.read or audio.control.
        for m in ["games.doctor", "audio.volume"] {
            let a = ask(m, Some(&who), Some("org.wolos.webapp.doctor"), &d, &g, &v);
            assert_eq!(code(&a), Some(Code::PermissionDenied), "{m}");
        }
        // No scope: wold does not guess, and an id in the request is no id.
        let a = ask("about", Some(&who), None, &d, &g, &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        // A scope that names an app with no folder.
        let a = ask(
            "about",
            Some(&who),
            Some("org.wolos.webapp.ghost"),
            &d,
            &g,
            &v,
        );
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        let a = ask("about", Some(&who), Some("../../etc"), &d, &g, &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        assert!(v.0.borrow().is_empty(), "nothing ran");
    }

    #[test]
    fn only_the_host_binary_may_ask() {
        let (d, v) = (world("who"), Fake::default());
        for c in [
            None,
            Some(Caller {
                uid: 1000,
                pid: 5,
                app: "/usr/bin/wol".into(),
            }),
            Some(Caller {
                uid: 0,
                pid: 5,
                app: "/tmp/wol-webhost".into(),
            }),
        ] {
            let a = ask(
                "about",
                c.as_ref(),
                Some("org.wolos.webapp.doctor"),
                &d,
                &Held(vec![]),
                &v,
            );
            assert_eq!(code(&a), Some(Code::PermissionDenied), "{c:?}");
        }
        assert!(v.0.borrow().is_empty());
    }

    #[test]
    fn an_app_of_others_needs_a_consent_grant_for_its_folder_and_never_for_system_only_permissions()
    {
        let (d, v) = (world("opt"), Fake::default());
        let who = host(1000);
        let notes = "org.wolos.webapp.notes";
        let folder = std::fs::canonicalize(d.opt.join("notes")).unwrap();
        let key: &'static str = Box::leak(folder.to_string_lossy().into_owned().into_boxed_str());
        // Declared but not granted.
        let a = ask("audio.get", Some(&who), Some(notes), &d, &Held(vec![]), &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        // Granted by folder: audio-info opens audio.read.
        let g = Held(vec![(key, "audio-info")]);
        assert!(ask("audio.get", Some(&who), Some(notes), &d, &g, &v).ok);
        // The grant of the capability is per permission: audio-info does not open audio.control.
        let a = ask("audio.volume", Some(&who), Some(notes), &d, &g, &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        // system.read has no consent capability: no grant ever opens it for others.
        let g = Held(vec![(key, "audio-info"), ("/other", "audio-info")]);
        let a = ask("about", Some(&who), Some(notes), &d, &g, &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        // A grant for another folder is not this app's.
        let other = Held(vec![("/home/x/notes", "audio-info")]);
        assert_eq!(
            code(&ask("audio.get", Some(&who), Some(notes), &d, &other, &v)),
            Some(Code::PermissionDenied)
        );
        assert_eq!(v.0.borrow().len(), 1);
    }

    #[test]
    fn a_folder_that_names_another_app_is_refused_and_bad_requests_are_invalid() {
        let (d, v) = (world("id"), Fake::default());
        let who = host(1000);
        // doctor's folder holds a manifest for another id.
        std::fs::write(
            d.system.join("doctor/app.toml"),
            "id='org.wolos.webapp.other'\nname='X'\nversion='1.0.0'\nicon='i.svg'\npermissions=['system.read']\n",
        )
        .unwrap();
        let a = ask(
            "about",
            Some(&who),
            Some("org.wolos.webapp.doctor"),
            &d,
            &Held(vec![]),
            &v,
        );
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        let d = world("bad");
        for r in ["", "rm.rf", "about [1]", "about nope"] {
            let a = ask(
                r,
                Some(&who),
                Some("org.wolos.webapp.doctor"),
                &d,
                &Held(vec![]),
                &v,
            );
            assert_eq!(code(&a), Some(Code::InvalidArgument), "{r:?}");
        }
        assert!(v.0.borrow().is_empty());
    }

    #[test]
    fn a_folder_that_is_a_link_out_of_its_tree_is_not_an_app() {
        let (d, v) = (world("link"), Fake::default());
        let elsewhere = tmp("link-target");
        std::fs::write(
            elsewhere.join("app.toml"),
            "id='org.wolos.webapp.evil'\npermissions=['system.read']\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&elsewhere, d.system.join("evil")).unwrap();
        let a = ask(
            "about",
            Some(&host(1000)),
            Some("org.wolos.webapp.evil"),
            &d,
            &Held(vec![]),
            &v,
        );
        assert_eq!(code(&a), Some(Code::PermissionDenied));
    }
    /// QA-100 M1: a package under /opt that reuses a system app's slug is not that system app: the
    /// slug in both trees names nothing, for the system app too, until the package is gone.
    #[test]
    fn slug_in_both_roots_denied() {
        let (d, v) = (world("both"), Fake::default());
        let doctor = Some("org.wolos.webapp.doctor");
        assert!(ask("about", Some(&host(1000)), doctor, &d, &Held(vec![]), &v).ok);
        // A package ships /opt/wolos/webapps/doctor/app.toml with the system app's id.
        std::fs::create_dir_all(d.opt.join("doctor")).unwrap();
        std::fs::write(
            d.opt.join("doctor/app.toml"),
            "id='org.wolos.webapp.doctor'\nname='Doctor'\nversion='1.0.0'\nicon='i.svg'\npermissions=['system.read']\n",
        )
        .unwrap();
        for tree in [&d.system, &d.opt] {
            let toml = tree.join("doctor").join(MANIFEST);
            let args = vec![HOST.to_owned(), toml.to_string_lossy().into_owned()];
            let conn = Conn {
                args: Some(&args),
                gone: &|| false,
            };
            let a = handle(
                "about",
                Some(&host(1000)),
                doctor,
                &d,
                &Held(vec![]),
                &v,
                &conn,
            );
            assert_eq!(code(&a), Some(Code::PermissionDenied), "{tree:?}");
        }
        assert_eq!(v.0.borrow().len(), 1, "only the first call ran");
        // The folder lookup says so itself, and leaves the other apps alone.
        assert!(folder(&d, "doctor").is_none());
        assert!(folder(&d, "notes").is_some_and(|(_, system)| !system));
    }

    /// QA-100 H9-08: the host must be running the manifest of the folder wold resolved. A program
    /// that starts the installed host on a manifest of its own, with a system app's id, gets no
    /// answer as that app.
    #[test]
    fn host_manifest_argv_checked() {
        let (d, v) = (world("argv"), Fake::default());
        let doctor = Some("org.wolos.webapp.doctor");
        let mine = tmp("argv-mine");
        std::fs::write(
            mine.join("app.toml"),
            "id='org.wolos.webapp.doctor'\nname='X'\nversion='1.0.0'\nicon='i.svg'\npermissions=['system.read']\n",
        )
        .unwrap();
        let real = std::fs::canonicalize(d.system.join("doctor/app.toml")).unwrap();
        let real = real.to_string_lossy().into_owned();
        let own = mine.join("app.toml").to_string_lossy().into_owned();
        let link = mine.join("link.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let run = |args: Option<Vec<String>>| {
            let conn = Conn {
                args: args.as_deref(),
                gone: &|| false,
            };
            let a = handle(
                "about",
                Some(&host(1000)),
                doctor,
                &d,
                &Held(vec![]),
                &v,
                &conn,
            );
            (a.ok, code(&a))
        };
        let denied = (false, Some(Code::PermissionDenied));
        assert!(
            run(Some(vec![HOST.into(), real.clone()])).0,
            "its own manifest"
        );
        assert!(
            run(Some(vec![HOST.into(), real.clone(), "--flag".into()])).0,
            "more words"
        );
        assert!(
            run(Some(vec![HOST.into(), link.to_string_lossy().into_owned()])).0,
            "a link to it"
        );
        assert_eq!(
            run(Some(vec![HOST.into(), own])),
            denied,
            "another manifest, same id"
        );
        assert_eq!(
            run(Some(vec![HOST.into(), "app.toml".into()])),
            denied,
            "relative"
        );
        assert_eq!(run(Some(vec![HOST.into()])), denied, "no manifest word");
        assert_eq!(run(None), denied, "unreadable");
        assert_eq!(v.0.borrow().len(), 3);
    }

    /// QA-100 L5: one administrator call per app waits at a time; the second is busy at once and
    /// does not park another thread. A plain call is not held to it.
    #[test]
    fn a_second_admin_call_of_the_same_app_is_busy() {
        struct Parked(
            std::sync::mpsc::Sender<()>,
            std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
        );
        impl Verbs for Parked {
            fn run(
                &self,
                _: &Caller,
                _: &str,
                _: &str,
                _: &str,
                _: &dyn Fn() -> bool,
            ) -> Result<Value, String> {
                let _ = self.0.send(());
                let _ = self
                    .1
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(20));
                Ok(json!({"ok": {}}))
            }
        }
        let d = world("admin-slot");
        let admin_method = ["doctor.fix", "update.install"]
            .into_iter()
            .find(|m| sysapp::permission_of(m).is_some_and(|p| p.admin))
            .unwrap();
        let perm = sysapp::permission_of(admin_method).unwrap().name;
        std::fs::write(
            d.system.join("doctor/app.toml"),
            format!("id='org.wolos.webapp.doctor'\nname='X'\nversion='1.0.0'\nicon='i.svg'\npermissions=['system.read','{perm}']\n"),
        )
        .unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let parked = Parked(started_tx, std::sync::Mutex::new(release_rx));
        let doctor = Some("org.wolos.webapp.doctor");
        std::thread::scope(|sc| {
            let (d, parked) = (&d, &parked);
            let first = sc.spawn(move || {
                ask(
                    &format!("{admin_method} {{\"id\":\"x\"}}"),
                    Some(&host(1000)),
                    doctor,
                    d,
                    &Held(vec![]),
                    parked,
                )
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("the first call runs");
            let v = Fake::default();
            let again = ask(
                admin_method,
                Some(&host(1000)),
                doctor,
                d,
                &Held(vec![]),
                &v,
            );
            assert_eq!(code(&again), Some(Code::Unavailable), "{again:?}");
            assert!(v.0.borrow().is_empty(), "the second never ran");
            // Another account's call of the same app, and a plain call, are not held back.
            let other = ask(
                admin_method,
                Some(&host(1001)),
                doctor,
                d,
                &Held(vec![]),
                &v,
            );
            assert!(other.ok, "{other:?}");
            assert!(ask("about", Some(&host(1000)), doctor, d, &Held(vec![]), &v).ok);
            let _ = release_tx.send(());
            assert!(first.join().unwrap().ok);
        });
        // The place is given back.
        let v = Fake::default();
        assert!(
            ask(
                admin_method,
                Some(&host(1000)),
                doctor,
                &d,
                &Held(vec![]),
                &v
            )
            .ok
        );
    }
    /// QA-100 H9-19: one cap for the parameters, and a request at the cap still fits wold's line.
    #[test]
    fn params_cap_fits_the_request_line() {
        use shared::shell::WEBAPP_MAX_PARAMS;
        let longest = sysapp::PERMISSIONS
            .iter()
            .flat_map(|p| p.methods.iter())
            .map(|m| m.len())
            .max()
            .unwrap();
        let line = format!(
            "webapp-call {} {}\n",
            "m".repeat(longest),
            "x".repeat(WEBAPP_MAX_PARAMS)
        );
        assert!(line.len() <= shared::portal::MAX_LINE, "{}", line.len());
        let (d, v) = (world("cap"), Fake::default());
        let who = host(1000);
        let doctor = Some("org.wolos.webapp.doctor");
        let big = format!("about {{\"x\":\"{}\"}}", "y".repeat(WEBAPP_MAX_PARAMS));
        let a = ask(&big, Some(&who), doctor, &d, &Held(vec![]), &v);
        assert_eq!(code(&a), Some(Code::InvalidArgument));
        assert_eq!(a.message.as_deref(), Some("The request was too large."));
        let fits = format!("about {{\"x\":\"{}\"}}", "y".repeat(WEBAPP_MAX_PARAMS - 10));
        assert!(ask(&fits, Some(&who), doctor, &d, &Held(vec![]), &v).ok);
    }
    /// QA-100 H9-05: when the host's process is gone (its window closed), the verb wold started
    /// for it is stopped, with its process group, and the call ends in seconds, not at its
    /// deadline. The host here is a real child watched the way server.rs watches it
    /// (`PeerIdentity::still_same`); the verb a real `sleep`.
    #[test]
    fn pollhup_kills_child() {
        let mut host = std::process::Command::new("sleep")
            .arg("60")
            .env_clear()
            .spawn()
            .unwrap();
        let pid = i32::try_from(host.id()).unwrap();
        let id = crate::peer::PeerIdentity::of_process(pid, &[]);
        assert!(id.still_same(), "the host is itself");
        let gone = || !id.still_same();
        let started = std::time::Instant::now();
        let code = std::thread::scope(|sc| {
            let verb = sc.spawn(|| {
                let mut cmd = std::process::Command::new("sh");
                cmd.args(["-c", "sleep 60 & wait"]);
                crate::reaper::run_until(&mut cmd, std::time::Duration::from_secs(120), &gone)
            });
            // The window is closed: the host ends (and is reaped by its parent, here this test).
            host.kill().unwrap();
            host.wait().unwrap();
            verb.join().unwrap()
        });
        assert_eq!(code, crate::reaper::CANCELLED);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(30),
            "{:?}: stopped when the host went, not at the deadline",
            started.elapsed()
        );
    }
}
