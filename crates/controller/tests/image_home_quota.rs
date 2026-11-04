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
            ("findmnt", "echo btrfs"),
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
            ("findmnt", "echo btrfs"),
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
            ("findmnt", "echo btrfs"),
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
