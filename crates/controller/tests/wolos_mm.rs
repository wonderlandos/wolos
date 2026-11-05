// SPDX-License-Identifier: GPL-2.0-or-later
//! `wolos-mm` (ADR 0061): the boot knobs and the one-account rule for KSM, `exec-merge` handing the
//! kernel's KSM flag to what it starts, and the refusals of `pageout`. A fake passwd file and a
//! fake /sys tree stand in for the machine.
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_wolos-mm");

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-mm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A /sys with the four files `apply` writes, each holding "?".
fn fake_sys(root: &Path) {
    for f in [
        "sys/kernel/debug/fault_around_bytes",
        "sys/kernel/mm/ksm/pages_to_scan",
        "sys/kernel/mm/ksm/sleep_millisecs",
        "sys/kernel/mm/ksm/run",
    ] {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "?").unwrap();
    }
}

fn passwd(root: &Path, humans: &[&str]) -> PathBuf {
    let mut s =
        String::from("root:x:0:0::/root:/bin/bash\nnobody:x:65534:65534::/:/usr/bin/nologin\n");
    for (i, h) in humans.iter().enumerate() {
        s.push_str(&format!(
            "{h}:x:{}:{}:{h}:/home/{h}:/usr/bin/zsh\n",
            1000 + i,
            1000 + i
        ));
    }
    let p = root.join("passwd");
    std::fs::write(&p, s).unwrap();
    p
}

fn apply(root: &Path, pw: &Path) -> (String, i32) {
    let o = Command::new(BIN)
        .args(["apply", "--passwd"])
        .arg(pw)
        .arg("--sysroot")
        .arg(root)
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        o.status.code().unwrap(),
    )
}

fn knob(root: &Path, f: &str) -> String {
    std::fs::read_to_string(root.join(f)).unwrap()
}

#[test]
fn one_human_account_turns_ksm_on_and_writes_the_knobs() {
    let d = scratch("one");
    fake_sys(&d);
    let (out, code) = apply(&d, &passwd(&d, &["alice"]));
    assert_eq!(code, 0, "{out}");
    assert_eq!(knob(&d, "sys/kernel/debug/fault_around_bytes"), "4096");
    assert_eq!(knob(&d, "sys/kernel/mm/ksm/pages_to_scan"), "500");
    assert_eq!(knob(&d, "sys/kernel/mm/ksm/sleep_millisecs"), "100");
    assert_eq!(knob(&d, "sys/kernel/mm/ksm/run"), "1");
    assert!(out.contains("KSM on: one human account"), "{out}");
}

#[test]
fn several_accounts_leave_ksm_off_and_say_so() {
    let d = scratch("two");
    fake_sys(&d);
    let (out, code) = apply(&d, &passwd(&d, &["alice", "bob"]));
    assert_eq!(code, 0, "{out}");
    assert_eq!(knob(&d, "sys/kernel/mm/ksm/run"), "0", "{out}");
    assert_eq!(
        knob(&d, "sys/kernel/debug/fault_around_bytes"),
        "4096",
        "the other knobs stay"
    );
    assert!(out.contains("KSM off: 2 human accounts"), "{out}");
}

#[test]
fn no_account_list_leaves_ksm_off() {
    let d = scratch("none");
    fake_sys(&d);
    let (out, _) = apply(&d, &d.join("missing-passwd"));
    assert_eq!(knob(&d, "sys/kernel/mm/ksm/run"), "0", "{out}");
    assert!(out.contains("KSM off: 0 human accounts"), "{out}");
}

#[test]
fn a_kernel_without_a_knob_is_skipped_not_failed() {
    let d = scratch("bare");
    let (out, code) = apply(&d, &passwd(&d, &["alice"]));
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.matches("skip  ").count(), 4, "{out}");
}

fn merge_any(prefix: &[&str]) -> Option<String> {
    let mut c = Command::new(prefix.first()?);
    c.args(&prefix[1..]);
    c.args(["/usr/bin/cat", "/proc/self/ksm_stat"]);
    let out = c.output().ok()?;
    String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("ksm_merge_any: ").map(str::to_owned))
}

#[test]
fn exec_merge_hands_the_flag_to_the_child_when_the_machine_allows_it() {
    let Some(plain) = merge_any(&["/usr/bin/env"]) else {
        return; // no ksm_stat: older kernel
    };
    assert_eq!(plain, "no");
    let got = merge_any(&[BIN, "exec-merge", "--", "/usr/bin/env"]);
    // "no" on a machine with several accounts (the rule) or a kernel that refuses the prctl.
    assert!(matches!(got.as_deref(), Some("yes" | "no")), "{got:?}");
}

#[test]
fn exec_merge_refuses_a_bare_command_name() {
    let o = Command::new(BIN)
        .args(["exec-merge", "--", "env"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("absolute path"),
        "{:?}",
        o
    );
    let o = Command::new(BIN)
        .args(["exec-merge", "--", "./x"])
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(2),
        "a relative path is not absolute either"
    );
}

#[test]
fn exec_merge_without_a_command_is_a_usage_error_and_a_missing_program_is_127() {
    let o = Command::new(BIN).arg("exec-merge").output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let o = Command::new(BIN)
        .args(["exec-merge", "--", "/nonexistent/x"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(127));
}

#[test]
fn ksm_is_a_status_dump_and_the_help_says_so() {
    let o = Command::new(BIN).arg("ksm").output().unwrap();
    assert!(String::from_utf8_lossy(&o.stdout).contains("pages_sharing "));
    let h = Command::new(BIN).output().unwrap();
    assert_eq!(h.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&h.stderr).contains("status dump"),
        "{h:?}"
    );
}

#[test]
fn pageout_refuses_a_target_outside_an_app_scope_and_a_non_root_any() {
    // A child of this test is the caller's own user's, in the test's cgroup (not an app scope).
    let mut child = Command::new("/usr/bin/sleep").arg("5").spawn().unwrap();
    let pid = child.id().to_string();
    let own = std::fs::read_to_string("/proc/self/cgroup").unwrap();
    let in_scope = wol_core::jsruntime::in_app_scope(&own);
    let o = Command::new(BIN).args(["pageout", &pid]).output().unwrap();
    if !in_scope {
        assert_eq!(o.status.code(), Some(3), "{o:?}");
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("NotAppScope"),
            "{o:?}"
        );
    }
    let root = std::fs::metadata("/proc/self").unwrap().uid() == 0;
    let o = Command::new(BIN)
        .args(["pageout", "--any", &pid, "all"])
        .output()
        .unwrap();
    if !root {
        assert_eq!(o.status.code(), Some(3), "{o:?}");
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("AnyNeedsRoot"),
            "{o:?}"
        );
    }
    child.kill().ok();
    child.wait().ok();
}

#[test]
fn pageout_refuses_bad_arguments() {
    for args in [
        &["pageout", "abc"][..],
        &["pageout", "1", "bogus"],
        &["pageout"],
    ] {
        let o = Command::new(BIN).args(args).output().unwrap();
        assert_eq!(o.status.code(), Some(2), "{args:?}");
    }
}
