// SPDX-License-Identifier: GPL-2.0-or-later
//! Web policy, track 1: the pacman hook's helper (`files/usr/lib/wolos/webpolicy`) applies the
//! byte patches `wol_core::webpolicy::plan` compiles, only to the file version they were written
//! for, once, again after a reinstall of that version, and never to a version that moved. Run
//! against a scratch root with the real helper.
#[path = "support/repo.rs"]
mod repo;
use repo::{read, repo};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use wol_core::webpolicy::{self, Op, Patch, Policy, StateLine};

const HELPER: &str = "files/usr/lib/wolos/webpolicy";
const FILE: &str = "/usr/lib/chat/app.js";

struct Root(PathBuf);

impl Root {
    fn new(tag: &str) -> Root {
        let d = std::env::temp_dir().join(format!("wol-webpolicy-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("plans")).unwrap();
        Root(d)
    }
    fn file(&self, abs: &str) -> PathBuf {
        self.0.join("root").join(abs.trim_start_matches('/'))
    }
    fn install(&self, abs: &str, bytes: &[u8]) {
        let f = self.file(abs);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        let _ = fs::remove_file(&f);
        fs::write(&f, bytes).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
    }
    fn plan(&self, app: &str, text: &str) {
        fs::write(self.0.join("plans").join(format!("{app}.plan")), text).unwrap();
    }
    fn run(&self) -> Output {
        let o = Command::new("sh")
            .arg(repo(HELPER))
            .env("WEBPOLICY_ROOT", self.0.join("root"))
            .env("WEBPOLICY_PLANS", self.0.join("plans"))
            .env("WEBPOLICY_STATE", self.0.join("state"))
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        o
    }
    fn read(&self, abs: &str) -> Vec<u8> {
        fs::read(self.file(abs)).unwrap()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn patch(original: &[u8], offset: u64, find: &str, with: Option<&str>) -> Patch {
    Patch {
        file: FILE.into(),
        sha256: webpolicy::sha256_hex(original),
        op: if with.is_some() {
            Op::Replace
        } else {
            Op::Remove
        },
        offset,
        find: find.into(),
        with: with.map(Into::into),
    }
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

const V1: &[u8] = b"start();sendTelemetry(\"x y\");setInterval(poll, 16);end();\n";

/// Where `16` (the poll interval) sits in V1.
fn at16() -> u64 {
    V1.windows(2).position(|w| w == b"16").unwrap() as u64
}

fn policy() -> Policy {
    let p = Policy {
        patches: vec![
            patch(V1, 8, "sendTelemetry(\"x y\");", None),
            patch(V1, at16(), "16", Some("1000/*\u{e9}*/")),
        ],
        ..Policy::default()
    };
    // What the helper gets is what the decoder accepts.
    webpolicy::parse(&serde_json::to_vec(&p).unwrap()).unwrap()
}

fn want() -> Vec<u8> {
    let mut out = V1.to_vec();
    for x in policy().patches.iter().rev() {
        out = Patch {
            sha256: webpolicy::sha256_hex(&out),
            ..x.clone()
        }
        .apply(&out)
        .unwrap();
    }
    out
}

#[test]
fn patches_apply_once_and_again_after_a_reinstall_of_the_same_version() {
    let r = Root::new("apply");
    r.install(FILE, V1);
    r.plan("chat", &webpolicy::plan(&policy()));
    let o = r.run();
    assert_eq!(r.read(FILE), want(), "{}", stderr(&o));
    assert_eq!(
        String::from_utf8(want()).unwrap(),
        "start();setInterval(poll, 1000/*\u{e9}*/);end();\n"
    );
    let mode = fs::metadata(r.file(FILE)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "the packaged mode stays");
    let state = fs::read_to_string(r.0.join("state/chat.state")).unwrap();
    assert_eq!(
        state,
        format!("patched {FILE} {}\n", webpolicy::sha256_hex(&want()))
    );
    // Again: already patched, nothing changes and nothing is refused.
    let o = r.run();
    assert_eq!(r.read(FILE), want());
    assert!(!stderr(&o).contains("refused"), "{}", stderr(&o));
    // pacman puts the same version back (reinstall): patched again, one state line.
    r.install(FILE, V1);
    r.run();
    assert_eq!(r.read(FILE), want());
    assert_eq!(
        fs::read_to_string(r.0.join("state/chat.state"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    // No leftovers next to the file.
    let names: Vec<_> = fs::read_dir(r.file(FILE).parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
}

#[test]
fn a_moved_version_or_a_wrong_offset_is_refused_and_left_as_packaged() {
    let r = Root::new("refuse");
    let v2 = b"start();sendTelemetry(\"x y\");setInterval(poll, 32);end();\n";
    r.install(FILE, v2);
    r.plan("chat", &webpolicy::plan(&policy()));
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let o = r.run();
    assert_eq!(r.read(FILE), v2, "untouched");
    assert!(stderr(&o).contains("not the version the patch was written for, refused"));
    // Recorded for Settings: reason, time, expected and found hash.
    let state = |r: &Root| {
        webpolicy::parse_state(&fs::read_to_string(r.0.join("state/chat.state")).unwrap())
    };
    match state(&r).as_slice() {
        [
            StateLine::Refused {
                file,
                time,
                expected,
                found,
                reason,
            },
        ] => {
            assert_eq!(file, FILE);
            assert!(*time >= before && *time < before + 600, "{time}");
            assert_eq!(*expected, webpolicy::sha256_hex(V1));
            assert_eq!(found.as_deref(), Some(webpolicy::sha256_hex(v2).as_str()));
            assert_eq!(reason, "not the version the patch was written for");
        }
        other => panic!("{other:?}"),
    }
    // Right version, but one edit's bytes are wrong: no edit of that file is applied.
    r.install(FILE, V1);
    let bad =
        webpolicy::plan(&policy()).replace(&format!(" {} ", at16()), &format!(" {} ", at16() + 1));
    assert!(bad.contains(&format!(" {} ", at16() + 1)));
    r.plan("chat", &bad);
    let o = r.run();
    assert_eq!(r.read(FILE), V1, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("are not the ones to edit, refused"),
        "{}",
        stderr(&o)
    );
    // One line per file: the new refusal replaced the old one.
    match state(&r).as_slice() {
        [StateLine::Refused { reason, found, .. }] => {
            assert!(reason.contains("are not the ones to edit"), "{reason}");
            assert_eq!(found.as_deref(), Some(webpolicy::sha256_hex(V1).as_str()));
        }
        other => panic!("{other:?}"),
    }
    // The policy applies once the bytes are right: the refusal is gone.
    r.plan("chat", &webpolicy::plan(&policy()));
    r.run();
    assert_eq!(r.read(FILE), want());
    assert!(matches!(state(&r).as_slice(), [StateLine::Patched { .. }]));
}

#[test]
fn only_plain_paths_under_the_roots_and_never_through_a_link() {
    let r = Root::new("paths");
    let sum = webpolicy::sha256_hex(V1);
    r.install("/etc/app.conf", V1);
    r.install("/usr/lib/chat/real.js", V1);
    std::os::unix::fs::symlink(
        r.file("/usr/lib/chat/real.js"),
        r.file("/usr/lib/chat/link.js"),
    )
    .unwrap();
    let text = format!(
        "# hand-written\n/etc/app.conf {sum} 0 7374 -\n/usr/lib/../etc/app.conf {sum} 0 7374 -\n\
         /usr/lib/chat/link.js {sum} 0 7374 -\n/usr/lib/missing.js {sum} 0 7374 -\n"
    );
    r.plan("evil", &text);
    let o = r.run();
    let e = stderr(&o);
    assert!(
        e.contains("/etc/app.conf: not under /usr/lib or /opt, refused"),
        "{e}"
    );
    assert!(e.contains("not a plain path, refused"), "{e}");
    let st = webpolicy::parse_state(&fs::read_to_string(r.0.join("state/evil.state")).unwrap());
    let refused: Vec<_> = st
        .iter()
        .filter_map(|l| match l {
            StateLine::Refused {
                file, found: None, ..
            } => Some(file.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(refused, ["/etc/app.conf", "/usr/lib/../etc/app.conf"]);
    assert_eq!(r.read("/etc/app.conf"), V1);
    assert_eq!(r.read("/usr/lib/chat/real.js"), V1, "not through the link");
}

/// The hook runs the helper after installs and upgrades of a plan or a patched file; the helper
/// is POSIX sh with the WolOS header and is shipped executable.
#[test]
fn the_hook_runs_the_shipped_helper() {
    let h = read("files/usr/share/libalpm/hooks/wolos-webpolicy.hook");
    for want in [
        "Type = Path",
        "Operation = Install",
        "Operation = Upgrade",
        "Target = usr/share/wolos/webpolicy/*.plan",
        "When = PostTransaction",
        "Exec = /usr/lib/wolos/webpolicy",
    ] {
        assert!(h.lines().any(|l| l == want), "{want}");
    }
    let s = read(HELPER);
    assert!(s.starts_with("#!/bin/sh\n# SPDX-License-Identifier: GPL-2.0-or-later\n# "));
    assert!(s.lines().any(|l| l == "set -eu"));
    let mode = fs::metadata(repo(HELPER)).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "executable");
    // The shipped role policies and their compiled plans.
    for f in [
        "code.json",
        "code.plan",
        "electron.json",
        "electron.plan",
        "helium.json",
        "helium.plan",
    ] {
        let p = repo("files/usr/share/wolos/webpolicy").join(f);
        assert!(Path::new(&p).is_file(), "{f}");
    }
}
