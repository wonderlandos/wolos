// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-14 (ADR 0051, docs/STANDARDS.md section 9): the integration branch
//! descends from the last release and from every merged wave, and a decision record's status
//! follows the tree, not a worker's report. RH-141 counts repo paths named in backticks by an
//! accepted or implemented decision record that do not exist in the tree. The ancestry half
//! (`git merge-base --is-ancestor <last release tag> HEAD`) is
//! `crates/controller/tests/image_release.rs::int_descends_from_last_release` (fix plan step 0.3).
//! Counts are frozen per record in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, assert_ratchet, hit, repo_path, spans};

/// Drift at the start of 0.13 (2026-11-04, mid-merge tree), per record. Only shrinks.
const BASELINE: &[Row] = &[
    ("RH-141", "docs/adr/0016-arch-linux-base.md", 2, "D7"),
    (
        "RH-141",
        "docs/adr/0029-repository-split-and-single-build-path.md",
        2,
        "D7",
    ),
    ("RH-141", "docs/adr/0039-libwolkit-and-wolkit.md", 1, "D7"),
];

const IDS: &[&str] = &["RH-141"];

/// True when the `Status:` line says the record is in force (accepted or implemented).
fn in_force(text: &str) -> bool {
    let Some(st) = text.lines().find_map(|l| l.strip_prefix("Status:")) else {
        return false;
    };
    let st = st.trim().to_lowercase();
    (st.starts_with("accepted") || st.starts_with("implemented"))
        && !st.contains("superseded by")
        && !st.contains("historical")
}

#[test]
fn rh_141_records_in_force_name_paths_that_exist() {
    let base = rc::root();
    let mut h = Hits::new();
    let mut read = 0;
    for d in rc::walk("docs/adr", &["md"]) {
        let text = rc::read(&d);
        if !in_force(&text) {
            continue;
        }
        read += 1;
        for (n, sp) in spans(&text) {
            if let Some(p) = repo_path(&sp)
                && !rc::resolves(&base, &p)
            {
                hit(&mut h, &d, format!("{d}:{n} `{p}` is not in the tree"));
            }
        }
    }
    assert!(read >= 10, "only {read} decision records read as in force");
    assert_ratchet(
        "RH-141",
        "an accepted record names only what the tree has",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_14_status_reader_reads_what_it_says() {
    assert!(in_force("# 1 - X\nStatus: accepted. Builds on 0016.\n"));
    assert!(in_force("# 1 - X\nStatus: implemented (v0.12)\n"));
    assert!(!in_force("# 1 - X\nStatus: proposed\n"));
    assert!(!in_force("# 1 - X\nStatus: accepted; superseded by 0040\n"));
    assert!(!in_force("# 1 - X\nno status\n"));
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_14_release_ancestry.rs");
    rc::assert_doc_agrees("rules_rc_14_release_ancestry.rs", me, IDS, BASELINE);
}
