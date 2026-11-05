// SPDX-License-Identifier: BSD-2-Clause
//! `wol-keyring` end to end in a scratch folder: init, set, get, export and import between two
//! keyrings, the audit chain, and a move between two processes over the loopback network.
use std::io::Write;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_wol-keyring");

fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wolkeyring-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(d: &std::path::Path, args: &[&str], stdin: &str) -> (bool, String, String) {
    let mut c = Command::new(BIN)
        .args(args)
        .env("WOLOS_KEYRING_DIR", d)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into(),
        String::from_utf8_lossy(&o.stderr).into(),
    )
}

#[test]
fn init_set_get_export_import_audit() {
    let a = dir("a");
    let b = dir("b");
    let (ok, out, err) = run(&a, &["init"], "first password\n");
    assert!(ok, "{err}");
    assert!(out.contains("Recovery code"), "{out}");
    let code = out
        .lines()
        .find(|l| l.starts_with("Recovery code"))
        .and_then(|l| l.rsplit(": ").next())
        .unwrap()
        .trim()
        .to_string();
    let (ok, _, err) = run(
        &a,
        &["set", "Example", "url=a.example", "user=ana"],
        "first password\nhunter2\n",
    );
    assert!(ok, "{err}");
    let (ok, out, _) = run(&a, &["get", "url=a.example"], "first password\n");
    assert!(ok);
    assert_eq!(out.trim(), "hunter2");
    // The recovery code opens it too; a wrong password does not.
    let (ok, out, _) = run(
        &a,
        &["get", "url=a.example"],
        &format!("{}\n", code.to_lowercase()),
    );
    assert!(ok && out.trim() == "hunter2");
    let (ok, _, err) = run(&a, &["get", "url=a.example"], "wrong password\n");
    assert!(!ok && err.contains("do not open"), "{err}");
    // Export to a passphrase, import into another keyring.
    assert!(run(&b, &["init"], "second password\n").0);
    let file = a.join("out.wkb");
    let f = file.to_str().unwrap();
    let (ok, out, err) = run(
        &a,
        &["export", f, "--passphrase"],
        "first password\na long export phrase\n",
    );
    assert!(ok, "{err}");
    assert!(out.contains("1 items exported"), "{out}");
    let (ok, out, err) = run(
        &b,
        &["import", f],
        "second password\na long export phrase\n",
    );
    assert!(ok, "{err}");
    assert!(out.contains("1 added"), "{out}");
    let (_, out, _) = run(&b, &["get", "url=a.example"], "second password\n");
    assert_eq!(out.trim(), "hunter2");
    // Export to b's identity.
    let (ok, idline, _) = run(&b, &["id"], "second password\n");
    assert!(ok && idline.starts_with("wolos-keyring-id/1 "));
    let idf = a.join("b.id");
    std::fs::write(&idf, &idline).unwrap();
    let f2 = a.join("to-b.wkb");
    let (ok, _, err) = run(
        &a,
        &[
            "export",
            f2.to_str().unwrap(),
            "--to",
            idf.to_str().unwrap(),
        ],
        "first password\n",
    );
    assert!(ok, "{err}");
    let (ok, out, err) = run(&b, &["import", f2.to_str().unwrap()], "second password\n");
    assert!(ok, "{err}");
    assert!(out.contains("1 kept"), "the same item, not newer: {out}");
    // a cannot open what was sealed to b.
    let (ok, _, _) = run(&a, &["import", f2.to_str().unwrap()], "first password\n");
    assert!(!ok);
    // The audit verifies; an edited line is found.
    let (ok, out, err) = run(&a, &["audit", "verify"], "first password\n");
    assert!(ok, "{err}");
    assert!(out.contains("chain intact"), "{out}");
    let log = a.join("login.audit");
    let t = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, t.replacen("\"export\"", "\"import\"", 1)).unwrap();
    let (ok, _, err) = run(&a, &["audit", "verify"], "first password\n");
    assert!(!ok, "{err}");
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

#[test]
fn move_between_two_processes() {
    let a = dir("ma");
    let b = dir("mb");
    assert!(run(&a, &["init"], "first password\n").0);
    assert!(run(&b, &["init"], "second password\n").0);
    assert!(
        run(
            &a,
            &["set", "VPN", "vpn=u1"],
            "first password\nwg-private\n"
        )
        .0
    );
    let port = 40000 + (std::process::id() % 20000) as u16;
    let bd = b.clone();
    let p = port.to_string();
    let recv = std::thread::spawn(move || {
        run(
            &bd,
            &["move", "receive", "--port", &p],
            "second password\nyes\n",
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let (ok, out, err_s) = run(
        &a,
        &["move", "send", &format!("127.0.0.1:{port}")],
        "first password\nyes\n",
    );
    assert!(ok, "{err_s}");
    assert!(out.contains("1 items sent"));
    let (ok, out, err) = recv.join().unwrap();
    let code = |s: &str| {
        s.lines()
            .find(|l| l.contains("same code:"))
            .map(|l| l.rsplit(": ").next().unwrap_or("").to_string())
    };
    assert!(code(&err).is_some());
    assert_eq!(
        code(&err),
        code(&err_s),
        "both screens show the same 8 digits"
    );
    assert!(ok, "{err}");
    assert!(out.contains("1 added"), "{out}");
    assert!(err.contains("same code:"), "{err}");
    let (_, out, _) = run(&b, &["get", "vpn=u1"], "second password\n");
    assert_eq!(out.trim(), "wg-private");
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

#[test]
fn files_restored_to_an_earlier_boundary_are_found() {
    // An attacker (or a backup restore) puts back the store AND the audit log as they were at one
    // checkpoint: each file is consistent with itself, so only a record kept elsewhere can tell.
    let a = dir("rb");
    assert!(run(&a, &["init"], "first password\n").0);
    assert!(run(&a, &["set", "One", "n=1"], "first password\nsecret-one\n").0);
    let (store, audit) = (a.join("login.wkr"), a.join("login.audit"));
    let (old_store, old_audit) = (
        std::fs::read(&store).unwrap(),
        std::fs::read(&audit).unwrap(),
    );
    assert!(run(&a, &["set", "Two", "n=2"], "first password\nsecret-two\n").0);
    assert!(
        run(&a, &["audit", "verify"], "first password\n").0,
        "intact before the rollback"
    );
    std::fs::write(&store, &old_store).unwrap();
    std::fs::write(&audit, &old_audit).unwrap();
    let (ok, _, err) = run(&a, &["audit", "verify"], "first password\n");
    assert!(!ok && err.contains("rolled back"), "{err}");
    // Any unlock says so too, without refusing the owner.
    let (ok, out, err) = run(&a, &["get", "n=1"], "first password\n");
    assert!(ok && out.trim() == "secret-one", "{err}");
    assert!(
        err.contains("WARNING") && err.contains("rolled back"),
        "{err}"
    );
    // The owner can accept the older state as the new baseline; then verify is quiet again.
    let (ok, _, err) = run(&a, &["audit", "accept"], "first password\n");
    assert!(ok, "{err}");
    let (ok, _, err) = run(&a, &["audit", "verify"], "first password\n");
    assert!(ok, "{err}");
    let _ = std::fs::remove_dir_all(&a);
}
