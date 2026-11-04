// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-07 (ADR 0051, docs/STANDARDS.md section 9): a view is driven by one live
//! model, and a widget the person touched holds its value until the host confirms. RH-071 counts
//! repeating timers in Settings, Control Center and the Store whose body writes a widget
//! directly (`.set_active(`, `.set_value(`): a poll that does so undoes what the person just
//! did. Such a poll updates the model, and the model skips touched widgets. Counts are frozen
//! per file in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, close_of, find_words, hit};

/// Debt at the start of 0.13 (2026-11-04, mid-merge tree), per file. Only shrinks.
const BASELINE: &[Row] = &[];

const IDS: &[&str] = &["RH-071"];

const SURFACES: &[&str] = &[
    "crates/shell/src/ui/settings",
    "crates/shell/src/ui/control_center",
    "crates/shell/src/ui/store",
];
const SURFACE_FILES: &[&str] = &[
    "crates/shell/src/ui/store.rs",
    "crates/shell/src/ui/store_plugins.rs",
];

/// Timers that fire again and again (the `_once` forms are not polls).
const REPEATING: &[&str] = &[
    "timeout_add_local(",
    "timeout_add_seconds_local(",
    "timeout_add(",
    "timeout_add_seconds(",
    "every_minute_while(",
];

/// Writes into a widget the person may be touching.
const SETTERS: &[&str] = &[
    ".set_active(",
    ".set_value(",
    ".set_selected(",
    ".set_state(",
    ".set_text(",
];

fn polls_writing_widgets(s: &Src, h: &mut Hits) {
    for n in REPEATING {
        for p in find_words(&s.code, n) {
            if s.code[..p].ends_with(b"fn ") {
                continue;
            }
            let open = p + n.len() - 1;
            let body = s.code_str(open, close_of(&s.code, open));
            if let Some(w) = SETTERS.iter().find(|w| body.contains(**w)) {
                hit(h, &s.rel, format!("{} {n}..) writes {w}", s.at(p)));
            }
        }
    }
}

#[test]
fn rh_071_no_poll_writes_a_widget_directly() {
    let mut h = Hits::new();
    let mut v = rc::sources(SURFACES);
    v.extend(SURFACE_FILES.iter().map(|f| Src::load(f)));
    for s in &v {
        polls_writing_widgets(s, &mut h);
    }
    assert_ratchet(
        "RH-071",
        "a poll updates the model, not the widget",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_07_counter_counts_what_it_says() {
    let n = |code: &str| {
        let s = Src::parse("crates/shell/src/ui/settings/x.rs", code.to_owned());
        let mut h = Hits::new();
        polls_writing_widgets(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(
        n("glib::timeout_add_local(POLL, move || { sw.set_active(on()); Continue })"),
        1
    );
    assert_eq!(
        n("glib::timeout_add_local_once(T, move || sw.set_active(on()))"),
        0
    );
    assert_eq!(
        n("glib::timeout_add_local(POLL, move || { model.apply(read()); Continue })"),
        0
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_07_live_model.rs");
    rc::assert_doc_agrees("rules_rc_07_live_model.rs", me, IDS, BASELINE);
}
