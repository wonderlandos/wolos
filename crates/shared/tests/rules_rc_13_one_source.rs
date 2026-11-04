// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-13 (ADR 0051, docs/STANDARDS.md section 9): every shared fact has
//! exactly one definition that every reader imports. RH-131 is the `single_source_facts` table:
//! for each fact, where it may be defined (its owner) and how a definition is recognised; the
//! count of definitions in shipped code is frozen in [`BASELINE`] and falls to the fact's target
//! (one, or zero where the fact is read from a shipped file at run time).
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, find_words, hit};

/// Twins at the start of 0.13 (2026-11-04, mid-merge tree), per fact. Only shrinks.
const BASELINE: &[Row] = &[
    ("RH-131", "language-list", 1, "A1"),
    ("RH-131", "shown-version", 5, "A4"),
    ("RH-131", "web-message-cap", 1, "C4"),
];

const IDS: &[&str] = &["RH-131"];

/// How a definition of a fact shows up in shipped code.
enum Seen {
    /// This text in the blanked code (an item definition).
    Code(&'static str),
    /// A string literal containing this text.
    Lit(&'static str),
}

/// (fact, how it is seen, owner file that must keep the one definition, target count).
/// An owner of `None` means the fact is read at run time and code holds no copy (target 0).
const FACTS: &[(&str, Seen, Option<&str>, usize)] = &[
    (
        "web-message-cap",
        Seen::Code("const MAX_MESSAGE"),
        Some("crates/webengine/src/bridge.rs"),
        1,
    ),
    (
        "language-list",
        Seen::Code("const LANGUAGES"),
        Some("crates/core/src/region.rs"),
        1,
    ),
    (
        "update-lock-path",
        Seen::Lit("update.lock"),
        Some("crates/core/src/update.rs"),
        1,
    ),
    // The version a person reads comes from the release file of the image, not the crate.
    ("shown-version", Seen::Lit("CARGO_PKG_VERSION"), None, 0),
    (
        "package-manager-path",
        Seen::Lit("/usr/bin/pacman"),
        Some("crates/core/src/update.rs"),
        1,
    ),
];

/// Where each fact may be seen at all (the web message cap is the engine's, not the portal's).
fn in_scope(fact: &str, rel: &str) -> bool {
    match fact {
        "web-message-cap" => {
            rel.starts_with("crates/webengine/") || rel.starts_with("crates/controller/")
        }
        "shown-version" => rel.starts_with("crates/shell/"),
        _ => true,
    }
}

fn definitions(s: &Src, h: &mut Hits) {
    for (fact, seen, _, _) in FACTS {
        if !in_scope(fact, &s.rel) {
            continue;
        }
        match seen {
            Seen::Code(t) => {
                for p in find_words(&s.code, t) {
                    hit(h, fact, s.at(p));
                }
            }
            Seen::Lit(t) => {
                for (off, l) in &s.lits {
                    if l.contains(t) {
                        hit(h, fact, s.at(*off));
                    }
                }
            }
        }
    }
}

#[test]
fn rh_131_single_source_facts() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates"]) {
        definitions(&s, &mut h);
    }
    for (fact, _, owner, _) in FACTS {
        let seen = h.get(*fact).map_or(&[][..], Vec::as_slice);
        if let Some(o) = owner {
            assert!(
                seen.iter().any(|w| w.starts_with(&format!("{o}:"))),
                "RH-131 {fact}: the owner {o} no longer defines it (seen at {seen:?})"
            );
        }
    }
    // The owner's own definition is the fact; every other one is a twin.
    let mut twins = Hits::new();
    for (fact, _, owner, target) in FACTS {
        let mut kept = 0;
        for w in h.get(*fact).into_iter().flatten() {
            let own = owner.is_some_and(|o| w.starts_with(&format!("{o}:")));
            if own && kept < *target {
                kept += 1;
            } else {
                hit(&mut twins, fact, w.clone());
            }
        }
    }
    assert_ratchet("RH-131", "one fact, one definition", BASELINE, &twins);
}

#[test]
fn rc_13_counters_count_what_they_say() {
    let s = Src::parse(
        "crates/webengine/src/x.rs",
        "pub const MAX_MESSAGE: usize = 1;\nconst LANGUAGES: [u8; 0] = [];\n// const LANGUAGES\nlet p = \"/run/wolos/update.lock\";".to_owned(),
    );
    let mut h = Hits::new();
    definitions(&s, &mut h);
    assert_eq!(h.get("web-message-cap").map(Vec::len), Some(1));
    assert_eq!(
        h.get("language-list").map(Vec::len),
        Some(1),
        "comments do not count"
    );
    assert_eq!(h.get("update-lock-path").map(Vec::len), Some(1));
    assert!(!h.contains_key("shown-version"));
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_13_one_source.rs");
    rc::assert_doc_agrees("rules_rc_13_one_source.rs", me, IDS, BASELINE);
}
