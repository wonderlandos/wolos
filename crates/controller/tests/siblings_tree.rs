// SPDX-License-Identifier: GPL-2.0-or-later
//! The overlaid sibling tree (`support/siblings.rs`): a copy a finished test process left in the
//! temp dir is removed by the next one, and the guard removes its own tree when dropped.
#[path = "support/siblings.rs"]
mod siblings;

/// One test in this binary, so `root()` runs here for the first time and its sweep is seen.
#[test]
fn a_finished_process_tree_is_swept_and_a_guard_removes_its_own() {
    let tmp = std::env::temp_dir();
    // No process has pid 4194303 + 1 on Linux (pid_max is at most 4194304).
    let stale = tmp.join(format!("{}4194305", siblings::TREE_PREFIX));
    std::fs::create_dir_all(stale.join("crates")).unwrap();
    let live = tmp.join(format!("{}{}", siblings::TREE_PREFIX, std::process::id()));
    let root = siblings::root();
    assert!(
        !stale.exists(),
        "a dead process's tree stays: {}",
        stale.display()
    );
    assert_eq!(root, live);
    assert!(root.join("crates").exists());
    let other = tmp.join(format!(
        "{}guard-{}",
        siblings::TREE_PREFIX,
        std::process::id()
    ));
    std::fs::create_dir_all(&other).unwrap();
    drop(siblings::TreeDir(other.clone()));
    assert!(!other.exists(), "the guard removes its tree");
}
