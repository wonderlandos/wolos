// SPDX-License-Identifier: GPL-2.0-or-later
//! The installer's secret files (audit3 R2B-6): `wol encrypt decide` checks, and `wol encrypt
//! enroll` adds to the disk, the same reading of a password file (`encrypt::typed_secret`: one
//! line end off, text only), so a Windows-written `pass\r\n` is checked and enrolled as `pass`.
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::path::PathBuf;

fn run(line: &str, env: &mut FakeEnv) -> i32 {
    let argv: Vec<&str> = line.split_whitespace().collect();
    execute(&parse(&argv).unwrap(), env)
}

#[test]
fn enroll_adds_what_was_typed_from_a_file_beside_the_key() {
    let pass = PathBuf::from("/plan/disk-password");
    let mut env = FakeEnv {
        vars: vec![("WOLOS_RECOVERY_OUT".into(), "/results/recovery-key".into())],
        files: vec![(pass.clone(), "white rabbit\r\n".into())],
        captures: vec![(
            "systemd-cryptenroll --recovery-key --unlock-key-file=/keys/key /dev/p2".into(),
            "abcd-efgh\n".into(),
        )],
        ..Default::default()
    };
    assert_eq!(
        run(
            "encrypt enroll /dev/p2 /keys/key passphrase /plan/disk-password",
            &mut env
        ),
        0,
        "{}",
        env.err
    );
    assert!(
        env.written
            .iter()
            .any(|(p, t)| p == &PathBuf::from("/keys/typed") && t == "white rabbit"),
        "{:?}",
        env.written
    );
    assert!(
        env.written
            .iter()
            .any(|(p, t)| p == &PathBuf::from("/results/recovery-key") && t == "abcd-efgh\n"),
        "{:?}",
        env.written
    );
    let add = env
        .execs
        .iter()
        .map(|e| e.join(" "))
        .find(|e| e.contains("luksAddKey"))
        .unwrap();
    assert!(add.ends_with("/dev/p2 /keys/typed"), "{add}");
    assert!(!add.contains("rabbit"), "never an argument: {add}");
}

#[test]
fn decide_reads_the_passphrase_the_way_enroll_does() {
    let file = |name: &str, bytes: &[u8]| {
        let p = std::env::temp_dir().join(format!("wol-secret-{name}-{}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    };
    let decide = |path: String| {
        let mut env = FakeEnv {
            vars: vec![("WOLOS_PASSPHRASE_FILE".into(), path)],
            ..Default::default()
        };
        let rc = run("encrypt decide passphrase", &mut env);
        (rc, env.out, env.err)
    };
    let (rc, out, _) = decide(file("eight", b"12345678\r\n"));
    assert_eq!((rc, out.as_str()), (0, "passphrase\n"));
    // Seven typed characters and a CR LF: seven, as enrolled.
    let (rc, _, err) = decide(file("seven", b"1234567\r\n"));
    assert_ne!(rc, 0);
    assert!(err.contains("8 characters"), "{err}");
    // Not text: said as such, not "a passphrase is needed".
    let (rc, _, err) = decide(file("binary", b"\xff\xfe12345678"));
    assert_ne!(rc, 0);
    assert!(err.contains("not typed text"), "{err}");
}

#[test]
fn the_account_password_is_read_the_same_way() {
    use controller::install::plan_problem;
    use wol_core::install::plan::{Account, Plan, Protect};
    let p = Plan {
        locale: "en_US.UTF-8".into(),
        keyboard: "us".into(),
        zone: "Europe/Berlin".into(),
        protect: Protect::Tpm,
        account: Some(Account {
            name: "alice".into(),
            ..Account::default()
        }),
        ..Plan::default()
    };
    assert_eq!(plan_problem(&p, "", Some("curiouser\r\n")), None);
    assert_eq!(
        plan_problem(&p, "", Some("two\nlines\n")),
        Some("WeakPassword")
    );
}
