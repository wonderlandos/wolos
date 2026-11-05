// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-29 (docs/STANDARDS.md section 9 and "Origins of the 0.13 bugs"): a state
//! file is replaced, never rewritten in place, and by one writer. `shared::atomic::atomic_write`
//! creates the temporary exclusively and without following a link, gives it its mode on the
//! descriptor, flushes it and renames it over the target. Every private copy of that idea is a
//! writer that forgot one of the four steps: the Store's journal tore a line (QA-STORE-21), a
//! plugin index was left half written (QA-PLUG-9), and the first sign-in Setup of 0.13 grew a
//! sixth `write_atomic` of its own. RH-291 finds a function named like an atomic writer
//! (`write_atomic`, `save_atomic`, `atomic_write`, `write_file_atomic`) outside
//! `crates/shared/src/atomic.rs`. Counts are frozen per file in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, find_words, hit};

/// The private writers of the tree when the rule came (2026-11-04, merged tree up to wd-store):
/// each one writes a temporary and renames it, and none of them is `O_EXCL` + `O_NOFOLLOW` +
/// `fsync`. Moving each to `shared::atomic::atomic_write` is the sweep. Only shrinks.
const BASELINE: &[Row] = &[
    ("RH-291", "crates/core/src/journal.rs", 1, "STD-7"),
    ("RH-291", "crates/core/src/looks.rs", 1, "STD-7"),
    ("RH-291", "crates/core/src/plugins.rs", 1, "STD-7"),
    ("RH-291", "crates/shell/src/first_login.rs", 1, "STD-7"),
    ("RH-291", "crates/shell/src/ui/maker/editor.rs", 1, "STD-7"),
    ("RH-291", "crates/wopkg/src/journal.rs", 1, "STD-7"),
];

const IDS: &[&str] = &["RH-291"];

/// The one place the writer lives.
const THE_WRITER: &str = "crates/shared/src/atomic.rs";

/// Names a function takes when it is a private atomic writer.
const NAMES: &[&str] = &[
    "write_atomic",
    "save_atomic",
    "atomic_write",
    "write_file_atomic",
];

fn private_writers(s: &Src, h: &mut Hits) {
    if s.rel == THE_WRITER {
        return;
    }
    for n in NAMES {
        let def = format!("{n}(");
        for p in find_words(&s.code, &def) {
            if s.code[..p].ends_with(b"fn ") {
                hit(
                    h,
                    &s.rel,
                    format!("{} fn {n} is a private atomic writer", s.at(p)),
                );
            }
        }
    }
}

#[test]
fn rh_291_state_files_are_written_by_the_shared_atomic_writer() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates"]) {
        private_writers(&s, &mut h);
    }
    assert_ratchet(
        "RH-291",
        "no private atomic writer beside shared::atomic",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_29_counter_counts_what_it_says() {
    let n = |rel: &str, code: &str| {
        let s = Src::parse(rel, code.to_owned());
        let mut h = Hits::new();
        private_writers(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(
        n("crates/x/src/a.rs", "fn write_atomic(p: &Path, t: &str) {}"),
        1
    );
    assert_eq!(
        n(
            "crates/x/src/a.rs",
            "pub fn save_atomic(p: &Path, t: &str) {}"
        ),
        1
    );
    assert_eq!(
        n(
            "crates/x/src/a.rs",
            "fn f() { shared::atomic::atomic_write(p, 0o644, d)?; }"
        ),
        0,
        "a call is the right thing"
    );
    assert_eq!(
        n(THE_WRITER, "pub fn atomic_write(p: &Path) {}"),
        0,
        "the shared writer itself"
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_29_one_atomic_write.rs");
    rc::assert_doc_agrees("rules_rc_29_one_atomic_write.rs", me, IDS, BASELINE);
}
