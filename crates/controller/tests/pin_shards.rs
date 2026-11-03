// SPDX-License-Identifier: GPL-2.0-or-later
//! tools/crystal/pin-shards.sh (wolos-shards/README.md "Shards still in the WolOS tree"): the three
//! files that name the in-tree curated shards' commit agree in the checkout, and the script moves
//! them together on a temporary copy, is idempotent, and refuses a bad commit without touching
//! anything. The script runs with `sh`; nothing in the checkout is written.
use std::path::{Path, PathBuf};
use std::process::Command;

const FILES: [&str; 4] = [
    "wolos-shards/curated.yml",
    "wolos-crystal/shards/wolkit/shard.yml",
    "wolos-crystal/shards/wolkit/shard.lock",
    "wolos-crystal/shards/cryswol/shard.yml",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// (cryswol row, wolkit row, wolkit's cryswol pin, the lock's commit).
fn commits(root: &Path) -> (String, String, String, String) {
    let curated = read(root, FILES[0]);
    let mut row = "";
    let (mut cryswol, mut wolkit) = (String::new(), String::new());
    for l in curated.lines() {
        if let Some(n) = l.strip_prefix("  - name: ") {
            row = n.trim();
        }
        if let Some(c) = l.strip_prefix("    commit: ") {
            match row {
                "cryswol" => cryswol = c.trim().into(),
                "wolkit" => wolkit = c.trim().into(),
                _ => {}
            }
        }
    }
    let yml = read(root, FILES[1]);
    let pin = yml
        .split("\n  cryswol:\n")
        .nth(1)
        .and_then(|b| b.lines().find_map(|l| l.strip_prefix("    commit: ")))
        .unwrap_or_default()
        .trim()
        .to_owned();
    let lock = read(root, FILES[2]);
    let locked = lock
        .split("+git.commit.")
        .nth(1)
        .unwrap_or_default()
        .trim()
        .to_owned();
    (cryswol, wolkit, pin, locked)
}

fn copy(name: &str) -> PathBuf {
    let t = std::env::temp_dir().join(format!("pin-shards-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    for f in FILES {
        let to = t.join(f);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(repo().join(f), to).unwrap();
    }
    t
}

fn pin(commit: &str, root: &Path) -> i32 {
    Command::new("sh")
        .arg(repo().join("tools/crystal/pin-shards.sh"))
        .args([commit, &root.to_string_lossy()])
        .output()
        .expect("sh")
        .status
        .code()
        .unwrap_or(-1)
}

#[test]
fn the_checkout_agrees() {
    let (c, w, p, l) = commits(&repo());
    assert_eq!(c.len(), 40, "cryswol row: {c:?}");
    assert_eq!(
        (&w, &p, &l),
        (&c, &c, &c),
        "curated rows, wolkit pin and lock differ"
    );
    // The lock names the pinned URL and cryswol's own version.
    let lock = read(&repo(), FILES[2]);
    let version = read(&repo(), FILES[3])
        .lines()
        .find_map(|l| l.strip_prefix("version: "))
        .unwrap()
        .trim()
        .to_owned();
    assert!(
        lock.contains(&format!("version: {version}+git.commit.{c}")),
        "{lock}"
    );
    assert!(
        lock.contains("git: https://git.wolos.org/shards/cryswol.git"),
        "{lock}"
    );
}

#[test]
fn it_moves_the_three_files_together_and_twice_is_once() {
    let t = copy("move");
    let new = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(pin(new, &t), 0);
    let (c, w, p, l) = commits(&t);
    assert_eq!(
        (c.as_str(), w.as_str(), p.as_str(), l.as_str()),
        (new, new, new, new)
    );
    let first: Vec<String> = FILES.iter().map(|f| read(&t, f)).collect();
    assert_eq!(pin(new, &t), 0);
    let second: Vec<String> = FILES.iter().map(|f| read(&t, f)).collect();
    assert_eq!(first, second, "a second run changed something");
    // Nothing else in the files moved: back at the old commit, they are the checkout's again.
    let (old, ..) = commits(&repo());
    assert_eq!(pin(&old, &t), 0);
    for f in FILES {
        assert_eq!(
            read(&t, f),
            read(&repo(), f),
            "{f} differs after the round trip"
        );
    }
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn a_bad_commit_changes_nothing() {
    let t = copy("bad");
    for bad in [
        "",
        "11cd053",
        "0123456789ABCDEF0123456789abcdef01234567",
        "../../etc/passwd",
    ] {
        assert_eq!(pin(bad, &t), 2, "{bad:?}");
    }
    for f in FILES {
        assert_eq!(read(&t, f), read(&repo(), f), "{f} was touched");
    }
    let _ = std::fs::remove_dir_all(&t);
}
