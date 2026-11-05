// SPDX-License-Identifier: GPL-2.0-or-later
//! Audit F10: the full database is fetched as `_xguard`; the guard (root) loads only what
//! `wol-xguard rules full` built, as root, from files whose publisher's signature checks. Each
//! refusal leaves the previous build as it was. `sigtool` is a stand-in here (`XGUARD_SIGTOOL`).
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Fix {
    base: PathBuf,
}

impl Fix {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!("xg-full-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // Modes are set here, not inherited from the umask of whoever runs the tests: `trusted()`
        // refuses a folder others or the group can write, and 0002 (a user with a private group)
        // would make these 0775 (QA-A9-SEC-5).
        std::fs::create_dir_all(&base).unwrap();
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
        for d in ["db", "tmp", "local", "src"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
            std::fs::set_permissions(base.join(d), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { base }
    }
    fn p(&self, s: &str) -> PathBuf {
        self.base.join(s)
    }
    /// `name.cvd`: a 512-byte header, then a gzipped tar of one hash list.
    fn cvd(&self, name: &str, version: u32) {
        let src = self.p("src").join(name);
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join(format!("{name}.hdb")),
            format!("44d88612fea8a8f36de82e1278abb02f:68:Test.{name}\n"),
        )
        .unwrap();
        let tgz = self.p(&format!("{name}.tgz"));
        assert!(
            Command::new("tar")
                .arg("-czf")
                .arg(&tgz)
                .arg("-C")
                .arg(&src)
                .arg(".")
                .status()
                .unwrap()
                .success()
        );
        let mut head =
            format!("ClamAV-VDB:03 Nov 2026 06-27 +0000:{version}:1:90:x:y:z:1").into_bytes();
        head.resize(512, 0);
        head.extend(std::fs::read(&tgz).unwrap());
        std::fs::write(self.p("db").join(format!("{name}.cvd")), head).unwrap();
    }
    /// A stand-in sigtool that prints `out` and exits `code`.
    fn sigtool(&self, out: &str, code: i32) -> PathBuf {
        let s = self.p("sigtool");
        std::fs::write(
            &s,
            format!("#!/bin/sh\nprintf '%s\\n' '{out}'\nexit {code}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o755)).unwrap();
        s
    }
    fn run(&self, sigtool: &Path, dest: &Path) -> (bool, String) {
        let o = Command::new(env!("CARGO_BIN_EXE_wol-xguard"))
            .args(["rules", "full"])
            .arg(self.p("db"))
            .arg(dest)
            .env("XGUARD_SIGTOOL", sigtool)
            .env("TMPDIR", self.p("tmp"))
            .output()
            .unwrap();
        (
            o.status.success(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }
    fn built(&self) -> Option<Vec<u8>> {
        std::fs::read(self.p("local/full.xgdb")).ok()
    }
    /// Root's record of the last attempt, next to DEST (Settings reads it).
    fn status(&self) -> Option<wol_core::malware::RulesStatus> {
        serde_json::from_slice(&std::fs::read(self.p(wol_core::malware::RULES_STATUS)).ok()?).ok()
    }
}

// Each test leaves no folder behind, pass or fail.
impl Drop for Fix {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[test]
fn a_signed_download_is_built_into_the_guards_folder() {
    let f = Fix::new("ok");
    f.cvd("main", 63);
    f.cvd("daily", 28173);
    let (ok, err) = f.run(&f.sigtool("Verification OK.", 0), &f.p("local"));
    assert!(ok, "{err}");
    let db = xguard::db::Db::open(&f.p("local/full.xgdb")).expect("a database");
    assert_eq!(db.version(), 28173, "the daily file's version");
    let s = f.status().expect("a status");
    assert!(s.ok && (s.main, s.daily) == (63, 28173), "{s:?}");
    assert_eq!(
        std::fs::read_dir(f.p("tmp")).unwrap().count(),
        0,
        "nothing left in the work folder"
    );
}

#[test]
fn a_download_whose_signature_does_not_verify_is_refused() {
    let f = Fix::new("bad");
    f.cvd("main", 63);
    f.cvd("daily", 28173);
    std::fs::write(f.p("local/full.xgdb"), b"before").unwrap();
    for (out, code) in [
        (
            "ERROR: cvdinfo: Verification: Can't verify database integrity",
            1,
        ),
        // Exit 0 is not enough: the line is.
        ("Digital signature: x", 0),
    ] {
        let (ok, err) = f.run(&f.sigtool(out, code), &f.p("local"));
        assert!(!ok, "accepted with sigtool saying {out:?}");
        assert!(err.contains("signature does not verify"), "{err}");
        assert_eq!(
            f.built().as_deref(),
            Some(&b"before"[..]),
            "the old build stays"
        );
        let s = f.status().expect("the refusal is recorded");
        assert!(
            !s.ok && s.reason.contains("signature does not verify"),
            "{s:?}"
        );
        let text = std::fs::read_to_string(f.p(wol_core::malware::RULES_STATUS)).ok();
        assert_eq!(
            wol_core::malware::refusal(text.as_deref()).as_deref(),
            Some("the publisher's signature does not verify"),
            "Settings reads it"
        );
    }
}

/// A file still signed but older than the one in use (a replay) is refused; the same or newer
/// goes in.
#[test]
fn an_older_signed_download_is_refused() {
    let f = Fix::new("old");
    let ok_tool = f.sigtool("Verification OK.", 0);
    f.cvd("main", 63);
    f.cvd("daily", 28173);
    assert!(f.run(&ok_tool, &f.p("local")).0);
    let built = f.built();
    f.cvd("daily", 28100);
    let (ok, err) = f.run(&ok_tool, &f.p("local"));
    assert!(
        !ok && err.contains("older than the signatures in use"),
        "{err}"
    );
    assert_eq!(f.built(), built, "the newer build stays");
    let s = f.status().unwrap();
    assert!(
        !s.ok && (s.main, s.daily) == (63, 28173),
        "versions in use kept: {s:?}"
    );
    f.cvd("main", 62);
    f.cvd("daily", 28174);
    assert!(
        !f.run(&ok_tool, &f.p("local")).0,
        "an older main is refused too"
    );
    f.cvd("main", 63);
    let (ok, err) = f.run(&ok_tool, &f.p("local"));
    assert!(ok, "{err}");
    assert!(f.status().unwrap().ok, "a good build clears the refusal");
}

#[test]
fn a_link_or_a_missing_file_in_the_download_is_refused() {
    let f = Fix::new("link");
    f.cvd("main", 63);
    f.cvd("daily", 28173);
    let ok_tool = f.sigtool("Verification OK.", 0);
    // _xguard points daily.cvd at a file of its choosing: not followed.
    let elsewhere = f.p("elsewhere.cvd");
    std::fs::rename(f.p("db/daily.cvd"), &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, f.p("db/daily.cvd")).unwrap();
    let (ok, err) = f.run(&ok_tool, &f.p("local"));
    assert!(!ok && err.contains("daily.cvd"), "{err}");
    assert!(f.built().is_none());
    std::fs::remove_file(f.p("db/daily.cvd")).unwrap();
    let (ok, _) = f.run(&ok_tool, &f.p("local"));
    assert!(!ok, "built without daily.cvd");
    assert!(f.built().is_none());
}

#[test]
fn a_destination_others_may_write_is_refused() {
    let f = Fix::new("dest");
    f.cvd("main", 63);
    f.cvd("daily", 28173);
    std::fs::set_permissions(f.p("local"), std::fs::Permissions::from_mode(0o775)).unwrap();
    let (ok, err) = f.run(&f.sigtool("Verification OK.", 0), &f.p("local"));
    assert!(!ok && err.contains("not root's alone"), "{err}");
    assert!(f.built().is_none());
}

/// What the guard loads from: not a link, not writable by others, root's or the caller's.
#[test]
fn signatures_load_only_from_a_folder_nobody_else_may_write() {
    let f = Fix::new("trust");
    assert!(xguard::trusted(&f.p("local")));
    std::fs::set_permissions(f.p("local"), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(!xguard::trusted(&f.p("local")));
    std::os::unix::fs::symlink(f.p("db"), f.p("linked")).unwrap();
    assert!(!xguard::trusted(&f.p("linked")));
    // The guard's real folders, from the tmpfiles line.
    assert!(xguard::RULES_DIR.starts_with(wol_core::malware::GUARD_DIR));
    assert!(xguard::LOCAL_DIR.starts_with(wol_core::malware::GUARD_DIR));
    assert_eq!(
        wol_core::malware::RULE_DIRS[0],
        xguard::RULES_DIR,
        "Settings reads the version where the guard loads"
    );
}
