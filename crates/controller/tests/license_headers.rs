// SPDX-License-Identifier: GPL-2.0-or-later
//! Every source file names the right license (tools/license/policy.json): a missing, wrong or
//! unexpected `SPDX-License-Identifier`, a stale license field or a stale license text fails.
//!
//! The check is on once `COPYRIGHT` names GPL-2.0-or-later (`relicense.py apply` copies it);
//! `WOLOS_LICENSE_ENFORCE=1` turns it on before that to see the list.
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn enforced() -> bool {
    std::env::var_os("WOLOS_LICENSE_ENFORCE").is_some()
        || std::fs::read_to_string(root().join("COPYRIGHT"))
            .is_ok_and(|s| s.contains("GPL-2.0-or-later (`LICENSE`)"))
}

#[test]
fn every_file_names_the_right_license() {
    if !enforced() {
        eprintln!("license policy not applied yet; skipped");
        return;
    }
    let out = Command::new("python3")
        .arg("-I")
        .arg(root().join("tools/license/relicense.py"))
        .args(["--root"])
        .arg(root())
        .arg("check")
        .output()
        .expect("python3 runs tools/license/relicense.py");
    assert!(
        out.status.success(),
        "license check failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn license_tools_are_tested() {
    if !enforced() {
        return;
    }
    let out = Command::new("python3")
        .current_dir(root().join("tools/license"))
        .args(["-m", "unittest", "test_relicense"])
        .output()
        .expect("python3 runs the unit tests");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
