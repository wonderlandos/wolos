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
}
