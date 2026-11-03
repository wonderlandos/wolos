// SPDX-License-Identifier: GPL-2.0-or-later
//! Audit R7-1: the background pass, run as an account that is not root, publishes what it found
//! as `sweep-<uid>.json` in its state directory, owned by the file's account and 0600, where the
//! shell's alerts read it (`wol_core::malware::sweep_file`).
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
    assert!(!err.contains("could not write"), "{err}");

    let owner = std::fs::metadata(home.join("evil.exe")).unwrap().uid();
    let list = state.join(wol_core::malware::sweep_file(owner));
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
    let _ = std::fs::remove_dir_all(&d);
}

/// Why the unit keeps CAP_CHOWN (audit R7-1): without it, handing a list to another account
/// fails and nothing appears at the path, so the shell would show nothing.
#[test]
fn giving_a_list_to_another_account_needs_cap_chown() {
    if shared::sys::euid() == 0 {
        return; // root may chown: nothing to show here.
    }
    let d = std::env::temp_dir().join(format!("xg-chown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join(wol_core::malware::sweep_file(0));
    let e = shared::atomic::write_as(&p, 0o600, Some(0), b"[]").unwrap_err();
    assert_eq!(e.raw_os_error(), Some(libc::EPERM), "{e}");
    assert!(!p.exists(), "nothing half-written at the path");
    assert_eq!(
        std::fs::read_dir(&d).unwrap().count(),
        0,
        "no temporary left"
    );
    let _ = std::fs::remove_dir_all(&d);
}
