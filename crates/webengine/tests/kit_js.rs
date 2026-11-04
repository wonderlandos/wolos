// SPDX-License-Identifier: BSD-2-Clause
//! The kit's JavaScript under cargo (ADR 0048, QA-WEB-12): `webapps/kit/test/*.test.mjs` run with
//! `node --test` when node is installed (and the logic tests of each app under `webapps/<app>/test`), so a change to `bridge.js` is tested by the same
//! `cargo test -p webengine` that tests its Rust end. Without node the test says so and passes
//! unless `WOL_NODE_REQUIRED` is set (CI and the forge set it). The browser test of an app
//! (`webapps/doctor/test/doctor.test.mjs`) needs `CHROME` and is run by hand.
use std::path::Path;
use std::process::Command;

/// The kit's tests and the logic tests of each app (`webapps/<app>/test/*.test.mjs`); the Doctor's
/// browser test, `doctor.test.mjs`, is among them and says it is skipped without `CHROME`.
fn kit_tests() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../webapps");
    let mut v: Vec<String> = Vec::new();
    for app in std::fs::read_dir(&root).unwrap_or_else(|e| panic!("{}: {e}", root.display())) {
        let dir = app.unwrap().path().join("test");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        v.extend(
            rd.map(|e| e.unwrap().path())
                .filter(|p| p.to_string_lossy().ends_with(".test.mjs"))
                .map(|p| p.display().to_string()),
        );
    }
    v.sort();
    v
}

#[test]
fn the_kit_javascript_tests_pass_under_node() {
    let files = kit_tests();
    assert!(!files.is_empty(), "no webapps/kit/test/*.test.mjs");
    let node = Command::new("node").arg("--version").output();
    if node.is_err() || !node.as_ref().is_ok_and(|o| o.status.success()) {
        assert!(
            std::env::var_os("WOL_NODE_REQUIRED").is_none(),
            "node is not installed and WOL_NODE_REQUIRED is set"
        );
        eprintln!("kit_js: node is not installed; the kit's JavaScript tests were not run");
        return;
    }
    let o = Command::new("node")
        .arg("--test")
        .args(&files)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}
