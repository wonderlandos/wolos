// SPDX-License-Identifier: GPL-2.0-or-later
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
    files.push(file("pkg", "wol-shell 0.12.1-1 -> 0.12.2-1\n"));
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
            captures: vec![("cat /usr/lib/wolos-release.json".into(), running.clone())],
            ..FakeEnv::default()
        };
        assert_eq!(run("update check --json", &mut env), 0);
        assert_eq!(env.out.trim(), want, "{running}");
    }
}

#[test]
fn check_never_says_up_to_date_without_refreshing_the_catalogue() {
    // FakeEnv's clock reads 1000: a check stamped 1000 ran for this request, 5 long before.
    let none = |checked: &str| FakeEnv {
        files: vec![file("checked", checked), file("pkg", "")],
        ..FakeEnv::default()
    };
    // wol-admind unreachable: the (possibly stale) findings prove nothing.
    let mut env = none("5\n");
    assert_ne!(run("update check --json", &mut env), 0);
    assert!(!env.out.contains("[]"), "{}", env.out);
    // wol-admind ran the check but it could not refresh (offline): the stamp stayed old.
    let mut env = FakeEnv {
        replies: vec!["ok\n".into()],
        ..none("5\n")
    };
    assert_ne!(run("update check --json", &mut env), 0);
    // Checked now: an empty list is trustworthy.
    let mut env = FakeEnv {
        replies: vec!["ok\n".into()],
        ..none("1000\n")
    };
    assert_eq!(run("update check --json", &mut env), 0);
    assert_eq!(env.out.trim(), "[]");
    let sent = &env.requests[0].1;
    assert!(String::from_utf8_lossy(sent).contains("pkg-refresh"));
}

/// e2e D3: `pacman -Qu` on an installed slot reads the sealed, never refreshed database and exits
/// 1 when nothing is pending; the check never runs it, whatever it would say.
#[test]
fn check_reads_the_findings_and_never_runs_pacman() {
    let mut env = FakeEnv {
        files: vec![
            file("checked", "1000\n"),
            file("pkg", "wolos-system 0.12.0-1 -> 0.12.1-1\n"),
        ],
        replies: vec!["ok\n".into()],
        // Whatever a command would print: a `pacman -Qu` run would add this package.
        captured: Some("stale 1-1 -> 2-1\n".into()),
        ..FakeEnv::default()
    };
    assert_eq!(run("update check --json", &mut env), 0);
    assert_eq!(env.out.trim(), r#"["wolos-system"]"#);
    assert!(
        env.execs.iter().all(|a| a[0] != "pacman"),
        "{:?}",
        env.execs
    );
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
    assert_eq!(
        (v["findings_total"].as_u64(), v["truncated"].as_bool()),
        (Some(2), Some(false))
    );
    assert_eq!(
        (v["changed"].as_u64(), v["missing"].as_u64()),
        (Some(1), Some(1))
    );
    // QA-LIVE3-25: a layer's worth of missing files is a summary, not a megabyte.
    let big: String = (0..3000)
        .map(|i| format!("{abc}  /usr/lib/layer/f{i}\n"))
        .collect();
    let mut env = FakeEnv {
        vars: vec![("WOL_ROOT".into(), root.to_string_lossy().into_owned())],
        files: vec![(
            root.join(wol_core::base::MANIFEST.trim_start_matches('/')),
            big,
        )],
        ..FakeEnv::default()
    };
    assert_eq!(run("base verify", &mut env), 1);
    assert!(env.out.len() < 60_000, "{} bytes", env.out.len());
    let v: serde_json::Value = serde_json::from_str(&env.out).unwrap();
    assert_eq!(
        (v["findings_total"].as_u64(), v["truncated"].as_bool()),
        (Some(3000), Some(true))
    );
    assert_eq!(v["findings"].as_array().unwrap().len(), 500);
    let mut text = FakeEnv {
        tty: true,
        ..FakeEnv::default()
    };
    text.vars = env.vars.clone();
    text.files = env.files.clone();
    assert_eq!(run("base verify", &mut text), 1);
    assert!(
        text.out
            .contains("3000 of 3000 base files differ: 0 changed, 3000 missing"),
        "{}",
        text.out
    );
    assert!(text.out.contains("... and 2980 more"), "{}", text.out);
    assert_eq!(text.out.lines().count(), 22, "{}", text.out);
    let mut none = FakeEnv::default();
    assert_eq!(run("base verify", &mut none), 66, "no manifest");
    let _ = std::fs::remove_dir_all(&root);
}

/// e2e D3: v0.12.0 ships no /usr/lib/wolos-release.json; once the check worked it offered
/// "WolOS 0.12.0" on 0.12.0. os-release names the running release.
#[test]
fn check_never_offers_the_running_release() {
    let (keys, mut files) = signed("running");
    files.push(file("checked", "1000\n"));
    files.push(file("pkg", ""));
    for (os, want) in [("2026.11", "[]"), ("2026.10", r#"["WolOS 2026.11"]"#)] {
        let mut f = files.clone();
        f.push((
            PathBuf::from("/etc/os-release"),
            format!("NAME=WolOS\nID=wolos\nVERSION_ID={os}\n"),
        ));
        let mut env = FakeEnv {
            files: f,
            vars: vec![keys.clone()],
            replies: vec!["ok\n".into()],
            ..FakeEnv::default()
        };
        assert_eq!(run("update check --json", &mut env), 0, "{}", env.err);
        assert_eq!(env.out.trim(), want, "running {os}");
    }
}

/// QA-LIVE4-14: an update cut off by a power loss is said, not hidden: its mark is there and no
/// build holds the lock. While a build runs (the lock held), or with no mark, nothing is said.
#[test]
fn an_interrupted_update_is_reported() {
    let mark = (
        PathBuf::from(wol_core::update::BUILDING),
        "wolos-b 2026/11/02\n".to_owned(),
    );
    let mut env = FakeEnv {
        files: vec![mark.clone(), file("checked", "5\n")],
        ..FakeEnv::default()
    };
    assert_eq!(run("update status --json", &mut env), 0);
    let v: serde_json::Value = serde_json::from_str(env.out.trim()).unwrap();
    assert_eq!(v["interrupted"], "wolos-b", "{}", env.out);
    let busy = FakeEnv {
        files: vec![mark, file("checked", "5\n")],
        ..FakeEnv::default()
    };
    let _held = {
        // The build is root's; the person asking below is not (review 4: the status asks
        // /proc/locks, it never takes the lock, which a person could not anyway).
        let mut b = FakeEnv {
            root: true,
            locks: std::sync::Arc::clone(&busy.locks),
            ..FakeEnv::default()
        };
        use controller::run::Env;
        b.try_lock(std::path::Path::new(wol_core::update::LOCK))
            .unwrap()
            .unwrap()
    };
    let mut busy = busy;
    assert_eq!(run("update status --json", &mut busy), 0);
    let v: serde_json::Value = serde_json::from_str(busy.out.trim()).unwrap();
    assert!(
        v["interrupted"].is_null(),
        "a build is running: {}",
        busy.out
    );
    let mut none = FakeEnv {
        files: vec![file("checked", "5\n")],
        ..FakeEnv::default()
    };
    assert_eq!(run("update status --json", &mut none), 0);
    let v: serde_json::Value = serde_json::from_str(none.out.trim()).unwrap();
    assert!(v["interrupted"].is_null());
}

/// QA-STORE-2 (frozen for the Store, C1): `wol update install --json` says which WolOS release
/// came with the update (the signed one the check found, newer than the running one), null when
/// none.
#[test]
fn install_json_names_the_release() {
    let (keys, mut files) = signed("install");
    files.push(file("checked", "5\n"));
    files.push(file("pkg", "wol-shell 0.12.1-1 -> 0.12.2-1\n"));
    for (running, want) in [
        (
            RELEASE.replace("2026.11", "2026.10"),
            serde_json::json!("2026.11"),
        ),
        (RELEASE.to_string(), serde_json::Value::Null),
    ] {
        let mut env = FakeEnv {
            files: files.clone(),
            vars: vec![keys.clone()],
            replies: vec!["ok\n".into()],
            captures: vec![("cat /usr/lib/wolos-release.json".into(), running.clone())],
            ..FakeEnv::default()
        };
        assert_eq!(run("update install --json", &mut env), 0, "{}", env.err);
        let v: serde_json::Value = serde_json::from_str(env.out.trim()).unwrap();
        assert_eq!(v["release"], want, "{}", env.out);
        assert_eq!(v["applied"], 1);
    }
}
