// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-20 (ADR 0051, docs/STANDARDS.md section 9): success is not reported on
//! a failure, and a copy of what pacman installed is not a quiet downgrade of it. RH-201 finds
//! `fs::copy(` in the root-side crates (`controller`, `core`, `api`): it carries content and the
//! permission bits and nothing else, and the `chown` after it clears the set-id bits and the file
//! capability, so a program a person installed (chrome-sandbox, fusermount3, `cap_net_raw+ep`
//! tools) came out of `wol layers fold` as a plain 0755 file with the fold exiting 0
//! (QA-A9-CORE-1). Installed files are copied by `controller::pkg::copy_tree` (owner, extended
//! attributes, mode, hard links), or by `cp -a`. The behaviour is the controller test
//! `fold_keeps_set_id_bits_xattrs_and_hard_links`. Counts are frozen per file in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, find_all, hit};

/// No debt: the one `fs::copy` of the tree's installers went with QA-A9-CORE-1. Only shrinks.
const BASELINE: &[Row] = &[];

const IDS: &[&str] = &["RH-201"];

/// Where the root side copies files that end up in a slot, a layer or the image.
const ROOT_SIDE: &[&str] = &["crates/controller/src", "crates/core/src", "crates/api/src"];

fn plain_copies(s: &Src, h: &mut Hits) {
    for p in find_all(&s.code, "fs::copy(") {
        hit(
            h,
            &s.rel,
            format!(
                "{} fs::copy drops the owner's set-id bits, xattrs and hard links",
                s.at(p)
            ),
        );
    }
}

#[test]
fn rh_201_installed_files_are_not_copied_with_fs_copy() {
    let mut h = Hits::new();
    for s in rc::sources(ROOT_SIDE) {
        plain_copies(&s, &mut h);
    }
    assert_ratchet(
        "RH-201",
        "an installed file is copied with its metadata (copy_tree, cp -a), not with fs::copy",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_20_counter_counts_what_it_says() {
    let n = |code: &str| {
        let s = Src::parse("crates/controller/src/pkg.rs", code.to_owned());
        let mut h = Hits::new();
        plain_copies(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(n("std::fs::copy(&src, &dst)?;"), 1);
    assert_eq!(n("fs::copy(a, b).ok();"), 1);
    assert_eq!(n("// fs::copy(a, b) in a comment"), 0);
    assert_eq!(n("let s = \"fs::copy(a, b)\";"), 0);
    assert_eq!(
        n("#[cfg(test)] mod t { fn f() { std::fs::copy(a, b); } }"),
        0
    );
    assert_eq!(n("std::io::copy(&mut from, &mut out)?;"), 0);
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_20_installed_metadata.rs");
    rc::assert_doc_agrees("rules_rc_20_installed_metadata.rs", me, IDS, BASELINE);
}
