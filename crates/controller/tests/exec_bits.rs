// SPDX-License-Identifier: GPL-2.0-or-later
//! Scripts keep their executable bit (audit5 X6-3). The repos are checked out with
//! core.filemode=false, so git never sees a new script's +x and a plain `git add` commits it as
//! 100644; a Linux checkout then cannot run it ("permission denied", or a check that quietly turns
//! a feature off). tools/exec-bits.txt lists every file of this tree (with the crates/shell and
//! design submodules) whose first line is a `#!` shebang. These tests keep that list complete and
//! check that the git index holds each listed file as 100755. Files git does not track yet cannot
//! be checked here: `git add --chmod=+x` them (the header of tools/exec-bits.txt says so).
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The git repositories of this tree: the root, then the submodules (path prefix).
const REPOS: [&str; 3] = ["", "crates/shell", "design"];
/// Directories whose shebang files are data, never run (scanner fixtures).
const NOT_RUN: [&str; 1] = ["crates/aurlaws/tests/fixtures/"];
/// Tracked as 100644 before the list existed; the commit that adds this test sets their bit
/// (`git update-index --chmod=+x`). Only shrinks: an entry that is 100755 is reported for removal.
const PENDING_CHMOD: &[&str] = &[
    "design/tools/apps/kvantum-vm.sh",
    "design/tools/perf/idle.py",
    "design/tools/perf/latency.py",
    "design/tools/perf/leak.py",
    "design/tools/perf/session_start.py",
    "files/usr/lib/wolos/ai-local",
    "files/usr/lib/wolos/modules-enable-ai-claude",
    "files/usr/lib/wolos/modules-enable-dev",
    "tools/check-adrs.sh",
    "tools/check-headers.sh",
    "tools/ci/fuzz-replay.sh",
    "tools/ci/pin-lint.py",
    "tools/license/relicense.py",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The paths tools/exec-bits.txt lists, in file order.
fn listed() -> Vec<String> {
    let t =
        std::fs::read_to_string(root().join("tools/exec-bits.txt")).expect("tools/exec-bits.txt");
    t.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Every file git tracks or does not ignore, repo-relative, with its index mode (None: untracked).
/// None when git or a repository is missing (a source tarball): the callers fall back or skip.
fn git_files() -> Option<BTreeMap<String, Option<String>>> {
    let mut m = BTreeMap::new();
    for r in REPOS {
        let dir = root().join(r);
        if !dir.join(".git").exists() {
            return None;
        }
        let prefix = if r.is_empty() {
            String::new()
        } else {
            format!("{r}/")
        };
        for e in git(&dir, &["ls-files", "-s", "-z"])?
            .split('\0')
            .filter(|e| !e.is_empty())
        {
            let (meta, path) = e.split_once('\t')?;
            let mode = meta.split(' ').next()?;
            if mode != "160000" {
                m.insert(format!("{prefix}{path}"), Some(mode.to_string()));
            }
        }
        for p in git(&dir, &["ls-files", "-o", "--exclude-standard", "-z"])?
            .split('\0')
            .filter(|p| !p.is_empty())
        {
            m.entry(format!("{prefix}{p}")).or_insert(None);
        }
    }
    Some(m)
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if matches!(n.as_str(), "target" | ".git" | "node_modules" | ".work") {
            continue;
        }
        if p.is_dir() {
            walk(&p, base, out);
        } else if let Ok(r) = p.strip_prefix(base) {
            out.push(r.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// `#!/...` or `#! ...` (a Rust `#![attr]` is not a shebang).
fn shebang(rel: &str) -> bool {
    use std::io::Read;
    let mut b = [0u8; 3];
    std::fs::File::open(root().join(rel))
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok()
        && (&b == b"#!/" || &b == b"#! ")
}

/// Every shebang file of the tree, outside NOT_RUN.
fn scripts() -> BTreeSet<String> {
    let files: Vec<String> = match git_files() {
        Some(m) => m.into_keys().collect(),
        None => {
            let mut v = Vec::new();
            walk(&root(), &root(), &mut v);
            v
        }
    };
    files
        .into_iter()
        .filter(|f| !NOT_RUN.iter().any(|d| f.starts_with(d)))
        .filter(|f| root().join(f).is_file() && shebang(f))
        .collect()
}

#[test]
fn the_list_names_every_script_and_nothing_else() {
    let listed: BTreeSet<String> = listed().into_iter().collect();
    let found = scripts();
    assert!(found.len() > 40, "only {} scripts found", found.len());
    let missing: Vec<&String> = found.difference(&listed).collect();
    let stale: Vec<&String> = listed.difference(&found).collect();
    assert!(
        missing.is_empty(),
        "shebang files missing from tools/exec-bits.txt (add each, commit it with `git add --chmod=+x`):\n{}",
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        stale.is_empty(),
        "tools/exec-bits.txt lists files that are gone or have no shebang:\n{}",
        stale
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_list_is_sorted_and_unique() {
    let l = listed();
    let mut s = l.clone();
    s.sort();
    s.dedup();
    assert_eq!(
        l, s,
        "tools/exec-bits.txt: keep it sorted, without duplicates"
    );
    let mut p = PENDING_CHMOD.to_vec();
    p.sort_unstable();
    p.dedup();
    assert_eq!(
        p, PENDING_CHMOD,
        "PENDING_CHMOD: sorted, without duplicates"
    );
    for f in PENDING_CHMOD {
        assert!(
            l.iter().any(|x| x == f),
            "PENDING_CHMOD names {f}, which the list does not"
        );
    }
}

#[test]
fn listed_scripts_are_committed_executable() {
    let Some(index) = git_files() else {
        eprintln!("no git checkout here; index modes not checked");
        return;
    };
    let mut bad = Vec::new();
    let mut untracked = Vec::new();
    for f in listed() {
        match index.get(&f) {
            Some(Some(mode)) if mode == "100755" => {
                if PENDING_CHMOD.contains(&f.as_str()) {
                    eprintln!("exec bits: {f} is 100755 now; remove it from PENDING_CHMOD");
                }
            }
            Some(Some(mode)) if !PENDING_CHMOD.contains(&f.as_str()) => {
                let repo = REPOS[1..].iter().find(|r| f.starts_with(&format!("{r}/")));
                let (dir, rel) = match repo {
                    Some(r) => (*r, &f[r.len() + 1..]),
                    None => (".", f.as_str()),
                };
                bad.push(format!(
                    "{f} ({mode}): git -C {dir} update-index --chmod=+x {rel}"
                ));
            }
            Some(None) => untracked.push(f),
            _ => {}
        }
    }
    if !untracked.is_empty() {
        eprintln!(
            "exec bits: not tracked yet, add with `git add --chmod=+x`:\n{}",
            untracked.join("\n")
        );
    }
    assert!(
        bad.is_empty(),
        "scripts the index holds without the executable bit (from the wolos root):\n{}",
        bad.join("\n")
    );
}
