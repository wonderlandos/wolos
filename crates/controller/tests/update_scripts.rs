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
        (
            "rw,subvol=/@wolos-a",
            &[][..],
            "wol update apply wolos-b --yes",
        ),
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

/// update-check in `d` with fake pacman/curl/df bodies; returns (exit code, stderr).
fn check(
    d: &Path,
    script: &Path,
    pacman: &str,
    curl: &str,
    df_avail_kb: u64,
) -> (Option<i32>, String) {
    for sub in ["db/local", "db/sync", "tmp", "out"] {
        fs::create_dir_all(d.join(sub)).unwrap();
    }
    fake(d, "pacman", pacman);
    fake(d, "curl", curl);
    fake(
        d,
        "df",
        &format!(
            "echo 'Filesystem 1024-blocks Used Available Capacity Mounted'; echo \"/dev/x 100 1 {df_avail_kb} 1% /\""
        ),
    );
    let o = Command::new("sh")
        .arg(script)
        .arg("now")
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_PACMAN_DB", d.join("db"))
        .env("TMPDIR", d.join("tmp"))
        .env("WOLOS_UPDATES", d.join("out"))
        .output()
        .unwrap();
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const CURL_OK: &str = "o=; p=; for u; do [ \"$p\" = -o ] && o=$u; p=$u; done\n\
case $u in *.sig) printf sig ;; *) printf '{}' > \"$o\" ;; esac";

fn check_script() -> PathBuf {
    std::env::var("WOL_UPDATE_CHECK_SCRIPT")
        .map_or_else(|_| repo("files/usr/lib/wolos/update-check"), PathBuf::from)
}

/// QA-STORE-12: `checked` is written only when the package lists were refreshed and read: a
/// package list pacman could not refresh leaves no stamp and exits 75 (try again later), so
/// "Last checked: now" never stands for a check that did not happen. A release manifest no server
/// gave does not undo the package check (review of B4): stamped, exit 0, and `base.error` says
/// why there is no release news; the next fetch that works clears it.
#[test]
fn stamp_only_on_success() {
    let ok_pacman = "case \"$*\" in *-Qu*) echo 'wol-shell 1-1 -> 2-1' ;; esac";
    for (why, pacman, curl, want, stamped, manifest_error) in [
        (
            "no release server answered",
            ok_pacman,
            "exit 22",
            0,
            true,
            true,
        ),
        (
            "offline",
            "case \"$*\" in *-Sy*) exit 1 ;; esac",
            CURL_OK,
            75,
            false,
            false,
        ),
        ("everything worked", ok_pacman, CURL_OK, 0, true, false),
    ] {
        let d = scratch("stamp");
        let (code, err) = check(&d, &check_script(), pacman, curl, 1 << 20);
        assert_eq!(code, Some(want), "{why}: {err}");
        assert_eq!(d.join("out/checked").exists(), stamped, "{why}");
        assert_eq!(d.join("out/base.error").exists(), manifest_error, "{why}");
        if want != 0 || manifest_error {
            assert!(
                err.starts_with("update-check: "),
                "{why}: a sentence: {err}"
            );
        }
    }
    // A fetch that works again clears the old error.
    let d = scratch("stamp-clear");
    fs::create_dir_all(d.join("out")).unwrap();
    fs::write(d.join("out/base.error"), "old\n").unwrap();
    let (code, err) = check(&d, &check_script(), ok_pacman, CURL_OK, 1 << 20);
    assert_eq!(code, Some(0), "{err}");
    assert!(!d.join("out/base.error").exists());
}

/// QA-UPG-1: the daily check never depends on `checkupdates`. The 0.12.0 check (wolos-system
/// 0.12.0-2, the script of tag v0.12.0) ran it first; it needs fakeroot, which the image does not
/// ship, so it said "Cannot find the fakeroot binary", exited 1, and the script left at once with
/// status 0: no `checked`, no `pkg`, no release manifest, for ever, on every 0.12.0 computer.
/// Here `checkupdates` fails as it does on the image, and the check must still read the packages
/// and the release, stamp, and never call it (or fakeroot). The proof against the old script:
/// `WOL_UPDATE_CHECK_SCRIPT=<v0.12.0 update-check> cargo test -p controller --test update_scripts`.
#[test]
fn check_runs_without_checkupdates_or_fakeroot() {
    let d = scratch("no-fakeroot");
    fake(
        &d,
        "checkupdates",
        "echo '==> ERROR: Cannot find the fakeroot binary.' >&2; exit 1",
    );
    fake(&d, "fakeroot", "exit 127");
    let ok_pacman = "case \"$*\" in *-Qu*) echo 'wolos-system 0.12.0-2 -> 0.13.0-1' ;; esac";
    let (code, err) = check(&d, &check_script(), ok_pacman, CURL_OK, 1 << 20);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        d.join("out/checked").exists(),
        "no stamp: the check stopped before it ran ({err})"
    );
    assert_eq!(
        fs::read_to_string(d.join("out/pkg")).unwrap_or_default(),
        "wolos-system 0.12.0-2 -> 0.13.0-1\n",
        "the package list"
    );
    assert!(
        d.join("out/base").exists(),
        "the release manifest was not fetched"
    );
    let calls = log(&d);
    assert!(
        !calls
            .lines()
            .any(|l| l.starts_with("checkupdates") || l.starts_with("fakeroot")),
        "the check calls checkupdates/fakeroot: {calls}"
    );
}

/// QA-LIVE4-10: on a full disk the findings were truncated to nothing and Check Now said "up to
/// date" twice. With less than 1 MiB free nothing is written and the check fails (1), naming it.
#[test]
fn disk_full_exit_nonzero() {
    let d = scratch("full");
    fs::create_dir_all(d.join("out")).unwrap();
    fs::write(d.join("out/pkg"), "wol-shell 1-1 -> 2-1\n").unwrap();
    let (code, err) = check(&d, &check_script(), "exit 0", CURL_OK, 100);
    assert_eq!(code, Some(1), "{err}");
    assert!(err.contains("no room left"), "{err}");
    assert_eq!(
        fs::read_to_string(d.join("out/pkg")).unwrap(),
        "wol-shell 1-1 -> 2-1\n",
        "the last findings stay"
    );
    assert!(!d.join("out/checked").exists());
}

/// QA-ACC-20: every fetch of the check is bounded and retried (a Wi-Fi drop or a portal's slow
/// answer is not the end of today's check).
#[test]
fn curl_retry_flags() {
    let d = scratch("retry");
    let log = d.join("log");
    let (code, err) = check(&d, &check_script(), "exit 0", CURL_OK, 1 << 20);
    assert_eq!(code, Some(0), "{err}");
    let calls: Vec<String> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .filter(|l| l.starts_with("curl "))
        .map(String::from)
        .collect();
    assert_eq!(calls.len(), 2, "manifest and signature: {calls:?}");
    for c in &calls {
        for flag in [
            "--retry 2",
            "--retry-connrefused",
            "--connect-timeout 15",
            "--max-time 60",
        ] {
            assert!(c.contains(flag), "{flag}: {c}");
        }
    }
}

/// QA-STORE-43: update-download never ends "nothing to download" (exit 0) when the check could
/// not refresh the package lists (offline, or the sealed database could not be refreshed): it
/// says so and exits 75, and no pacman runs.
#[test]
fn update_download_fails_when_the_check_fails() {
    let d = scratch("download-check-fails");
    fs::write(d.join("auto-update"), "download = on\nmetered = on\n").unwrap();
    fake(&d, "wol", "exit 69");
    fake(&d, "pacman", "exit 0");
    fake(&d, "pacman-conf", "echo /var/lib/pacman");
    let out = Command::new("sh")
        .arg(repo("files/usr/lib/wolos/update-download"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_AUTO_UPDATE", d.join("auto-update"))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(75), "{err}");
    assert!(err.contains("did not run"), "{err}");
    assert!(!log(&d).contains("pacman "), "{}", log(&d));
}

/// update-check with `args` (or update-settle for `settle`) and the fake tools already in `d`.
fn check_with(d: &Path, args: &[&str]) -> (Option<i32>, String) {
    // `settle`: the boot step next to the check (update-settle).
    let script = if args.first() == Some(&"settle") {
        repo("files/usr/lib/wolos/update-settle")
    } else {
        check_script()
    };
    let o = Command::new("sh")
        .arg(script)
        .args(args.iter().filter(|a| **a != "settle"))
        .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
        .env("WOLOS_PACMAN_DB", d.join("db"))
        .env("TMPDIR", d.join("tmp"))
        .env("WOLOS_UPDATES", d.join("out"))
        .output()
        .unwrap();
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// QA-UPG-5: right after the 0.12.0 -> 0.12.1 upgrade Software Update said "10 updates
/// available" and offered packages the new slot already had: the findings in the shared /var
/// were read against the old slot's installed packages. The check records which ones (`basis`);
/// at every start (`settle`, before sessions) findings that describe another package set are
/// dropped (the release manifest stays: it does not depend on the slot), and the next hourly run
/// checks again whatever the 20-hour stamp says. Findings of the running slot stay as they are.
#[test]
fn stale_findings_dropped_after_slot_switch() {
    let d = scratch("settle");
    let pacman = "case \"$*\" in *-Qu*) echo 'hyprland-wolos 1-1 -> 2-1'; echo 'linux-wolos 7.3.0-1 -> 7.3.1-1' ;; esac";
    for p in [
        "db/local/hyprland-wolos-1-1",
        "db/local/linux-wolos-7.3.0-1",
    ] {
        fs::create_dir_all(d.join(p)).unwrap();
    }
    let (code, err) = check(&d, &check_script(), pacman, CURL_OK, 1 << 20);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        d.join("out/basis").exists(),
        "the check records what it read"
    );
    // The same slot at the next start: nothing changes.
    let (code, err) = check_with(&d, &["settle"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(d.join("out/pkg").exists() && d.join("out/checked").exists());
    // A fresh stamp and the same packages: the hourly run waits.
    fs::write(d.join("log"), "").unwrap();
    let (code, _) = check_with(&d, &[]);
    assert_eq!(code, Some(0));
    assert!(!log(&d).contains("pacman"), "{}", log(&d));
    // The new slot holds the new versions: its first start drops what described the old one.
    fs::remove_dir_all(d.join("db/local")).unwrap();
    for p in [
        "db/local/hyprland-wolos-2-1",
        "db/local/linux-wolos-7.3.1-1",
    ] {
        fs::create_dir_all(d.join(p)).unwrap();
    }
    let (code, err) = check_with(&d, &["settle"]);
    assert_eq!(code, Some(0), "{err}");
    for f in ["pkg", "audit", "kernels", "checked", "basis"] {
        assert!(
            !d.join("out").join(f).exists(),
            "{f} described the other slot"
        );
    }
    assert!(d.join("out/base").exists(), "the release manifest stays");
    // Findings from a check that recorded nothing (the 0.12.0 script) are another slot's too.
    fs::write(d.join("out/pkg"), "hyprland-wolos 1-1 -> 2-1\n").unwrap();
    fs::write(d.join("out/checked"), "1\n").unwrap();
    let (code, _) = check_with(&d, &["settle"]);
    assert_eq!(code, Some(0));
    assert!(!d.join("out/pkg").exists());
    // A package change with a fresh stamp (a layer): the hourly run checks again.
    let (code, err) = check(&d, &check_script(), pacman, CURL_OK, 1 << 20);
    assert_eq!(code, Some(0), "{err}");
    fs::create_dir_all(d.join("db/local/htop-3.4-1")).unwrap();
    fs::write(d.join("log"), "").unwrap();
    let (code, err) = check_with(&d, &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(log(&d).contains("pacman --dbpath"), "{}", log(&d));
}
