// SPDX-License-Identifier: GPL-2.0-or-later
//! The guard against the real kernel. Needs root and a directory on its own mount:
//! `sudo XGUARD_TEST_DIR=/mnt/b/t <test binary> --test-threads=1`. Skipped otherwise.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use xguard::automaton::{Pattern, Place, Tok};
use xguard::db::Builder;

const MARKER: &str = "XGUARD-TEST-MARKER-CAFEBABE-1234";

struct Guard {
    child: Child,
    dir: PathBuf,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

fn root() -> Option<PathBuf> {
    // SAFETY: plain syscall.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("skipped: needs root");
        return None;
    }
    std::env::var_os("XGUARD_TEST_DIR").map(PathBuf::from)
}

fn start(base: &Path, tag: &str) -> Guard {
    let dir = base.join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["rules", "state", "work", "dl"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let mut b = Builder::default();
    assert!(b.rule(
        "Test.Marker",
        vec![Pattern {
            toks: vec![Tok::Lit(MARKER.as_bytes().to_vec())],
            place: Place::Anywhere,
            kinds: 0,
            id: 0,
            part: 0,
        }]
    ));
    std::fs::write(dir.join("rules/test.xgdb"), b.build(1, 1)).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
        .arg("guard")
        .arg("--state")
        .arg(dir.join("state"))
        .arg("--rules")
        .arg(dir.join("rules"))
        .arg("--scope")
        .arg(&dir)
        .arg("--downloads")
        .arg(dir.join("dl"))
        .arg("--socket")
        .arg(dir.join("sock"))
        .spawn()
        .unwrap();
    let g = Guard { child, dir };
    wait(|| g.dir.join("state/status.json").exists());
    std::thread::sleep(Duration::from_millis(100));
    g
}

fn wait(f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(10), "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n# {body}\nexit 7\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// exit code, or the OS error when it could not start.
fn exec(path: &Path) -> Result<i32, i32> {
    match Command::new(path).status() {
        Ok(s) => Ok(s.code().unwrap_or(-1)),
        Err(e) => Err(e.raw_os_error().unwrap_or(0)),
    }
}

fn scanned(g: &Guard) -> (u64, u64) {
    // status.json is rewritten every 5 s: ask through the findings-free counters file by waiting.
    std::thread::sleep(Duration::from_millis(5200));
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(g.dir.join("state/status.json")).unwrap()).unwrap();
    (v["asked"].as_u64().unwrap(), v["scanned"].as_u64().unwrap())
}

#[test]
fn a_clean_file_runs_a_marked_file_does_not() {
    let Some(base) = root() else { return };
    let g = start(&base, "basic");
    let good = g.dir.join("work/good");
    script(&good, "harmless");
    assert_eq!(exec(&good), Ok(7));
    let bad = g.dir.join("work/bad");
    script(&bad, MARKER);
    assert_eq!(exec(&bad), Err(libc::EPERM));
    assert_eq!(exec(&bad), Err(libc::EPERM), "remembered");
    let found: serde_json::Value =
        serde_json::from_slice(&std::fs::read(g.dir.join("state/findings-0.json")).unwrap())
            .unwrap();
    assert_eq!(found[0]["name"], "Test.Marker");
    assert_eq!(found[0]["path"], bad.to_str().unwrap());
}

#[test]
fn after_the_first_run_the_kernel_stops_asking_until_the_file_changes() {
    let Some(base) = root() else { return };
    let g = start(&base, "ignore");
    let f = g.dir.join("work/tool");
    script(&f, "harmless");
    assert_eq!(exec(&f), Ok(7));
    let before = scanned(&g);
    assert_eq!(before, (1, 1), "one question, one scan");
    for _ in 0..20 {
        assert_eq!(exec(&f), Ok(7));
    }
    assert_eq!(scanned(&g), before, "no event at all once clean");
    // Writing to it clears the kernel's mark: the next run is a new question.
    script(&f, MARKER);
    assert_eq!(exec(&f), Err(libc::EPERM));
}

#[test]
fn a_restarted_guard_remembers_without_reading() {
    let Some(base) = root() else { return };
    let mut g = start(&base, "restart");
    let f = g.dir.join("work/tool");
    script(&f, "harmless");
    assert_eq!(exec(&f), Ok(7));
    g.child.kill().unwrap();
    g.child.wait().unwrap();
    std::fs::remove_file(g.dir.join("state/status.json")).unwrap();
    g.child = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
        .arg("guard")
        .arg("--state")
        .arg(g.dir.join("state"))
        .arg("--rules")
        .arg(g.dir.join("rules"))
        .arg("--scope")
        .arg(&g.dir)
        .arg("--socket")
        .arg(g.dir.join("sock"))
        .spawn()
        .unwrap();
    wait(|| g.dir.join("state/status.json").exists());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(exec(&f), Ok(7));
    assert_eq!(scanned(&g), (1, 0), "asked once, read never");
}

#[test]
fn when_the_guard_dies_everything_still_runs() {
    let Some(base) = root() else { return };
    let mut g = start(&base, "die");
    let f = g.dir.join("work/after");
    script(&f, MARKER);
    g.child.kill().unwrap();
    g.child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(exec(&f), Ok(7));
}

#[test]
fn a_new_download_is_tagged_and_checked_on_first_open() {
    let Some(base) = root() else { return };
    let g = start(&base, "dl");
    let f = g.dir.join("dl/report.pdf");
    std::fs::write(&f, format!("%PDF-1.7 {MARKER}")).unwrap();
    wait(|| xattr(&f, "user.wolos.quarantine"));
    std::thread::sleep(Duration::from_millis(100));
    let e = std::fs::read(&f).unwrap_err();
    assert_eq!(e.raw_os_error(), Some(libc::EPERM));
    let ok = g.dir.join("dl/notes.pdf");
    std::fs::write(&ok, "%PDF-1.7 fine").unwrap();
    wait(|| xattr(&ok, "user.wolos.quarantine"));
    std::thread::sleep(Duration::from_millis(100));
    assert!(std::fs::read(&ok).is_ok());
    let seen = scanned(&g);
    for _ in 0..5 {
        std::fs::read(&ok).unwrap();
    }
    assert_eq!(scanned(&g), seen, "the mark is gone after the first check");
    // A file elsewhere is opened without a question.
    let other = g.dir.join("work/plain.txt");
    std::fs::write(&other, MARKER).unwrap();
    assert!(std::fs::read(&other).is_ok());
}

#[test]
fn a_file_with_fs_verity_is_known_by_its_digest_alone() {
    let Some(base) = root() else { return };
    let g = start(&base, "verity");
    let f = g.dir.join("work/sealed-tool");
    script(&f, "harmless");
    let copy = g.dir.join("work/sealed-copy");
    script(&copy, "harmless");
    for p in [&f, &copy] {
        let st = Command::new("fsverity").arg("enable").arg(p).status();
        if !st.is_ok_and(|s| s.success()) {
            eprintln!("skipped: no fs-verity on this filesystem");
            return;
        }
    }
    assert_eq!(exec(&f), Ok(7));
    assert_eq!(exec(&copy), Ok(7));
    assert_eq!(scanned(&g), (2, 1), "the second file was never read");
}

#[test]
fn open_anyway_lets_this_content_run_and_only_this_content() {
    let Some(base) = root() else { return };
    let g = start(&base, "allow");
    let bad = g.dir.join("work/bad");
    script(&bad, MARKER);
    assert_eq!(exec(&bad), Err(libc::EPERM));
    let sock = g.dir.join("sock");
    let f = std::fs::File::open(&bad).unwrap();
    assert_eq!(xguard::ctl::ask(&sock, "check", &f).unwrap().state, "found");
    assert_eq!(
        xguard::ctl::ask(&sock, "allow", &f).unwrap().state,
        "allowed"
    );
    assert_eq!(exec(&bad), Ok(7));
    let found = std::fs::read(g.dir.join("state/findings-0.json")).unwrap();
    assert_eq!(found, b"[]", "nothing left to show");
    let other = g.dir.join("work/other");
    script(&other, &format!("{MARKER} changed"));
    assert_eq!(exec(&other), Err(libc::EPERM));
}

fn xattr(p: &Path, name: &str) -> bool {
    let (Ok(c), Ok(n)) = (
        std::ffi::CString::new(p.to_str().unwrap()),
        std::ffi::CString::new(name),
    ) else {
        return false;
    };
    // SAFETY: size 0 asks for the length only.
    unsafe { libc::getxattr(c.as_ptr(), n.as_ptr(), std::ptr::null_mut(), 0) >= 0 }
}
