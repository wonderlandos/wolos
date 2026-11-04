// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-16 (ADR 0051, docs/STANDARDS.md section 9): a document names only what
//! the tree has. RH-161 counts repo paths in backticks (`crates/...`, `files/...`) in the current
//! docs that do not exist; RH-162 counts `wol <verb>` spans whose verb is not in the command tree
//! (`crates/controller/src/cli.rs` `TREE`). Decision records are RH-141's (rules_rc_14). The
//! on-screen names check (manual control names come from the copy catalog) is the site's
//! `check-manual` gate (ADR 0051, brief C8). Counts are frozen per document in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, assert_ratchet, hit, repo_path, spans};
use std::collections::BTreeSet;

/// Drift at the start of 0.13 (2026-11-04, mid-merge tree), per document. Only shrinks.
const BASELINE: &[Row] = &[
    ("RH-161", "docs/REVIEW.md", 1, "D7"),
    ("RH-161", "docs/modules/apps.md", 1, "C8"),
    ("RH-161", "docs/modules/cryswol.md", 2, "C8"),
    ("RH-161", "docs/modules/gallery.md", 1, "C8"),
    ("RH-161", "docs/modules/intel-graphics.md", 2, "C8"),
    ("RH-162", "docs/modules/memory-bandwidth.md", 1, "C8"),
    ("RH-162", "docs/modules/modules.md", 1, "C8"),
    ("RH-162", "docs/modules/scripting.md", 1, "C8"),
    ("RH-162", "docs/research/attested-gaming-mode.md", 1, "C8"),
];

const IDS: &[&str] = &["RH-161", "RH-162"];

/// The current documents: README, BUILD and docs/**/*.md except decision records.
fn current_docs() -> Vec<String> {
    let mut v = vec!["README.md".to_owned(), "BUILD.md".to_owned()];
    v.extend(
        rc::walk("docs", &["md"])
            .into_iter()
            .filter(|d| !d.starts_with("docs/adr/")),
    );
    v
}

/// The first words of the `wol` command tree.
fn verbs() -> BTreeSet<String> {
    let cli = rc::read("crates/controller/src/cli.rs");
    let tree = &cli[cli.find("pub const TREE").expect("cli.rs has TREE")..];
    let mut v = BTreeSet::new();
    for head in ["cmd(", "alias(", "path:"] {
        for (p, _) in tree.match_indices(head) {
            let rest = tree[p + head.len()..].trim_start();
            if let Some(r) = rest.strip_prefix("&[\"") {
                v.insert(r.split('"').next().unwrap_or("").to_owned());
            }
        }
    }
    v
}

/// The verb of a `wol <verb> ...` span, when the span is a command line.
fn verb_of(span: &str) -> Option<&str> {
    let w = span
        .trim()
        .strip_prefix("wol ")?
        .split_whitespace()
        .next()?;
    let word = w.starts_with(|c: char| c.is_ascii_lowercase());
    (word && w.chars().all(|c| c.is_ascii_lowercase() || c == '-')).then_some(w)
}

#[test]
fn rh_161_docs_name_only_paths_that_exist() {
    let base = rc::root();
    let mut h = Hits::new();
    for d in current_docs() {
        for (n, sp) in spans(&rc::read(&d)) {
            if let Some(p) = repo_path(&sp)
                && !rc::resolves(&base, &p)
            {
                hit(&mut h, &d, format!("{d}:{n} `{p}` is not in the tree"));
            }
        }
    }
    assert_ratchet("RH-161", "a path in a document exists", BASELINE, &h);
}

#[test]
fn rh_162_docs_name_only_verbs_the_cli_has() {
    let known = verbs();
    assert!(
        known.len() > 20,
        "the command tree reader found {} verbs",
        known.len()
    );
    let mut h = Hits::new();
    for d in current_docs() {
        for (n, sp) in spans(&rc::read(&d)) {
            if let Some(v) = verb_of(&sp)
                && !known.contains(v)
            {
                hit(&mut h, &d, format!("{d}:{n} `wol {v}` is not a verb"));
            }
        }
    }
    assert_ratchet(
        "RH-162",
        "a verb in a document is in the command tree",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_16_readers_read_what_they_say() {
    assert_eq!(
        spans("a `crates/x.rs:12` b `wol about`\n```\n`crates/no`\n```\nc `odd"),
        [
            (1, "crates/x.rs:12".to_owned()),
            (1, "wol about".to_owned())
        ]
    );
    assert_eq!(
        repo_path("crates/x/src/a.rs:10-20").as_deref(),
        Some("crates/x/src/a.rs")
    );
    assert_eq!(
        repo_path("crates/x/src/a.rs::f").as_deref(),
        Some("crates/x/src/a.rs")
    );
    assert_eq!(repo_path("crates/*/src"), None);
    assert_eq!(repo_path("/usr/bin/wol"), None);
    assert_eq!(repo_path("desktop/overlay/.../applications"), None);
    assert!(
        rc::resolves(&rc::root(), "docs/adr/0016"),
        "a record named by its number"
    );
    assert!(!rc::resolves(&rc::root(), "docs/adr/9999"));
    assert_eq!(verb_of("wol update --json"), Some("update"));
    assert_eq!(verb_of("wol --help"), None);
    assert!(verbs().contains("about"));
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_16_docs_name_the_tree.rs");
    rc::assert_doc_agrees("rules_rc_16_docs_name_the_tree.rs", me, IDS, BASELINE);
}
