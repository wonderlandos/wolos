// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-14, the pin half (docs/STANDARDS.md "Origins of the 0.13 bugs"): a pin is
//! worth what the ref behind it is worth. The repositories were rewritten in 0.13; the commits
//! the old pins named stayed on a disk but no branch reached them, a host serves a commit by sha
//! only while a ref reaches it, and a publish run failed at its first step (f7605d99). RH-142
//! reads the submodule pins of this repository (`git ls-tree HEAD`: `crates/shell`, `design`) and
//! wants each to be a commit that a remote branch or tag of that submodule contains. The same
//! rule for the pins in the recipes is RP-057 (wolos-pkgbuilds) and for the pinned workflows
//! RC-071 (wolos-build). Without a git checkout, or with the submodule not checked out, nothing
//! is checked (it says so).
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, assert_ratchet, hit};
use std::path::Path;
use std::process::Command;

/// No pin of this tree is allowed to be loose.
const BASELINE: &[Row] = &[];

const IDS: &[&str] = &["RH-142"];

fn git(dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
}

/// (path, sha) of every submodule pin recorded in `HEAD` of the checkout at `dir`; `None` when
/// `dir` is no git work tree.
fn pins(dir: &Path) -> Option<Vec<(String, String)>> {
    let top = git(dir, &["rev-parse", "--is-inside-work-tree"])?;
    if !top.status.success() {
        return None;
    }
    let out = git(dir, &["ls-tree", "-r", "HEAD"])?;
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let (meta, path) = l.split_once('\t')?;
                let mut f = meta.split_whitespace();
                (f.next()? == "160000")
                    .then(|| (path.to_owned(), f.nth(1).unwrap_or("").to_owned()))
            })
            .collect(),
    )
}

/// Whether a remote branch or tag of the git directory `dir` contains `sha`; `None` when `dir`
/// holds no checkout of it.
fn reachable(dir: &Path, sha: &str) -> Option<bool> {
    let top = git(dir, &["rev-parse", "--is-inside-work-tree"])?;
    if !top.status.success() {
        return None;
    }
    if !git(dir, &["cat-file", "-e", &format!("{sha}^{{commit}}")])?
        .status
        .success()
    {
        return Some(false);
    }
    let b = git(dir, &["branch", "-r", "--contains", sha])?;
    let t = git(dir, &["tag", "--contains", sha])?;
    Some(!b.stdout.is_empty() || !t.stdout.is_empty())
}

#[test]
fn rh_142_a_submodule_pin_is_a_commit_a_ref_still_reaches() {
    let root = rc::root();
    let mut h = Hits::new();
    let Some(pins) = pins(&root) else {
        println!(
            "RH-142: {} is no git work tree: not checked",
            root.display()
        );
        return;
    };
    let mut checked = 0;
    for (path, sha) in pins {
        match reachable(&root.join(&path), &sha) {
            Some(true) => checked += 1,
            Some(false) => hit(
                &mut h,
                &path,
                format!("{path} pinned at {sha}: no remote branch or tag reaches it"),
            ),
            None => println!("RH-142: {path} is not checked out: not checked"),
        }
    }
    assert!(
        checked > 0 || !h.is_empty() || std::env::var_os("CI").is_none(),
        "no submodule pin could be read"
    );
    assert_ratchet(
        "RH-142",
        "a pin is reachable from a ref of its repository",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_14_pin_reader_reads_what_it_says() {
    let d = std::env::temp_dir().join(format!("rh142-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let g = |args: &[&str]| {
        let o = Command::new("git")
            .arg("-C")
            .arg(&d)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_owned()
    };
    g(&["init", "-q"]);
    g(&["commit", "-q", "--allow-empty", "-m", "one"]);
    let kept = g(&["rev-parse", "HEAD"]);
    g(&["update-ref", "refs/remotes/origin/main", &kept]);
    let tree = g(&["rev-parse", "HEAD^{tree}"]);
    let lost = g(&["commit-tree", &tree, "-m", "orphan"]);
    assert_eq!(reachable(&d, &kept), Some(true), "a remote branch holds it");
    assert_eq!(
        reachable(&d, &lost),
        Some(false),
        "a loose commit: a rewritten history"
    );
    assert_eq!(
        reachable(&d, &"0".repeat(40)),
        Some(false),
        "a commit the repository lacks"
    );
    assert_eq!(
        reachable(&d.join("absent"), &kept),
        None,
        "no checkout: not checked"
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_14_submodule_pins.rs");
    rc::assert_doc_agrees("rules_rc_14_submodule_pins.rs", me, IDS, BASELINE);
}
