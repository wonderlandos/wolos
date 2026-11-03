// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol sessions save|restore|list [--json]` (ADR 0032 "Sessions", docs/modules/modules.md): the
//! terminal and agent windows of this session, saved to `$XDG_STATE_HOME/wolos/sessions.json` and
//! reopened at the next login in their workspace and directory. The model and every check are in
//! `wol_core::sessions`; this file reads the compositor (`hyprctl -j clients`) and `/proc`, writes the
//! file (temp + rename) and hands each window to `hyprctl dispatch exec`.
//!
//! Invariant: runs as the person, touches only their state file and their windows; what it reads
//! back is rebuilt into argv by `wol_core::sessions::hypr_exec`, never run as written.
use crate::cli::Invocation;
use crate::run::{Env, emit_json, fail};
use shared::errors::ErrorCode;
use std::path::{Path, PathBuf};
use wol_core::sessions::{self, Proc, Saved};

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json");
    match inv.action {
        "sessions.save" => save(env, json, inv.flag("yes")),
        "sessions.restore" => restore(env, json),
        "sessions.list" => list(env, json),
        other => fail(
            env,
            ErrorCode::Usage,
            &format!("{other} is not a sessions command"),
        ),
    }
}

fn file(env: &dyn Env) -> PathBuf {
    env.state_home().join(sessions::FILE)
}

/// `/proc/<pid>`: argv, directory (`readlink`), children. None for a process that is gone.
fn proc_of(env: &mut dyn Env, pid: u32) -> Option<Proc> {
    let dir = format!("/proc/{pid}");
    let raw = env.read_file(Path::new(&format!("{dir}/cmdline")))?;
    let children = env
        .read_file(Path::new(&format!("{dir}/task/{pid}/children")))
        .map(|c| sessions::parse_children(&c))
        .unwrap_or_default();
    let cwd = env
        .capture(&["readlink", "--", &format!("{dir}/cwd")])
        .map(|c| c.trim_end_matches('\n').to_string())
        .unwrap_or_default();
    Some(Proc {
        argv: sessions::parse_cmdline(&raw),
        cwd,
        children,
    })
}

/// Read every process the windows need once, then let the core walk them.
fn gather(env: &mut dyn Env) -> Option<Vec<sessions::Entry>> {
    let windows = sessions::parse_clients(&env.capture(&["hyprctl", "-j", "clients"])?);
    // Terminal -> shell -> program -> its child: at most MAX_DEPTH levels below each window.
    let mut seen: Vec<(u32, Proc)> = Vec::new();
    let mut level: Vec<u32> = windows
        .iter()
        .filter(|w| sessions::TERMINALS.contains(&w.class.as_str()))
        .filter_map(|w| u32::try_from(w.pid).ok())
        .collect();
    for _ in 0..=sessions::MAX_DEPTH {
        let mut next = Vec::new();
        for pid in level {
            if pid == 0 || seen.iter().any(|s| s.0 == pid) {
                continue;
            }
            if let Some(p) = proc_of(env, pid) {
                next.extend(p.children.iter().copied());
                seen.push((pid, p));
            }
        }
        if next.is_empty() || seen.len() > sessions::MAX_WINDOWS * 8 {
            break;
        }
        level = next;
    }
    let lookup = |pid: u32| seen.iter().find(|s| s.0 == pid).map(|s| s.1.clone());
    Some(sessions::collect(&windows, &lookup))
}

fn read_saved(env: &dyn Env) -> Saved {
    env.read_file(&file(env))
        .map(|t| sessions::parse(&t))
        .unwrap_or_default()
}

/// Save the open windows. An empty list never replaces a non-empty saved set unless `clear`
/// (`--yes`): a desktop that is still starting, or one whose clients the compositor is closing at logout,
/// answers with no windows, and that must not wipe the set the next login offers.
fn save(env: &mut dyn Env, json: bool, clear: bool) -> i32 {
    let Some(windows) = gather(env) else {
        return fail(
            env,
            ErrorCode::Unavailable,
            "no desktop session to save (the compositor did not answer)",
        );
    };
    let path = file(env);
    if windows.is_empty() && !clear {
        let kept = read_saved(env).windows.len();
        if kept > 0 {
            if json {
                emit_json(env, &serde_json::json!({"saved": false, "kept": kept}));
            } else {
                env.out(&format!(
                    "no windows open: kept the {kept} saved window(s) (wol sessions save --yes clears them)\n"
                ));
            }
            return 0;
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let text = sessions::to_json(&windows, env.now_secs());
    if let Err(e) = env.write_file(&path, &text) {
        return fail(
            env,
            ErrorCode::Failed,
            &format!("could not save the window set: {e}"),
        );
    }
    if json {
        env.out(&text);
    } else {
        env.out(&format!("saved {} window(s)\n", windows.len()));
    }
    0
}

fn list(env: &mut dyn Env, json: bool) -> i32 {
    let s = read_saved(env);
    if json {
        emit_json(env, &serde_json::to_value(&s).unwrap_or_default());
        return 0;
    }
    if s.windows.is_empty() {
        env.out("no saved windows\n");
        return 0;
    }
    let mut t = String::new();
    for e in &s.windows {
        let what = if e.command.is_empty() {
            "shell"
        } else {
            e.command.as_str()
        };
        t.push_str(&format!("{:>3}  {:<20}  {}\n", e.workspace, what, e.cwd));
    }
    env.out(&t);
    0
}

fn restore(env: &mut dyn Env, json: bool) -> i32 {
    let s = read_saved(env);
    let mut opened = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for e in &s.windows {
        let Some(cmd) = sessions::hypr_exec(e) else {
            continue;
        };
        let argv = ["hyprctl".to_string(), "dispatch".into(), "exec".into(), cmd];
        match env.exec_quiet(&argv) {
            Ok(0) => opened += 1,
            _ => failed.push(e.cwd.clone()),
        }
    }
    if json {
        emit_json(
            env,
            &serde_json::json!({"opened": opened, "failed": failed, "saved_at": s.saved_at}),
        );
    } else {
        env.out(&format!("reopened {opened} window(s)\n"));
    }
    if failed.is_empty() {
        0
    } else {
        for f in &failed {
            env.err(&format!("wol: {f}: the window did not reopen\n"));
        }
        ErrorCode::Failed.exit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::FakeEnv;

    fn inv(action: &'static str, json: bool) -> Invocation {
        Invocation {
            path: Vec::new(),
            action,
            args: Vec::new(),
            flags: if json {
                vec!["json".into()]
            } else {
                Vec::new()
            },
        }
    }

    fn fake(home: &Path) -> FakeEnv {
        let mut env = FakeEnv {
            home: home.into(),
            ..Default::default()
        };
        env.captures.push((
            "hyprctl -j clients".into(),
            r#"[{"class":"foot","title":"site","workspace":{"id":2},"pid":10},
                {"class":"firefox","title":"web","workspace":{"id":1},"pid":20}]"#
                .into(),
        ));
        env.files.push(("/proc/10/cmdline".into(), "foot\0".into()));
        env.files
            .push(("/proc/10/task/10/children".into(), "11".into()));
        env.files.push(("/proc/11/cmdline".into(), "-zsh\0".into()));
        env.files
            .push(("/proc/11/task/11/children".into(), "12 ".into()));
        env.files.push((
            "/proc/12/cmdline".into(),
            "/home/a/.local/bin/claude\0".into(),
        ));
        for (pid, cwd) in [(10, "/home/a"), (11, "/home/a/Work/site"), (12, "/x")] {
            env.captures
                .push((format!("readlink -- /proc/{pid}/cwd"), format!("{cwd}\n")));
        }
        env
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wol-sessions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_writes_the_terminal_and_agent_windows() {
        let home = tmp("save");
        let mut env = fake(&home);
        assert_eq!(
            execute(&inv("sessions.save", false), &mut env),
            0,
            "{}",
            env.err
        );
        assert_eq!(env.out, "saved 1 window(s)\n");
        let (path, text) = env.written.last().unwrap().clone();
        assert_eq!(path, home.join("state/wolos/sessions.json"));
        let s = sessions::parse(&text);
        assert_eq!(s.saved_at, 1000);
        assert_eq!(s.windows.len(), 1);
        let w = &s.windows[0];
        assert_eq!((w.workspace, w.cwd.as_str()), (2, "/home/a/Work/site"));
        assert_eq!(w.command, "claude --continue");
    }

    #[test]
    fn saving_no_windows_keeps_a_saved_set_unless_cleared() {
        let home = tmp("keep");
        let mut env = fake(&home);
        env.captures.retain(|c| c.0 != "hyprctl -j clients");
        env.captures
            .push(("hyprctl -j clients".into(), "[]".into()));
        let one = r#"{"workspace":1,"cwd":"/h","kind":"terminal","command":"","title":"t"}"#;
        env.files.push((
            home.join("state/wolos/sessions.json"),
            format!(r#"{{"version":1,"saved_at":7,"windows":[{one},{one},{one}]}}"#),
        ));
        assert_eq!(
            execute(&inv("sessions.save", false), &mut env),
            0,
            "{}",
            env.err
        );
        assert!(env.written.is_empty(), "{:?}", env.written);
        assert!(env.out.contains("kept the 3 saved"), "{}", env.out);
        let mut clear = inv("sessions.save", false);
        clear.flags.push("yes".into());
        assert_eq!(execute(&clear, &mut env), 0, "{}", env.err);
        let (_, text) = env.written.last().unwrap().clone();
        assert!(sessions::parse(&text).windows.is_empty());
    }

    #[test]
    fn save_without_hyprland_says_so() {
        let mut env = FakeEnv::default();
        assert_ne!(execute(&inv("sessions.save", false), &mut env), 0);
        assert!(env.err.contains("no desktop session"), "{}", env.err);
        assert!(env.written.is_empty());
    }

    #[test]
    fn restore_and_list_read_the_file_and_refuse_what_is_not_theirs() {
        let home = tmp("restore");
        let mut env = fake(&home);
        let good = r#"{"workspace":2,"cwd":"/home/a/Work/site","kind":"agent","command":"claude --continue","title":"site","terminal":"foot"}"#;
        let evil = r#"{"workspace":1,"cwd":"/home/a","kind":"terminal","command":"sh -c reboot","title":"x"}"#;
        env.files.push((
            home.join("state/wolos/sessions.json"),
            format!(r#"{{"version":1,"saved_at":7,"windows":[{good},{evil}]}}"#),
        ));
        assert_eq!(
            execute(&inv("sessions.restore", false), &mut env),
            0,
            "{}",
            env.err
        );
        assert_eq!(env.execs.len(), 1, "{:?}", env.execs);
        assert_eq!(
            env.execs[0],
            [
                "hyprctl",
                "dispatch",
                "exec",
                "[workspace 2 silent] 'foot' '--working-directory' '/home/a/Work/site' 'claude' '--continue'"
            ]
        );
        assert_eq!(env.out, "reopened 1 window(s)\n");
        env.out.clear();
        assert_eq!(execute(&inv("sessions.list", true), &mut env), 0);
        let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
        assert_eq!(v["saved_at"], 7);
        assert_eq!(v["windows"].as_array().unwrap().len(), 1);
        env.out.clear();
        assert_eq!(execute(&inv("sessions.list", false), &mut env), 0);
        assert!(env.out.contains("claude --continue"), "{}", env.out);
    }

    #[test]
    fn a_window_that_does_not_reopen_is_reported() {
        let home = tmp("fail");
        let mut env = fake(&home);
        env.files.push((
            home.join("state/wolos/sessions.json"),
            r#"{"version":1,"windows":[{"workspace":1,"cwd":"/h","kind":"terminal","command":"","title":""}]}"#.into(),
        ));
        env.exec_codes.push(1);
        assert_ne!(execute(&inv("sessions.restore", true), &mut env), 0);
        assert!(
            env.err.contains("/h: the window did not reopen"),
            "{}",
            env.err
        );
        let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
        assert_eq!(v["opened"], 0);
    }

    #[test]
    fn nothing_saved_lists_and_restores_nothing() {
        let home = tmp("empty");
        let mut env = fake(&home);
        assert_eq!(execute(&inv("sessions.list", false), &mut env), 0);
        assert_eq!(env.out, "no saved windows\n");
        env.out.clear();
        assert_eq!(execute(&inv("sessions.restore", false), &mut env), 0);
        assert!(env.execs.is_empty());
    }
}
