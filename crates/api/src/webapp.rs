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
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);

/// How long a call of `method` may run: [`DEADLINE`], or for an administrator method the sheet's
/// wait plus wol-admind's long steps (Update Now, a Doctor fix of up to
/// `shared::shell::MAX_DOCTOR_STEPS` package installs), as `wol` itself waits for wold.
pub fn deadline(method: &str) -> std::time::Duration {
    use wol_core::admin::{LONG_STEP_SECS, REPLY_MARGIN_SECS};
    if !sysapp::permission_of(method).is_some_and(|p| p.admin) {
        return DEADLINE;
    }
    let steps = shared::shell::MAX_DOCTOR_STEPS as u64;
    std::time::Duration::from_secs(
        u64::from(shared::shell::APPROVAL_WAIT_SECS)
            + steps * u64::from(LONG_STEP_SECS)
            + u64::from(REPLY_MARGIN_SECS)
            + DEADLINE.as_secs(),
    )
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
    /// `caller` is the app's host (its uid is the account the verb runs as).
    fn run(&self, caller: &Caller, app: &str, method: &str, params: &str) -> Result<Value, String>;
}

fn denied(message: &str) -> Answer {
    Answer::err(Code::PermissionDenied, message)
}

/// The app's folder and whether it is a system app: the sealed image's, then the packages'. The
/// folder is canonical (a symlink does not move an app out of its tree).
fn folder(dirs: &Dirs, slug: &str) -> Option<(PathBuf, bool)> {
    [(&dirs.system, true), (&dirs.opt, false)]
        .into_iter()
        .find_map(|(root, system)| {
            let dir = std::fs::canonicalize(root.join(slug)).ok()?;
            (dir.starts_with(std::fs::canonicalize(root).ok()?) && dir.join("app.toml").is_file())
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
    let Ok(decl) = std::fs::read_to_string(dir.join("app.toml"))
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
    match verbs.run(c, app, method, params) {
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
    fn run(&self, caller: &Caller, app: &str, method: &str, params: &str) -> Result<Value, String> {
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
        let code = crate::reaper::run(&mut cmd, deadline(method));
        drop(cmd);
        let out = reading
            .join()
            .map_err(|_| "The answer could not be read.")?;
        if code == crate::reaper::TIMED_OUT {
            return Err("It took too long, so WolOS stopped it.".into());
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
        fn run(&self, c: &Caller, app: &str, method: &str, params: &str) -> Result<Value, String> {
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

    #[test]
    fn an_administrator_method_may_wait_for_the_sheet_and_wol_admind() {
        assert_eq!(deadline("about"), DEADLINE);
        for m in ["update.install", "doctor.fix"] {
            assert!(deadline(m) > std::time::Duration::from_secs(3600), "{m}");
        }
    }

    #[test]
    fn a_system_app_holds_what_it_declares_and_the_verb_runs_as_its_account() {
        let (d, v) = (world("sys"), Fake::default());
        let a = handle(
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
        let a = handle(
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
            let a = handle(m, Some(&who), Some("org.wolos.webapp.doctor"), &d, &g, &v);
            assert_eq!(code(&a), Some(Code::PermissionDenied), "{m}");
        }
        // No scope: wold does not guess, and an id in the request is no id.
        let a = handle("about", Some(&who), None, &d, &g, &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        // A scope that names an app with no folder.
        let a = handle(
            "about",
            Some(&who),
            Some("org.wolos.webapp.ghost"),
            &d,
            &g,
            &v,
        );
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        let a = handle("about", Some(&who), Some("../../etc"), &d, &g, &v);
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
            let a = handle(
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
        let a = handle("audio.get", Some(&who), Some(notes), &d, &Held(vec![]), &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        // Granted by folder: audio-info opens audio.read.
        let g = Held(vec![(key, "audio-info")]);
        assert!(handle("audio.get", Some(&who), Some(notes), &d, &g, &v).ok);
        // The grant of the capability is per permission: audio-info does not open audio.control.
        let a = handle("audio.volume", Some(&who), Some(notes), &d, &g, &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        // system.read has no consent capability: no grant ever opens it for others.
        let g = Held(vec![(key, "audio-info"), ("/other", "audio-info")]);
        let a = handle("about", Some(&who), Some(notes), &d, &g, &v);
        assert_eq!(code(&a), Some(Code::PermissionDenied));
        // A grant for another folder is not this app's.
        let other = Held(vec![("/home/x/notes", "audio-info")]);
        assert_eq!(
            code(&handle(
                "audio.get",
                Some(&who),
                Some(notes),
                &d,
                &other,
                &v
            )),
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
        let a = handle(
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
            let a = handle(
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
        let a = handle(
            "about",
            Some(&host(1000)),
            Some("org.wolos.webapp.evil"),
            &d,
            &Held(vec![]),
            &v,
        );
        assert_eq!(code(&a), Some(Code::PermissionDenied));
    }
}
