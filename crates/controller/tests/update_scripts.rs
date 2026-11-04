// SPDX-License-Identifier: GPL-2.0-or-later
//! The update scripts under files/ (wolos-update-slot, update-check, update-download), run with
//! fake tools on PATH that log their arguments.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo(p: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(p)
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wol-update-scripts-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("bin")).unwrap();
    d
}

/// A fake tool in `d/bin` that logs `name args` to `d/log`, then runs `body`.
fn fake(d: &Path, name: &str, body: &str) {
    let f = d.join("bin").join(name);
    fs::write(
        &f,
        format!(
            "#!/bin/sh\necho \"{name} $*\" >> '{}'\n{body}\n",
            d.join("log").display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
}

fn log(d: &Path) -> String {
    fs::read_to_string(d.join("log")).unwrap_or_default()
}

fn run(d: &Path, script: &Path, args: &[&str]) -> Output {
    Command::new("sh")
        .arg(script)
        .args(args)
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .output()
        .unwrap()
}

/// QA-STORE-47: `wol update apply` takes a slot name (`wolos-b`, `update::is_valid_be`) and the
/// archive date `YYYY/MM/DD`; the wrapper passed the subvolume `@wolos-b`, which wol refuses.
#[test]
fn wolos_update_slot_names_the_slot_as_wol_does() {
    let script = std::env::var("WOL_UPDATE_SLOT_SCRIPT")
        .map_or_else(|_| repo("files/usr/bin/wolos-update-slot"), PathBuf::from);
    for (mount, args, want) in [
        ("rw,subvol=/@wolos-a", &[][..], "wol update apply wolos-b --yes"),
        (
            "rw,subvol=/@wolos-b",
            &["2026-12-01"][..],
            "wol update apply wolos-a 2026/12/01 --yes",
        ),
    ] {
        let d = scratch("slot");
        fake(&d, "findmnt", &format!("echo '{mount}'"));
        fake(&d, "wol", "exit 0");
        let o = run(&d, &script, args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let calls: Vec<String> = log(&d)
            .lines()
            .filter(|l| l.starts_with("wol "))
            .map(String::from)
            .collect();
        assert_eq!(calls, [want], "{mount} {args:?}");
    }
    // A date wol would refuse, or a computer without slots: a sentence, wol is never run.
    for (mount, args, code) in [
        ("rw,subvol=/@wolos-a", &["soon"][..], 64),
        ("rw,subvol=/@root", &[][..], 69),
    ] {
        let d = scratch("slot-bad");
        fake(&d, "findmnt", &format!("echo '{mount}'"));
        fake(&d, "wol", "exit 0");
        let o = run(&d, &script, args);
        assert_eq!(o.status.code(), Some(code), "{mount} {args:?}");
        assert!(!o.stderr.is_empty());
        assert!(!log(&d).contains("wol "), "{}", log(&d));
    }
}
