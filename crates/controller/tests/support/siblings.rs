// SPDX-License-Identifier: GPL-2.0-or-later
//! WolOS lives in three source repos: wolos (code), wolos-pkgbuilds (recipes), wolos-iso (image).
//! Contract tests read files by their repo path, whichever repo holds them. `root()` is one
//! directory with the three trees overlaid (symlinks; the repo running the tests wins a clash,
//! directories present in several are merged). Siblings: $WOLOS_SRC, $WOLOS_PKGBUILDS, $WOLOS_ISO,
//! else ../wolos, ../wolos-pkgbuilds, ../wolos-iso next to this repo (CI checks them out there).
//! A missing sibling is not an error here: the test that needs its file fails naming the path.
//!
//! One source, three byte-identical copies (the repos are separate git repos and stay
//! self-contained): this file is the original; `wolos-iso/checks/tests/support/siblings.rs` and
//! `wolos-pkgbuilds/checks/tests/support/siblings.rs` are verbatim copies, and each of those crates'
//! `tests/siblings_copy.rs` fails when its copy differs from this one. Edit here, then copy.
#![allow(dead_code)]
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// This repo's root: the nearest ancestor of the including crate that holds `LICENSE` (every repo
/// root does, no crate directory does), so the same file works at any crate depth.
fn repo_root() -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    crate_dir
        .ancestors()
        .find(|d| d.join("LICENSE").is_file())
        .unwrap_or(&crate_dir)
        .to_path_buf()
}

fn overlay(src: &Path, dst: &Path) {
    let Ok(entries) = fs::read_dir(src) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        if name == ".git" || name == "target" {
            continue;
        }
        let to = dst.join(&name);
        match fs::symlink_metadata(&to) {
            Err(_) => std::os::unix::fs::symlink(e.path(), &to).unwrap(),
            // Both have this directory: a real one with the entries of each.
            Ok(m) if m.file_type().is_symlink() && to.is_dir() && e.path().is_dir() => {
                let first = fs::read_link(&to).unwrap();
                fs::remove_file(&to).unwrap();
                fs::create_dir(&to).unwrap();
                overlay(&first, &to);
                overlay(&e.path(), &to);
            }
            Ok(m) if m.is_dir() && e.path().is_dir() => overlay(&e.path(), &to),
            Ok(_) => {} // a file: the earlier repo keeps it
        }
    }
}

/// The overlaid tree as a string (for `format!("{}/path", root_str())`).
pub fn root_str() -> &'static str {
    static ROOT: OnceLock<(TreeDir, String)> = OnceLock::new();
    &ROOT
        .get_or_init(|| {
            let own = repo_root();
            let parent = own.parent().unwrap().to_path_buf();
            sweep_dead_trees(&std::env::temp_dir());
            let u = std::env::temp_dir().join(format!("{TREE_PREFIX}{}", std::process::id()));
            let _ = fs::remove_dir_all(&u);
            fs::create_dir_all(&u).unwrap();
            let guard = TreeDir(u.clone());
            overlay(&own, &u);
            for (var, dir) in [
                ("WOLOS_SRC", "wolos"),
                ("WOLOS_PKGBUILDS", "wolos-pkgbuilds"),
                ("WOLOS_ISO", "wolos-iso"),
            ] {
                let r = std::env::var_os(var).map_or_else(|| parent.join(dir), PathBuf::from);
                match r.canonicalize() {
                    Ok(r) if r != own => overlay(&r, &u),
                    _ => {}
                }
            }
            let text = u.to_string_lossy().into_owned();
            (guard, text)
        })
        .1
}

/// Name of a process's overlaid tree in the temp dir: the prefix, then the process id.
pub const TREE_PREFIX: &str = "wolos-tree-";

/// The overlaid tree (symlinks and directories only), removed when dropped. A `static` is never
/// dropped, so [`root_str`]'s copy outlives its process: [`sweep_dead_trees`] removes those.
pub struct TreeDir(pub PathBuf);

// Removes the symlink tree; the files it points at are the repos' own and are never touched.
impl Drop for TreeDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Removes every `wolos-tree-<pid>` in `tmp` whose process is gone (no `/proc/<pid>`): the copies
/// test processes left (3126 of them filled the forge's /tmp inodes on 2026-11-04).
pub fn sweep_dead_trees(tmp: &Path) {
    let Ok(entries) = fs::read_dir(tmp) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.strip_prefix(TREE_PREFIX) else {
            continue;
        };
        if pid.parse::<u32>().is_ok() && !Path::new("/proc").join(pid).exists() {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

pub fn root() -> PathBuf {
    PathBuf::from(root_str())
}
