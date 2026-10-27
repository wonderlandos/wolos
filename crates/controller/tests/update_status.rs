//! `wol update status`: what the daily check found (wol_core::update::check), for any account.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::path::PathBuf;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

fn file(name: &str, text: &str) -> (PathBuf, String) {
    (
        PathBuf::from(wol_core::update::check::DIR).join(name),
        text.into(),
    )
}

#[test]
fn status_reports_the_last_check() {
    let mut env = FakeEnv {
        files: vec![
            file("checked", "1761600000\n"),
            file("pkg", "curl-8.1 <   needs updating (remote has 8.2)\n"),
            file("audit", "curl-8.1\n"),
            file(
                "base",
                "The following files will be updated as part of updating to 15.1-RELEASE-p3:\n",
            ),
        ],
        ..FakeEnv::default()
    };
    assert_eq!(run("update status --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["checked"], 1761600000);
    assert_eq!(v["packages"], serde_json::json!(["curl"]));
    assert_eq!(v["security"], serde_json::json!(["curl"]));
    assert_eq!(v["base"], "15.1-RELEASE-p3");
    assert_eq!(v["count"], 2);

    let mut env = FakeEnv {
        tty: true,
        files: env.files.clone(),
        ..FakeEnv::default()
    };
    assert_eq!(run("update status", &mut env), 0);
    assert!(env.out.contains("curl (security fix)"), "{}", env.out);
    assert!(env.out.contains("FreeBSD 15.1-RELEASE-p3"), "{}", env.out);

    let mut never = FakeEnv::default();
    assert_eq!(run("update status --json", &mut never), 0);
    assert_eq!(never.out.trim(), "null", "never checked");
}

#[test]
fn check_lists_fetched_freebsd_fixes_until_installed() {
    let base = file("base", "updating to 15.1-RELEASE-p3:\n");
    let checked = file("checked", "5\n");
    for (userland, want) in [
        ("15.1-RELEASE-p2\n", r#"["curl","FreeBSD 15.1-RELEASE-p3"]"#),
        ("15.1-RELEASE-p3\n", r#"["curl"]"#),
    ] {
        let mut env = FakeEnv {
            files: vec![base.clone(), checked.clone()],
            captures: vec![
                (
                    "pkg version -U -vRL=".into(),
                    "curl-8.1 <   needs updating\n".into(),
                ),
                ("freebsd-version -u".into(), userland.into()),
            ],
            ..FakeEnv::default()
        };
        assert_eq!(run("update check --json", &mut env), 0);
        assert_eq!(env.out.trim(), want, "{userland}");
    }
}

#[test]
fn check_never_says_up_to_date_without_refreshing_the_catalogue() {
    let none = || FakeEnv {
        captures: vec![("pkg version -U -vRL=".into(), "".into())],
        ..FakeEnv::default()
    };
    // wol-admind unreachable: the (possibly stale) catalogue proves nothing.
    let mut env = none();
    assert_ne!(run("update check --json", &mut env), 0);
    assert!(!env.out.contains("[]"), "{}", env.out);
    // Refreshed first: an empty list is trustworthy.
    let mut env = FakeEnv {
        replies: vec!["ok\n".into()],
        ..none()
    };
    assert_eq!(run("update check --json", &mut env), 0);
    assert_eq!(env.out.trim(), "[]");
    let sent = &env.requests[0].1;
    assert!(String::from_utf8_lossy(sent).contains("pkg-refresh"));
}
