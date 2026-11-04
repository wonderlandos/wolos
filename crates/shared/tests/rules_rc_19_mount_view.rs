// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-19 (ADR 0051, docs/STANDARDS.md section 9): code that reads or writes
//! system paths says which view it needs and gets it from one helper; mounts for a transaction
//! are private. RH-191 counts root-side plans that start the package manager by its bare name or
//! path instead of the one named constant; RH-192 counts files that build `mount` steps with no
//! private mount namespace (`unshare`), so the mounts leak into every service. The sealed-slot
//! journeys (`qa100-store-pkg-paths`, `qa100-data-verity-after-install`) are the live half
//! (briefs B4, B5). Counts are frozen per file in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, hit};

/// Debt at the start of 0.13 (2026-11-04, mid-merge tree), per file. Only shrinks.
const BASELINE: &[Row] = &[
    ("RH-192", "crates/core/src/update.rs", 1, "B4"),
    ("RH-192", "crates/core/src/update/layers.rs", 3, "B5"),
];

const IDS: &[&str] = &["RH-191", "RH-192"];

/// Where root-side plans are built.
const ROOT_SIDE: &[&str] = &[
    "crates/core/src/admin.rs",
    "crates/core/src/update",
    "crates/api/src",
];

/// The package manager's bare names.
const BARE: &[&str] = &["\"pacman\"", "\"/usr/bin/pacman\""];

fn root_side() -> Vec<Src> {
    let mut v = rc::sources(&ROOT_SIDE[1..]);
    v.push(Src::load(ROOT_SIDE[0]));
    v.push(Src::load("crates/core/src/update.rs"));
    v
}

fn bare_manager(s: &Src, h: &mut Hits) {
    for (off, lit) in &s.lits {
        if !BARE.contains(&lit.as_str()) {
            continue;
        }
        let line = s.line(s.line_of(*off)).trim_start();
        let named = line.starts_with("pub const ") || line.starts_with("const ");
        let compared = line.contains("==") || line.contains("!=");
        if !named && !compared {
            hit(h, &s.rel, format!("{} {lit}", s.at(*off)));
        }
    }
}

fn shared_mounts(s: &Src, h: &mut Hits) {
    let private = s.lits.iter().any(|(_, l)| l == "\"unshare\"");
    if private {
        return;
    }
    for (off, lit) in &s.lits {
        if lit == "\"mount\"" {
            let before = s.text[..*off].trim_end();
            if before.ends_with('[') {
                hit(
                    h,
                    &s.rel,
                    format!("{} mount step outside a private namespace", s.at(*off)),
                );
            }
        }
    }
}

#[test]
fn rh_191_root_plans_name_the_package_manager_once() {
    let mut h = Hits::new();
    for s in root_side() {
        bare_manager(&s, &mut h);
    }
    assert_ratchet(
        "RH-191",
        "the package manager is started through its one constant",
        BASELINE,
        &h,
    );
}

#[test]
fn rh_192_transaction_mounts_are_private() {
    let mut h = Hits::new();
    for s in root_side() {
        shared_mounts(&s, &mut h);
    }
    assert_ratchet(
        "RH-192",
        "a transaction mounts in a private namespace",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_19_counters_count_what_they_say() {
    let n = |code: &str| {
        let s = Src::parse("crates/core/src/admin.rs", code.to_owned());
        let mut h = Hits::new();
        bare_manager(&s, &mut h);
        shared_mounts(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(n("vec![argv(&[\"pacman\", \"-Sy\"])]"), 1);
    assert_eq!(n("pub const PACMAN: &str = \"/usr/bin/pacman\";"), 0);
    assert_eq!(n("if argv[0] == \"pacman\" {}"), 0);
    assert_eq!(n("v(&[\"mount\", \"--rbind\", \"/\", CHROOT])"), 1);
    assert_eq!(
        n("v(&[\"unshare\", \"-m\"]); v(&[\"mount\", \"--rbind\", \"/\", CHROOT])"),
        0
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_19_mount_view.rs");
    rc::assert_doc_agrees("rules_rc_19_mount_view.rs", me, IDS, BASELINE);
}
