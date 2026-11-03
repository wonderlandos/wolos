// SPDX-License-Identifier: BSD-2-Clause
//! The process model (ADR 0042): one app is one scope and one host process; the engine's content
//! processes, one per site of the app's frames, run inside that scope. The host process
//! (`wol-webhost <manifest>`, WA-3) runs the engine; each content process is a child, the same
//! executable started with `--content-process TOKEN` ([`content_token`]). All of them live in one systemd scope of the user's manager, named exactly like the
//! scopes the Memory Saver makes when it adopts a window (`app-wol-<app id>-<pid>.scope`,
//! `crates/api/src/memsaver/procs.rs` `scope_name`): the saver finds it as an app scope and does not
//! adopt it again, and the frontmost QoS boosts and parks the whole app at once. The launcher calls
//! [`scope_argv`] with its own pid and then `exec`s the result, so the pid in the name is the
//! scope's main process.

/// Prefix of a web app's id (plan-wave3 WA-3: `org.wolos.webapp.<slug>`).
pub const APP_ID_PREFIX: &str = "org.wolos.webapp.";
/// Tasks one app may hold, as the Memory Saver's own app scopes (`procs::APP_TASKS`, ADR 0027).
pub const APP_TASKS: u32 = 8192;
/// The argument the engine starts its content process with.
pub const CONTENT_FLAG: &str = "--content-process";

/// The app id of the web app named `name` (`None` when the name has no letter or digit).
pub fn app_id(name: &str) -> Option<String> {
    wol_core::webapps::slug(name).map(|s| format!("{APP_ID_PREFIX}{s}"))
}

/// The scope unit of the app's host process `pid`, as the Memory Saver names app scopes.
pub fn scope_unit(app_id: &str, pid: u32) -> String {
    format!("app-wol-{}-{pid}.scope", shared::memsaver::app_id(app_id))
}

/// `systemd-run` argv that runs `host` (the host program and its arguments) in the app's scope of
/// the user's manager. `name` is what Settings and `systemctl --user status` show.
pub fn scope_argv(app_id: &str, name: &str, pid: u32, host: &[String]) -> Vec<String> {
    let mut a: Vec<String> = ["systemd-run", "--user", "--scope", "--quiet", "--collect"]
        .map(String::from)
        .into();
    a.push(format!("--unit={}", scope_unit(app_id, pid)));
    a.push(format!("--description={name} (WolOS app)"));
    a.push(format!("--property=TasksMax={APP_TASKS}"));
    a.push("--".into());
    a.extend(host.iter().cloned());
    a
}

/// The token when this process was started as the engine's content process
/// (`<exe> --content-process TOKEN`): the host then calls [`crate::ffi::run_content_process`]
/// before anything else.
pub fn content_token(args: &[String]) -> Option<&str> {
    match args {
        [_, flag, token, ..] if flag == CONTENT_FLAG => Some(token),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_and_scopes_follow_the_memory_saver_names() {
        let id = app_id("My Notes").unwrap();
        assert_eq!(id, "org.wolos.webapp.my-notes");
        assert_eq!(app_id("!!!"), None);
        let unit = scope_unit(&id, 4242);
        assert_eq!(unit, "app-wol-org.wolos.webapp.my-notes-4242.scope");
        // What the saver recognises as an app scope and leaves alone (memsaver/engine.rs adopt).
        let cg = format!("/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}");
        assert!(cg.contains("/app-wol-"));
        let part = cg.rsplit('/').next().unwrap();
        assert!(part.starts_with("app-") && part.ends_with(".scope"));
    }

    #[test]
    fn the_host_runs_in_its_scope_with_its_arguments_untouched() {
        let host = vec![
            "/usr/bin/wol-webhost".to_string(),
            "/a b/notes.toml".to_string(),
        ];
        let a = scope_argv("org.wolos.webapp.notes", "Notes", 7, &host);
        assert_eq!(
            a[..5],
            ["systemd-run", "--user", "--scope", "--quiet", "--collect"]
        );
        assert!(a.contains(&"--unit=app-wol-org.wolos.webapp.notes-7.scope".to_string()));
        assert!(a.contains(&"--description=Notes (WolOS app)".to_string()));
        assert!(a.contains(&"--property=TasksMax=8192".to_string()));
        let dash = a.iter().position(|x| x == "--").unwrap();
        assert_eq!(a[dash + 1..], host[..]);
    }

    #[test]
    fn the_content_process_is_told_by_its_first_argument() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            content_token(&v(&["h", "--content-process", "t0"])),
            Some("t0")
        );
        assert_eq!(
            content_token(&v(&["h", "notes.toml", "--content-process", "t"])),
            None
        );
        assert_eq!(content_token(&v(&["h", "--content-process"])), None);
        assert_eq!(content_token(&v(&["h"])), None);
    }
}
