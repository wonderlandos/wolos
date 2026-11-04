// SPDX-License-Identifier: GPL-2.0-or-later
//! Unit tests of controller run, split out of the parent file (`#[cfg(test)] mod tests;`).
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use wol_core::system::KILLED;

/// QA-STORE-1 review: a pacman a signal ended is not "nothing pending", and a hung one does
/// not hang the check.
#[test]
fn a_killed_or_hung_pacman_is_not_nothing_pending() {
    let sh = |script: &'static str, t| run_status(&["sh", "-c", script], Duration::from_secs(t));
    assert_eq!(sh("exit 1", 5), Some((1, String::new(), String::new())));
    assert_eq!(sh("echo oops >&2; exit 2", 5).unwrap().0, 2);
    let killed = sh("kill -9 $$", 5).expect("ran");
    assert_eq!(killed.0, KILLED, "a signal has no exit code");
    let started = std::time::Instant::now();
    assert_eq!(
        sh("sleep 30", 1),
        None,
        "past the timeout: killed, no answer"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-run-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Audit S4-10: `foo.conf` and `foo.key` written at once each end with their own text (no
/// shared `foo.wol-tmp`), a 0600 file stays 0600, a new file is 0644, no temporary is left.
#[test]
fn write_file_is_atomic_per_file_and_keeps_the_mode() {
    let d = tmp("write");
    let (conf, key) = (d.join("foo.conf"), d.join("foo.key"));
    std::fs::write(&key, "old").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let threads: Vec<_> = [(conf.clone(), "conf"), (key.clone(), "key")]
        .into_iter()
        .map(|(p, text)| {
            std::thread::spawn(move || {
                for i in 0..100 {
                    RealEnv.write_file(&p, &format!("{text} {i}\n")).unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(std::fs::read_to_string(&conf).unwrap(), "conf 99\n");
    assert_eq!(std::fs::read_to_string(&key).unwrap(), "key 99\n");
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&key), 0o600);
    assert_eq!(mode(&conf), 0o644);
    RealEnv.write_private(&d.join("secret"), "s").unwrap();
    assert_eq!(mode(&d.join("secret")), 0o600);
    let left: Vec<_> = std::fs::read_dir(&d)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(shared::atomic::TMP_SUFFIX))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    std::fs::remove_dir_all(d).unwrap();
}

fn admin_req(action: AdminAction, args: &[&str]) -> AdminRequest {
    AdminRequest {
        action,
        args: args.iter().map(|s| (*s).to_string()).collect(),
    }
}

/// A theme change tells libwol apps: one `publish` of the `theme` event with the new theme's id
/// on wold's socket, for `wol theme light|dark|set` and for appearance (verbs.rs `set_theme`).
/// No wold is not an error: the theme is still written.
#[test]
fn theme_changes_publish_the_theme_event_on_wold() {
    let run = |line: &str, e: &mut FakeEnv| {
        let argv: Vec<&str> = line.split_whitespace().collect();
        execute(&crate::cli::parse(&argv).unwrap(), e)
    };
    let published = |e: &FakeEnv| -> Vec<String> {
        e.requests
            .iter()
            .filter(|(p, _)| p == Path::new("/w/wold.sock"))
            .map(|(_, r)| String::from_utf8_lossy(r).into_owned())
            .collect()
    };
    let mut e = FakeEnv {
        home: tmp("theme-event"),
        vars: vec![("WOL_JOBS_SOCKET".into(), "/w/wold.sock".into())],
        replies: vec![String::new(), String::new()],
        ..Default::default()
    };
    assert_eq!(run("theme dark", &mut e), 0, "{}", e.err);
    assert_eq!(run("theme light", &mut e), 0, "{}", e.err);
    assert_eq!(
        published(&e),
        [
            "publish {\"event\":\"theme\",\"id\":\"wol-dark\"}\n",
            "publish {\"event\":\"theme\",\"id\":\"wol-light\"}\n"
        ]
    );
    // What wold reads back is the event a subscriber gets (shared::jobs / shared::events).
    let line = published(&e)[0].clone();
    match JobRequest::parse(line.trim_end()) {
        Some(JobRequest::Publish(j)) => assert_eq!(
            shared::events::Event::parse(&j),
            Some(shared::events::Event::Theme {
                id: "wol-dark".into()
            })
        ),
        other => panic!("{other:?}"),
    }
    // wold not running (no reply): still written, still 0.
    e.requests.clear();
    assert_eq!(run("theme dark", &mut e), 0, "{}", e.err);
    assert_eq!(published(&e).len(), 1);
    // Appearance goes through the same announcement.
    e.requests.clear();
    assert_eq!(run("appearance set light", &mut e), 0, "{}", e.err);
    assert_eq!(published(&e).len(), 1, "{:?}", e.requests);
    assert!(published(&e)[0].starts_with("publish {\"event\":\"theme\""));
}

/// Audit S4-4: a package request waits as long as wol-admind may run pacman; a quick one
/// keeps a short wait; a reply that does not come in time is not "service unavailable".
#[test]
fn admin_requests_wait_per_action() {
    let mut e = FakeEnv {
        replies: vec!["ok\n".into()],
        ..Default::default()
    };
    assert_eq!(
        admin_send(&mut e, &admin_req(AdminAction::PkgInstall, &["gimp"])),
        0
    );
    assert!(e.waits[0] >= 1800, "{:?}", e.waits);
    let mut e = FakeEnv {
        replies: vec!["ok\n".into()],
        ..Default::default()
    };
    assert_eq!(admin_send(&mut e, &admin_req(AdminAction::Revoke, &[])), 0);
    assert!(e.waits[0] < 1800, "{:?}", e.waits);
}

/// A request wol-admind is still running: one request, "still working", never unavailable.
#[test]
fn a_slow_reply_is_not_service_unavailable() {
    struct Slow(FakeEnv);
    impl Env for Slow {
        fn out(&mut self, s: &str) {
            self.0.out(s)
        }
        fn err(&mut self, s: &str) {
            self.0.err(s)
        }
        fn stdout_tty(&self) -> bool {
            false
        }
        fn var(&self, n: &str) -> Option<String> {
            self.0.var(n)
        }
        fn is_root(&self) -> bool {
            false
        }
        fn request(&mut self, s: &Path, r: &[u8]) -> io::Result<String> {
            self.0.requests.push((s.into(), r.into()));
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        }
        fn exec(&mut self, a: &[String]) -> io::Result<i32> {
            self.0.exec(a)
        }
        fn capture(&mut self, a: &[&str]) -> Option<String> {
            self.0.capture(a)
        }
        fn stdin_line(&mut self) -> Option<String> {
            self.0.stdin_line()
        }
        fn read_file(&self, p: &Path) -> Option<String> {
            self.0.read_file(p)
        }
        fn sys_topic(&mut self, t: &str) -> Option<serde_json::Value> {
            self.0.sys_topic(t)
        }
        fn now_secs(&self) -> u64 {
            0
        }
        fn config_home(&self) -> PathBuf {
            PathBuf::new()
        }
    }
    let mut e = Slow(FakeEnv::default());
    let code = admin_send(&mut e, &admin_req(AdminAction::PkgInstall, &["gimp"]));
    assert_eq!(code, ErrorCode::Failed.exit());
    assert!(e.0.err.contains("still working"), "{}", e.0.err);
    assert!(!e.0.err.contains("unavailable"), "{}", e.0.err);
    assert_eq!(e.0.requests.len(), 1, "never sent twice");
}

/// Audit S4-18: three wrong passwords are three requests with a password; a fourth line on
/// stdin is never read.
#[test]
fn admin_reads_no_fourth_password() {
    let mut e = FakeEnv {
        replies: ["auth", "denied", "denied", "denied"]
            .map(|r| format!("{r}\n"))
            .to_vec(),
        stdin: ["a1", "b2", "c3", "d4"].map(String::from).to_vec(),
        ..Default::default()
    };
    let code = admin_send(&mut e, &admin_req(AdminAction::PkgInstall, &["gimp"]));
    assert_eq!(code, ErrorCode::Denied.exit());
    assert_eq!(
        e.requests.len(),
        4,
        "the first asks, three carry a password"
    );
    assert_eq!(e.stdin, ["d4"], "the fourth line stays unread");
}

/// `wol shell call|status` (SP-6): one line out, the reply line printed, ok=false exits 1.
#[test]
fn shell_call_and_status_print_the_reply_line() {
    let ask = |argv: &[&str], reply: &str, tty: bool| {
        let mut env = FakeEnv {
            vars: vec![("XDG_RUNTIME_DIR".into(), "/run/u".into())],
            replies: vec![reply.into()],
            tty,
            ..Default::default()
        };
        let code = execute(&crate::cli::parse(argv).unwrap(), &mut env);
        let sent = env
            .requests
            .first()
            .map(|r| String::from_utf8(r.1.clone()).unwrap())
            .unwrap_or_default();
        (code, sent, env.out, env.err)
    };
    let ok = "{\"ok\":true,\"value\":{\"volume\":40}}\n";
    let (code, sent, out, _) = ask(&["shell", "call", "alice.sound", "volume", "40"], ok, true);
    assert_eq!((code, sent.as_str()), (0, "call alice.sound volume 40\n"));
    assert_eq!(out, ok);
    // Words after the function are one JSON value, sent on one line.
    let (_, sent, _, _) = ask(
        &["shell", "call", "alice.sound", "set", "{\"level\":", "40}"],
        ok,
        false,
    );
    assert_eq!(sent, "call alice.sound set {\"level\":40}\n");
    let (_, sent, _, _) = ask(&["shell", "call", "alice.sound", "mute"], ok, false);
    assert_eq!(sent, "call alice.sound mute null\n");
    let no = "{\"ok\":false,\"error\":\"Volume is 0 to 100.\"}\n";
    let (code, _, out, _) = ask(
        &["shell", "call", "alice.sound", "volume", "300"],
        no,
        false,
    );
    assert_eq!((code, out.as_str()), (ErrorCode::Failed.exit(), no));
    let (code, sent, _, err) = ask(
        &["shell", "call", "alice.sound", "volume", "{nope"],
        ok,
        false,
    );
    assert_eq!((code, sent.as_str()), (ErrorCode::Usage.exit(), ""));
    assert!(err.contains("one JSON value"), "{err}");
    let (code, _, _, err) = ask(&["shell", "call", "alice.sound", "mute"], "", false);
    assert_eq!(
        code,
        ErrorCode::Unavailable.exit(),
        "an old shell answers nothing"
    );
    assert!(err.contains("did not answer"), "{err}");
    let st = concat!(
        "{\"ok\":true,\"panels\":[{\"name\":\"session\",\"open\":true}],",
        "\"surfaces\":[{\"namespace\":\"wol-dock\",\"visible\":false}],\"stopped\":[\"clipboard\"]}\n"
    );
    let (code, sent, out, _) = ask(&["shell", "status"], st, false);
    assert_eq!((code, sent.as_str(), out.as_str()), (0, "status\n", st));
    let (_, _, out, _) = ask(&["shell", "status"], st, true);
    assert_eq!(
        out,
        format!(
            "panel    {:<24} open\nsurface  {:<24} hidden\nstopped  clipboard\n",
            "session", "wol-dock"
        )
    );
    let (_, _, out, _) = ask(&["shell", "status", "--json"], st, true);
    assert_eq!(out, st);
}
