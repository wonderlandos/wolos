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

const SEED: [u8; 32] = [7; 32];
const RELEASE: &str = r#"{"version":"2026.11","snapshot":"2026/11/02","wolos_repo":"r42","kernel":"6.17.3.wolos1-1","image":"https://example.invalid/wolos.iso","sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}"#;

/// A keys folder trusting [`SEED`], and the signed release files.
fn signed(tag: &str) -> ((String, String), Vec<(PathBuf, String)>) {
    use wol_core::package::{hex, public_of, public_pem, sign};
    let d = std::env::temp_dir().join(format!("wol-upd-keys-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("wolos.pub"), public_pem(&public_of(&SEED))).unwrap();
    let sig = hex(sign(&SEED, RELEASE.as_bytes()));
    (
        ("WOL_KEYS_DIR".into(), d.to_string_lossy().into_owned()),
        vec![file("base", RELEASE), file("base.sig", &sig)],
    )
}

#[test]
fn status_reports_the_last_check() {
    let (keys, base) = signed("status");
    let mut files = vec![
        file("checked", "1761600000\n"),
        file("pkg", "wol-shell 0.12.1-1 -> 0.12.2-1\n"),
        file("audit", "wol-shell\n"),
        file("kernels", "linux-wolos\nlinux-zen\n"),
    ];
    files.extend(base);
    let mut env = FakeEnv {
        files,
        vars: vec![keys.clone()],
        ..FakeEnv::default()
    };
    assert_eq!(run("update status --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["checked"], 1761600000);
    assert_eq!(v["packages"], serde_json::json!(["wol-shell"]));
    assert_eq!(v["security"], serde_json::json!(["wol-shell"]));
    assert_eq!(v["base"], "2026.11");
    assert_eq!(v["snapshot"], "2026/11/02");
    assert_eq!(v["kernels"], serde_json::json!(["linux-zen"]));
    assert_eq!(v["count"], 2);

    let mut env = FakeEnv {
        tty: true,
        files: env.files.clone(),
        vars: vec![keys],
        ..FakeEnv::default()
    };
    assert_eq!(run("update status", &mut env), 0);
    assert!(env.out.contains("wol-shell (security fix)"), "{}", env.out);
    assert!(env.out.contains("WolOS 2026.11"), "{}", env.out);
    assert!(
        env.out.contains("linux-zen: your own kernel, kept"),
        "{}",
        env.out
    );

    // Unsigned or untrusted manifest: no release offered.
    let mut env = FakeEnv {
        files: env.files.clone(),
        ..FakeEnv::default()
    };
    assert_eq!(run("update status --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["base"], serde_json::Value::Null);

    let mut never = FakeEnv::default();
    assert_eq!(run("update status --json", &mut never), 0);
    assert_eq!(never.out.trim(), "null", "never checked");
}

#[test]
fn check_lists_a_new_release_until_installed() {
    let (keys, mut files) = signed("check");
    files.push(file("checked", "5\n"));
    for (running, want) in [
        (
            RELEASE.replace("2026.11", "2026.10"),
            r#"["wol-shell","WolOS 2026.11"]"#,
        ),
        (RELEASE.to_string(), r#"["wol-shell"]"#),
    ] {
        let mut env = FakeEnv {
            files: files.clone(),
            vars: vec![keys.clone()],
            captures: vec![
                (
                    "pacman -Qu".into(),
                    "wol-shell 0.12.1-1 -> 0.12.2-1\n".into(),
                ),
                ("cat /usr/lib/wolos-release.json".into(), running.clone()),
            ],
            ..FakeEnv::default()
        };
        assert_eq!(run("update check --json", &mut env), 0);
        assert_eq!(env.out.trim(), want, "{running}");
    }
}

#[test]
fn check_never_says_up_to_date_without_refreshing_the_catalogue() {
    let none = || FakeEnv {
        captures: vec![("pacman -Qu".into(), "".into())],
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

#[test]
fn status_names_the_base_and_its_kernel() {
    let mut env = FakeEnv {
        tty: true,
        files: vec![
            file("checked", "5\n"),
            (
                PathBuf::from("/etc/os-release"),
                "ID=wolos\nVERSION_ID=2026.11\n".into(),
            ),
            (
                PathBuf::from("/proc/sys/kernel/osrelease"),
                "6.17.3-wolos1-1\n".into(),
            ),
        ],
        ..FakeEnv::default()
    };
    assert_eq!(run("update status", &mut env), 0);
    assert!(
        env.out
            .starts_with("base 2026.11, kernel 6.17.3-wolos1-1 (linux-wolos)\n"),
        "{}",
        env.out
    );
}

#[test]
fn base_verify_reports_changed_and_missing_files() {
    let root = std::env::temp_dir().join(format!("wol-base-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("usr/bin")).unwrap();
    std::fs::write(root.join("usr/bin/ls"), b"abc").unwrap();
    std::fs::write(root.join("usr/bin/cp"), b"tampered").unwrap();
    let abc = wol_core::package::sha256_hex(b"abc");
    let manifest = format!("{abc}  /usr/bin/ls\n{abc}  /usr/bin/cp\n{abc}  /usr/bin/gone\n");
    let mpath = root.join(wol_core::base::MANIFEST.trim_start_matches('/'));
    let mut env = FakeEnv {
        vars: vec![("WOL_ROOT".into(), root.to_string_lossy().into_owned())],
        files: vec![(mpath, manifest)],
        ..FakeEnv::default()
    };
    assert_eq!(run("base verify", &mut env), 1);
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(v["files"], 3);
    assert_eq!(
        v["findings"],
        serde_json::json!([
            {"path": "/usr/bin/cp", "state": "changed"},
            {"path": "/usr/bin/gone", "state": "missing"}
        ])
    );
    let mut none = FakeEnv::default();
    assert_eq!(run("base verify", &mut none), 66, "no manifest");
    let _ = std::fs::remove_dir_all(&root);
}
