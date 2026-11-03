// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol-xguard scan`, the way installers call it.
use std::process::Command;
use xguard::automaton::{Pattern, Place, Tok};
use xguard::db::Builder;

const MARKER: &str = "XGUARD-CLI-MARKER-DEADBEEF-5678";

#[test]
fn a_file_that_cannot_be_opened_does_not_lose_the_others() {
    let d = std::env::temp_dir().join(format!("xg-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("rules")).unwrap();
    let mut b = Builder::default();
    assert!(b.rule(
        "Test.Cli",
        vec![Pattern {
            toks: vec![Tok::Lit(MARKER.as_bytes().to_vec())],
            place: Place::Anywhere,
            kinds: 0,
            id: 0,
            part: 0,
        }]
    ));
    std::fs::write(d.join("rules/t.xgdb"), b.build(1, 1)).unwrap();
    let (good, bad) = (d.join("good"), d.join("bad"));
    std::fs::write(&good, "nothing").unwrap();
    std::fs::write(&bad, MARKER).unwrap();
    let scan = |paths: &[&std::path::Path]| {
        let o = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
            .args(["scan", "--json", "--rules"])
            .arg(d.join("rules"))
            .args(paths)
            .env_remove(xguard::clam::SOCKET_ENV)
            .output()
            .unwrap();
        (
            o.status.code().unwrap(),
            String::from_utf8_lossy(&o.stdout).lines().count(),
        )
    };
    let gone = d.join("gone");
    assert_eq!(scan(&[&gone, &good]), (3, 1), "could not run, one report");
    assert_eq!(scan(&[&gone, &bad, &good]), (1, 2), "a finding outranks it");
    assert_eq!(scan(&[&good]), (0, 1));
    // S4-16: after `--` a name that starts with `-` is a file, not a flag.
    let o = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
        .current_dir(&d)
        .args(["scan", "--json", "--rules"])
        .arg(d.join("rules"))
        .args(["--", "--json"])
        .env_remove(xguard::clam::SOCKET_ENV)
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(3),
        "a file named --json that is not there"
    );
    std::fs::write(d.join("--json"), MARKER).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
        .current_dir(&d)
        .args(["scan", "--json", "--rules"])
        .arg(d.join("rules"))
        .args(["--", "--json"])
        .env_remove(xguard::clam::SOCKET_ENV)
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(1),
        "the file named --json was scanned"
    );
}

fn report(args: &[&std::ffi::OsStr], socket: Option<&std::path::Path>) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_wol-xguard"));
    c.args(["scan", "--json"]).args(args);
    c.env_remove(xguard::clam::SOCKET_ENV);
    if let Some(s) = socket {
        c.env(xguard::clam::SOCKET_ENV, s);
    }
    let o = c.output().unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

#[test]
fn asked_for_clamav_with_none_here_is_not_clean() {
    let d = std::env::temp_dir().join(format!("xg-cli-need-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("rules")).unwrap();
    std::fs::write(d.join("rules/t.xgdb"), Builder::default().build(1, 1)).unwrap();
    let f = d.join("file");
    std::fs::write(&f, "nothing").unwrap();
    let rules = d.join("rules");
    let args: Vec<&std::ffi::OsStr> = vec![
        "--clamav".as_ref(),
        "--rules".as_ref(),
        rules.as_ref(),
        f.as_ref(),
    ];
    let (code, out) = report(&args, None);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("\"verdict\":\"incomplete\""), "{out}");
    assert!(out.contains("clamav: no signatures"), "{out}");
}

/// With `XGUARD_CLAM_BIN` and `XGUARD_CLAM_DB` (the full database): a real scan of EICAR.
#[test]
fn real_clamav_names_eicar_and_passes_a_clean_file() {
    let (Some(bin), Some(db)) = (
        std::env::var_os("XGUARD_CLAM_BIN"),
        std::env::var_os("XGUARD_CLAM_DB"),
    ) else {
        eprintln!("XGUARD_CLAM_BIN or XGUARD_CLAM_DB not set: not tested here");
        return;
    };
    let d = std::env::temp_dir().join(format!("xg-cli-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("rules")).unwrap();
    std::fs::write(d.join("rules/t.xgdb"), Builder::default().build(1, 1)).unwrap();
    let sock = d.join("sock");
    let mut serve = Command::new(bin)
        .args(["serve", "--idle", "120", "--db"])
        .arg(db)
        .arg("--socket")
        .arg(&sock)
        .spawn()
        .unwrap();
    for _ in 0..600 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let (eicar, clean) = (d.join("eicar.com"), d.join("note.txt"));
    let sig = concat!(
        "X5O!P%@AP[4\\PZX54(P^)7CC)7}$",
        "EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*"
    );
    std::fs::write(&eicar, sig).unwrap();
    std::fs::write(&clean, "a note").unwrap();
    let scan = |f: &std::path::Path| {
        let rules = d.join("rules");
        let args: Vec<&std::ffi::OsStr> = vec![
            "--clamav".as_ref(),
            "--rules".as_ref(),
            rules.as_ref(),
            f.as_ref(),
        ];
        report(&args, Some(&sock))
    };
    let (found, clean) = (scan(&eicar), scan(&clean));
    serve.kill().ok();
    serve.wait().ok();
    assert_eq!(found.0, 1, "{}", found.1);
    assert!(found.1.contains("Eicar-Test-Signature"), "{}", found.1);
    assert_eq!(clean.0, 0, "{}", clean.1);
    assert!(clean.1.contains("\"verdict\":\"clean\""), "{}", clean.1);
}
