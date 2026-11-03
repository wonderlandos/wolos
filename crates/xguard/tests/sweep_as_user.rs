// SPDX-License-Identifier: GPL-2.0-or-later
//! Audit R7-1 / DT-6: the background pass, run as an account that is not root, hands what it
//! found to the guard over its socket; the guard writes `sweep-<uid>.json` in its own folder,
//! owned by the file's account and 0600, where the shell's alerts read it
//! (`wol_core::malware::sweep_path`).
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::process::Command;
use xguard::automaton::{Pattern, Place, Tok};
use xguard::db::Builder;

const MARKER: &str = "XGUARD-SWEEP-MARKER-CAFEBABE-4242";

fn rules(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let mut b = Builder::default();
    assert!(b.rule(
        "Test.Sweep",
        vec![Pattern {
            toks: vec![Tok::Lit(MARKER.as_bytes().to_vec())],
            place: Place::Anywhere,
            kinds: 0,
            id: 0,
            part: 0,
        }]
    ));
    std::fs::write(dir.join("t.xgdb"), b.build(1, 1)).unwrap();
}

/// The guard's socket, as far as `findings` goes: each connection's list goes through
/// [`xguard::guard::take_sweep`], trusting this test's account as the pass.
fn guard_socket(sock: &std::path::Path, guard_dir: std::path::PathBuf) {
    let l = std::os::unix::net::UnixListener::bind(sock).unwrap();
    std::thread::spawn(move || {
        for c in l.incoming().flatten() {
            let Ok((verb, Some(fd))) = xguard::ctl::recv_fd(&c) else {
                continue;
            };
            let uid = verb.strip_prefix(xguard::ctl::FINDINGS).unwrap_or("x");
            let r = xguard::guard::take_sweep(
                &guard_dir,
                Some(shared::sys::euid()),
                xguard::ctl::peer_uid(&c),
                uid,
                &std::fs::File::from(fd),
            );
            xguard::ctl::write_reply(
                &c,
                &xguard::ctl::Reply {
                    state: if r.is_ok() { "taken" } else { "refused" }.into(),
                    name: String::new(),
                },
            );
        }
    });
}

/// DT-6: the pass hands what it found to the guard over the socket, and the guard writes it in
/// its own folder for the file's account (0600, owned by it). The pass's folder gets no list.
#[test]
fn a_sweep_hit_reaches_the_account_that_owns_the_file() {
    let d = std::env::temp_dir().join(format!("xg-sweep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    rules(&d.join("rules"));
    let home = d.join("home/alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("evil.exe"), MARKER).unwrap();
    std::fs::write(home.join("fine.sh"), "echo hi\n").unwrap();
    let state = d.join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o775)).unwrap();
    let guard = d.join("guard");
    std::fs::create_dir_all(&guard).unwrap();
    let sock = d.join("xguard.sock");
    guard_socket(&sock, guard.clone());

    let o = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
        .arg("sweep")
        .arg("--rules")
        .arg(d.join("rules"))
        .arg("--socket")
        .arg(&sock)
        .arg(&state)
        .arg(d.join("home"))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "sweep failed: {err}");
    assert!(!err.contains("could not hand"), "{err}");

    let owner = std::fs::metadata(home.join("evil.exe")).unwrap().uid();
    let list = guard.join(wol_core::malware::sweep_file(owner));
    let m = std::fs::metadata(&list).expect("the account's findings list exists");
    assert_eq!(m.uid(), owner, "owned by the account the file belongs to");
    assert_eq!(m.mode() & 0o777, 0o600, "nobody else reads it");
    let found: Vec<wol_core::malware::Blocked> =
        serde_json::from_str(&std::fs::read_to_string(&list).expect("readable by the account"))
            .unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].path.ends_with("/evil.exe"), "{found:?}");
    assert_eq!(found[0].name, "Test.Sweep");
    assert_eq!(found[0].uid, owner);
    // What the shell's alerts do with the account's lists.
    assert_eq!(
        wol_core::malware::merge(&[std::fs::read_to_string(&list).unwrap().as_str()]).len(),
        1
    );
    let mine: Vec<String> = std::fs::read_dir(&state)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .collect();
    assert!(mine.is_empty(), "the pass wrote a list itself: {mine:?}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Without the guard to take a list, the pass says so and fails (the unit shows it), and writes
/// no list itself: it has no right to give one to an account any more (DT-6).
#[test]
fn no_guard_no_list_and_the_pass_says_so() {
    let d = std::env::temp_dir().join(format!("xg-noguard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    rules(&d.join("rules"));
    let home = d.join("home/alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("evil.exe"), MARKER).unwrap();
    let state = d.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
        .arg("sweep")
        .arg("--rules")
        .arg(d.join("rules"))
        .arg("--socket")
        .arg(d.join("nobody.sock"))
        .arg(&state)
        .arg(d.join("home"))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success(), "a list that went nowhere is a failure");
    assert!(err.contains("could not hand the list"), "{err}");
    let owner = std::fs::metadata(home.join("evil.exe")).unwrap().uid();
    assert!(!state.join(wol_core::malware::sweep_file(owner)).exists());
    let _ = std::fs::remove_dir_all(&d);
}

/// F10: the guard's `verdicts` is root's (0644) and the pass runs as _xguard: the pass keeps its
/// own memory (`sweep-verdicts`) and never opens the guard's to write. Before, every pass that got
/// past the gate died on "Permission denied" (exit 3) and the unit failed.
#[test]
fn the_pass_leaves_the_guards_verdicts_alone() {
    let d = std::env::temp_dir().join(format!("xg-own-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    rules(&d.join("rules"));
    let home = d.join("home/alice");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("fine.sh"), "echo hi\n").unwrap();
    let state = d.join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o775)).unwrap();
    let guards = state.join("verdicts");
    std::fs::write(&guards, b"").unwrap();
    std::fs::set_permissions(&guards, std::fs::Permissions::from_mode(0o444)).unwrap();

    for _ in 0..2 {
        let o = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
            .arg("sweep")
            .arg("--rules")
            .arg(d.join("rules"))
            .arg(&state)
            .arg(d.join("home"))
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(o.status.success(), "sweep failed: {err}");
    }
    let g = std::fs::metadata(&guards).unwrap();
    assert_eq!(g.len(), 0, "the guard's file was not opened to write");
    assert_eq!(g.mode() & 0o777, 0o444);
    assert!(
        std::fs::metadata(state.join("sweep-verdicts")).is_ok_and(|m| m.len() > 0),
        "the pass remembers in its own file"
    );
    let _ = std::fs::remove_dir_all(&d);
}
