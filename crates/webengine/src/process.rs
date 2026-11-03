// SPDX-License-Identifier: BSD-2-Clause
//! The process model (ADR 0042): one app is one scope and one host process; the engine's content
//! processes, one per site of the app's frames, run inside that scope. The host process
//! (`wol-webhost <manifest>`, WA-3) runs the engine; each content process is a child, the same
//! executable started with `--content-process TOKEN` ([`content_token`]). All of them live in one
//! systemd scope of the user's manager, named like the scopes the Memory Saver makes when it adopts
//! a window (`app-wol-<app id>-<pid>-<n>.scope`; the saver's are `app-wol-<app id>-<pid>.scope`,
//! `crates/api/src/memsaver/procs.rs` `scope_name`), under the limits of every app scope (ADR 0027:
//! 80 % of RAM, 25 % of swap, [`APP_TASKS`] tasks), so the app is limited from its first page even
//! with the Memory Saver off. With the saver on, it adopts the app into a scope of its own like any
//! app in a user-manager scope, and the frontmost QoS boosts and parks the whole app at once.
//! The launcher picks the unit with [`pick_unit`] from its own pid, calls [`scope_argv`] and then
//! `exec`s the result, so the pid in the name is the scope's main process; `<n>` keeps a reused pid
//! off a scope an earlier app left behind.

/// Prefix of a web app's id (plan-wave3 WA-3: `org.wolos.webapp.<slug>`).
pub const APP_ID_PREFIX: &str = "org.wolos.webapp.";
/// Tasks one app may hold, as the Memory Saver's own app scopes (`procs::APP_TASKS`, ADR 0027).
pub const APP_TASKS: u32 = 8192;
/// One app's share of RAM and of swap (ADR 0027), as the Memory Saver's own app scopes
/// (`procs::APP_MEMORY_SCALE`, `procs::APP_SWAP_SCALE`): `systemd-run` sends a percentage as the
/// same `MemoryMaxScale` and `MemorySwapMaxScale`.
pub const APP_MEMORY: &str = "80%";
pub const APP_SWAP: &str = "25%";
/// Names [`pick_unit`] tries before it settles on the last.
const UNIT_TRIES: u32 = 8;
/// The argument the engine starts its content process with.
pub const CONTENT_FLAG: &str = "--content-process";

/// The app id of the web app named `name` (`None` when the name has no letter or digit).
pub fn app_id(name: &str) -> Option<String> {
    wol_core::webapps::slug(name).map(|s| format!("{APP_ID_PREFIX}{s}"))
}

/// The scope unit of the app's host process `pid`, named like the Memory Saver's app scopes, with
/// `n` after the pid ([`pick_unit`]).
pub fn scope_unit(app_id: &str, pid: u32, n: u32) -> String {
    format!(
        "app-wol-{}-{pid}-{n:x}.scope",
        shared::memsaver::app_id(app_id)
    )
}

/// A scope unit for the host process `pid` that `exists` does not know: a scope an earlier
/// process of the same pid left behind (an abandoned scope keeps its name while anything is left
/// in it) would make `systemd-run` refuse to start. `n` is the clock with a per-process counter in
/// its low byte, so two launches of one pid do not pick the same name; when [`UNIT_TRIES`] names
/// all exist the next one is used unasked (and `systemd-run` says why it fails, if it does).
pub fn pick_unit(app_id: &str, pid: u32, mut exists: impl FnMut(&str) -> bool) -> String {
    static LAUNCH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let launch = LAUNCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let start = (clock & !0xff) | (launch & 0xff);
    let mut unit = scope_unit(app_id, pid, start);
    for i in 1..=UNIT_TRIES {
        if !exists(&unit) {
            return unit;
        }
        unit = scope_unit(app_id, pid, start.wrapping_add(i << 8));
    }
    unit
}

/// Whether the user's manager has a unit of this name ([`pick_unit`]'s `exists` for the launcher).
/// `false` when it cannot be asked: `systemd-run` then says.
pub fn unit_exists(unit: &str) -> bool {
    std::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            "--property=LoadState",
            "--value",
            "--",
            unit,
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && known(&o.stdout))
}

/// `systemctl show -P LoadState` output of a unit the manager knows (not `not-found`).
fn known(load_state: &[u8]) -> bool {
    let s = String::from_utf8_lossy(load_state);
    let s = s.trim();
    !s.is_empty() && s != "not-found"
}

/// `systemd-run` argv that runs `host` (the host program and its arguments) in the app's scope
/// `unit` ([`pick_unit`]) of the user's manager, under the limits of every app scope. `name` is
/// what Settings and `systemctl --user status` show.
pub fn scope_argv(unit: &str, name: &str, host: &[String]) -> Vec<String> {
    let mut a: Vec<String> = ["systemd-run", "--user", "--scope", "--quiet", "--collect"]
        .map(String::from)
        .into();
    a.push(format!("--unit={unit}"));
    a.push(format!("--description={name} (WolOS app)"));
    a.push(format!("--property=TasksMax={APP_TASKS}"));
    a.push(format!("--property=MemoryMax={APP_MEMORY}"));
    a.push(format!("--property=MemorySwapMax={APP_SWAP}"));
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
        let unit = scope_unit(&id, 4242, 0x1f);
        assert_eq!(unit, "app-wol-org.wolos.webapp.my-notes-4242-1f.scope");
        // What the saver recognises as an app scope (memsaver/procs.rs app_scope).
        let cg = format!("/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}");
        let part = cg.rsplit('/').next().unwrap();
        assert!(part.starts_with("app-wol-") && part.ends_with(".scope"));
    }

    /// A reused pid: the scope an earlier app of that pid left behind is not picked again, and
    /// two launches of one pid get two names.
    #[test]
    fn a_reused_pid_does_not_collide_with_a_scope_left_behind() {
        let id = "org.wolos.webapp.notes";
        let first = pick_unit(id, 7, |_| false);
        assert!(
            first.starts_with("app-wol-org.wolos.webapp.notes-7-"),
            "{first}"
        );
        assert!(first.ends_with(".scope"));
        assert_ne!(
            pick_unit(id, 7, |_| false),
            first,
            "a second launch, same pid"
        );
        let left = std::cell::RefCell::new(vec![first.clone()]);
        let taken = |u: &str| {
            let mut l = left.borrow_mut();
            let hit = l.len() < 3;
            l.push(u.to_owned());
            hit
        };
        let picked = pick_unit(id, 7, taken);
        let asked = left.borrow();
        assert_eq!(
            asked.len(),
            4,
            "two names existed, the third is free: {asked:?}"
        );
        assert_eq!(&picked, asked.last().unwrap());
        assert_ne!(picked, asked[1]);
        assert_ne!(picked, asked[2]);
        // Every name exists: the tries end and the next name is used.
        let mut asked = Vec::new();
        let all = pick_unit(id, 7, |u| {
            asked.push(u.to_owned());
            true
        });
        assert_eq!(asked.len(), usize::try_from(UNIT_TRIES).unwrap());
        assert!(!asked.contains(&all), "{all}");
        assert!(all.starts_with("app-wol-org.wolos.webapp.notes-7-"));
        assert!(known(b"loaded\n") && known(b"masked"));
        assert!(!known(b"not-found\n") && !known(b""));
    }

    #[test]
    fn the_host_runs_in_its_scope_with_its_arguments_untouched() {
        let host = vec![
            "/usr/bin/wol-webhost".to_string(),
            "/a b/notes.toml".to_string(),
        ];
        let unit = scope_unit("org.wolos.webapp.notes", 7, 0);
        let a = scope_argv(&unit, "Notes", &host);
        assert_eq!(
            a[..5],
            ["systemd-run", "--user", "--scope", "--quiet", "--collect"]
        );
        assert!(a.contains(&"--unit=app-wol-org.wolos.webapp.notes-7-0.scope".to_string()));
        assert!(a.contains(&"--description=Notes (WolOS app)".to_string()));
        assert!(a.contains(&"--property=TasksMax=8192".to_string()));
        // ADR 0027, as every app scope: 80 % of RAM, 25 % of swap.
        assert!(a.contains(&"--property=MemoryMax=80%".to_string()));
        assert!(a.contains(&"--property=MemorySwapMax=25%".to_string()));
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
