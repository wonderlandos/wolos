// SPDX-License-Identifier: GPL-2.0-or-later
//! QA-LIVE-17: one btrfs holds /, /var and /home, and one plain account's `fallocate` took the pool
//! to 224 KB (journal, package database, updates and every other account with it). The homes'
//! subvolume is capped at 85 % of the pool (`home-quota`), set by the installer before the first
//! account and by wol-admind after every `useradd`; the journal is capped too (ADR 0023).
#![allow(dead_code)] // siblings.rs helpers this file does not call
#[path = "support/siblings.rs"]
mod siblings;

use std::{fs, path::Path, process::Command};

fn read(p: &str) -> String {
    fs::read_to_string(siblings::root().join(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wol-quota-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// Fake commands in DIR/bin that log "name args" to DIR/log; returns PATH.
fn fakes(d: &Path, names: &[(&str, &str)]) -> String {
    let bin = d.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for (n, body) in names {
        let p = bin.join(n);
        fs::write(
            &p,
            format!(
                "#!/bin/sh\necho \"{n} $*\" >> {}\n{body}\n",
                d.join("log").display()
            ),
        )
        .unwrap();
        Command::new("chmod").arg("+x").arg(&p).status().unwrap();
    }
    format!("{}:/usr/bin:/bin", bin.display())
}

/// findmnt as the pool's own @home mount answers it: the file system, and the mount options with
/// the subvolume (the kernel prints `subvol=/@home`).
const FINDMNT_HOME: &str = "case \"$*\" in *OPTIONS*) echo rw,noatime,compress=zstd:1,subvol=/@home;; *) echo btrfs;; esac";

fn run(_d: &Path, path: &str, arg: &[&str]) -> std::process::Output {
    Command::new("sh")
        .arg(siblings::root().join("files/usr/lib/wolos/home-quota"))
        .args(arg)
        .env("PATH", path)
        .output()
        .unwrap()
}

fn log(d: &Path) -> String {
    fs::read_to_string(d.join("log")).unwrap_or_default()
}

/// 85 % of the pool (blocks x block size), on the homes' subvolume, quotas switched on first;
/// plain quotas where simple ones are refused; never an error for the account being made.
#[test]
fn the_home_subvolume_is_capped_at_85_percent_of_the_pool() {
    let d = scratch("limit");
    // 1_000_000 blocks of 4096 bytes: a 4.096 GB pool.
    let path = fakes(
        &d,
        &[
            ("findmnt", FINDMNT_HOME),
            ("stat", "echo '1000000 4096'"),
            ("btrfs", ""),
        ],
    );
    let o = run(&d, &path, &["/mnt/home"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let l = log(&d);
    let limit = 1_000_000u64 * 4096 / 100 * 85;
    let enable = l.find("btrfs quota enable --simple /mnt/home").expect(&l);
    let set = l
        .find(&format!("btrfs qgroup limit {limit} /mnt/home"))
        .expect(&l);
    assert!(enable < set, "quotas on before the limit: {l}");
    assert!(l.contains("findmnt -n -o FSTYPE -T /mnt/home"), "{l}");

    // Simple quotas refused: plain ones are tried.
    let d = scratch("plain");
    let path = fakes(
        &d,
        &[
            ("findmnt", FINDMNT_HOME),
            ("stat", "echo '1000000 4096'"),
            ("btrfs", "case \"$*\" in *--simple*) exit 1;; esac"),
        ],
    );
    assert!(run(&d, &path, &[]).status.success());
    let l = log(&d);
    assert!(l.contains("btrfs quota enable /home"), "{l}");
    assert!(
        l.contains(&format!("btrfs qgroup limit {limit} /home")),
        "{l}"
    );

    // A limit that cannot be set is said, and the script still exits 0 (the account exists).
    let d = scratch("refused");
    let path = fakes(
        &d,
        &[
            ("findmnt", FINDMNT_HOME),
            ("stat", "echo '1000000 4096'"),
            ("btrfs", "case \"$*\" in *limit*) exit 1;; esac"),
        ],
    );
    let o = run(&d, &path, &[]);
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("was not set"));

    // Another file system (a kept ext4 /home): nothing is touched.
    let d = scratch("ext4");
    let path = fakes(
        &d,
        &[
            ("findmnt", "echo ext4"),
            ("stat", "echo '1000000 4096'"),
            ("btrfs", ""),
        ],
    );
    assert!(run(&d, &path, &[]).status.success());
    assert!(!log(&d).contains("btrfs"), "{}", log(&d));
}

/// Only the pool's own @home subvolume is capped (ADR 0023): a btrfs that is not mounted from
/// @home (a plain /home directory of the root subvolume, a kept /home on its own partition, another
/// subvolume's name that merely contains "@home") is left alone, with nothing run.
#[test]
fn only_the_pools_own_home_subvolume_is_capped() {
    for (name, opts) in [
        ("root-subvol", "rw,noatime,subvol=/@wolos-a"),
        ("no-subvol", "rw,noatime"),
        ("lookalike", "rw,subvol=/@home-old"),
        ("nested", "rw,subvol=/data/@home"),
    ] {
        let d = scratch(name);
        let body = format!("case \"$*\" in *OPTIONS*) echo {opts};; *) echo btrfs;; esac");
        let path = fakes(
            &d,
            &[
                ("findmnt", body.as_str()),
                ("stat", "echo '1000000 4096'"),
                ("btrfs", ""),
            ],
        );
        let o = run(&d, &path, &["/home"]);
        assert!(o.status.success(), "{name}");
        assert!(!log(&d).contains("btrfs"), "{name}: {}", log(&d));
    }
    // The spelling without the slash (as fstab writes it) is the same subvolume.
    let d = scratch("noslash");
    let path = fakes(
        &d,
        &[
            (
                "findmnt",
                "case \"$*\" in *OPTIONS*) echo rw,subvol=@home;; *) echo btrfs;; esac",
            ),
            ("stat", "echo '1000000 4096'"),
            ("btrfs", ""),
        ],
    );
    assert!(run(&d, &path, &[]).status.success());
    assert!(log(&d).contains("btrfs qgroup limit"), "{}", log(&d));
}

/// A share that is not a number from 1 to 100, or a pool of zero blocks, never breaks "exit 0" and
/// never sets a limit of 0 (which would lock the homes at once): the default 85 % is used, and a
/// size that cannot be read sets nothing.
#[test]
fn a_bad_share_or_size_never_sets_a_zero_limit() {
    let limit85 = 1_000_000u64 * 4096 / 100 * 85;
    for bad in ["abc", "", "0", "08", "-5", "101", "1e2", "50%", " "] {
        let d = scratch("share");
        let path = fakes(
            &d,
            &[
                ("findmnt", FINDMNT_HOME),
                ("stat", "echo '1000000 4096'"),
                ("btrfs", ""),
            ],
        );
        let o = Command::new("sh")
            .arg(siblings::root().join("files/usr/lib/wolos/home-quota"))
            .env("PATH", &path)
            .env("HOME_QUOTA_PERCENT", bad)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{bad:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let l = log(&d);
        assert!(
            l.contains(&format!("btrfs qgroup limit {limit85} /home")),
            "{bad:?} falls back to 85 %: {l}"
        );
    }
    // A valid other share is honoured.
    let d = scratch("share-50");
    let path = fakes(
        &d,
        &[
            ("findmnt", FINDMNT_HOME),
            ("stat", "echo '1000000 4096'"),
            ("btrfs", ""),
        ],
    );
    let o = Command::new("sh")
        .arg(siblings::root().join("files/usr/lib/wolos/home-quota"))
        .env("PATH", &path)
        .env("HOME_QUOTA_PERCENT", "50")
        .output()
        .unwrap();
    assert!(o.status.success());
    assert!(
        log(&d).contains(&format!(
            "btrfs qgroup limit {} /home",
            1_000_000u64 * 4096 / 100 * 50
        )),
        "{}",
        log(&d)
    );
    // No blocks, no block size, or garbage from stat: nothing is set, still exit 0.
    for stat in [
        "echo '0 4096'",
        "echo '1000000 0'",
        "echo ''",
        "echo 'x y'",
        "exit 1",
    ] {
        let d = scratch("size");
        let path = fakes(
            &d,
            &[("findmnt", FINDMNT_HOME), ("stat", stat), ("btrfs", "")],
        );
        let o = run(&d, &path, &[]);
        assert!(o.status.success(), "{stat}");
        assert!(!log(&d).contains("btrfs"), "{stat}: {}", log(&d));
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("cannot read the size"),
            "{stat}"
        );
    }
}

/// The installer caps the new /home right after mounting it, before the system is copied in and
/// so before any account exists, and only when /home is the pool's own subvolume.
#[test]
fn the_installer_caps_the_new_home_before_any_account() {
    let s = read("files/usr/lib/wolos/install-run");
    let mounted = s.find("subvol=@home,$O").expect("@home is mounted");
    let capped = s
        .find("[ -n \"$home_dev\" ] || /usr/lib/wolos/home-quota \"$T/home\"")
        .expect("install-run caps /home");
    let copied = s
        .find("unsquashfs -f -d \"$T\"")
        .expect("the system is copied");
    assert!(mounted < capped && capped < copied);
    let line = &s[capped..s[capped..].find('\n').map_or(s.len(), |e| capped + e)];
    assert!(line.ends_with("|| true"), "never stops an install: {line}");
}

/// The journal never competes with the account's files for the pool.
#[test]
fn the_journal_is_capped() {
    let c = read("files/usr/lib/systemd/journald.conf.d/wolos.conf");
    assert!(c.lines().any(|l| l == "[Journal]"));
    assert!(c.lines().any(|l| l == "SystemMaxUse=512M"), "{c}");
    assert!(read("docs/adr/0023-installer-plans.md").contains("QA-LIVE-17"));
}
