// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-01 (ADR 0051, docs/STANDARDS.md section 9): a control never shows a
//! value the system has not confirmed. RH-011 counts plain state widgets (switch, picker,
//! segmented, entry, slider) in Settings, Control Center, Store and Displays whose handler calls
//! the host; RH-012 counts Control Center and Store actions sent with no way to report back.
//! Counts are frozen per file in [`BASELINE`] and only fall.
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, close_of, find_words, hit};

/// Debt at the start of 0.13 (2026-11-04, mid-merge tree), per file. Only shrinks.
const BASELINE: &[Row] = &[
    (
        "RH-011",
        "crates/shell/src/ui/settings/desktop_dock.rs",
        1,
        "A2",
    ),
    ("RH-011", "crates/shell/src/ui/settings/effects.rs", 5, "A2"),
    ("RH-011", "crates/shell/src/ui/settings/general.rs", 1, "A2"),
    (
        "RH-011",
        "crates/shell/src/ui/settings/notifications.rs",
        1,
        "A2",
    ),
    ("RH-011", "crates/shell/src/ui/settings/sound.rs", 2, "A2"),
    (
        "RH-012",
        "crates/shell/src/ui/control_center/display.rs",
        2,
        "A3",
    ),
    (
        "RH-012",
        "crates/shell/src/ui/control_center/mod.rs",
        7,
        "A3",
    ),
    (
        "RH-012",
        "crates/shell/src/ui/control_center/pages.rs",
        3,
        "A3",
    ),
    (
        "RH-012",
        "crates/shell/src/ui/control_center/sound.rs",
        2,
        "A3",
    ),
];

const IDS: &[&str] = &["RH-011", "RH-012"];

/// Where a person changes system state with a control.
const SURFACES: &[&str] = &[
    "crates/shell/src/ui/settings",
    "crates/shell/src/ui/control_center",
    "crates/shell/src/ui/store",
];
const SURFACE_FILES: &[&str] = &[
    "crates/shell/src/ui/store.rs",
    "crates/shell/src/ui/store_plugins.rs",
    "crates/shell/src/ui/displays.rs",
];

/// Constructors that show the new value at once (their `pending_` twins wait for `done`).
const PLAIN: &[&str] = &[
    "toggle_row(",
    "w::toggle(",
    "widgets::toggle(",
    "picker_row(",
    "segmented_row(",
    "w::segmented(",
    "entry_row(",
    "slider_row(",
];

/// Words in a handler that reach outside the process.
const HOST: &[&str] = &[
    "act(",
    "save",
    "spawn",
    "write(",
    "hypr::",
    "privileged",
    "admind",
    "Command::",
    "apply(",
    "change(",
    "set_pref",
    "run_",
    "send(",
];

fn surfaces() -> Vec<Src> {
    let mut v = rc::sources(SURFACES);
    v.extend(SURFACE_FILES.iter().map(|f| Src::load(f)));
    v
}

fn plain_widgets_calling_the_host(s: &Src, h: &mut Hits) {
    for n in PLAIN {
        for p in find_words(&s.code, n) {
            if s.code[..p].ends_with(b"fn ") {
                continue; // the definition
            }
            let open = p + n.len() - 1;
            let args = s.code_str(open, close_of(&s.code, open));
            if let Some(w) = HOST.iter().find(|w| args.contains(**w)) {
                hit(h, &s.rel, format!("{} {n} handler calls {w}", s.at(p)));
            }
        }
    }
}

fn actions_without_done(s: &Src, h: &mut Hits) {
    for p in find_words(&s.code, "act(").filter(|&p| p > 0 && s.code[p - 1] == b'.') {
        let open = p + 3;
        let args = s.code_str(open, close_of(&s.code, open));
        if !args.contains('|') && !args.contains("done") {
            hit(h, &s.rel, format!("{} .act( with no done", s.at(p)));
        }
    }
}

#[test]
fn rh_011_no_plain_state_widget_calls_the_host() {
    let mut h = Hits::new();
    for s in surfaces() {
        plain_widgets_calling_the_host(&s, &mut h);
    }
    assert_ratchet(
        "RH-011",
        "a state control that calls the host is a pending control",
        BASELINE,
        &h,
    );
}

#[test]
fn rh_012_every_host_action_reports_back() {
    let mut h = Hits::new();
    let cc_store: Vec<Src> = surfaces()
        .into_iter()
        .filter(|s| !s.rel.contains("/ui/settings/"))
        .collect();
    for s in &cc_store {
        actions_without_done(s, &mut h);
    }
    assert_ratchet(
        "RH-012",
        "every action carries a done callback",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_01_counters_count_what_they_say() {
    let probe = |code: &str| {
        let s = Src::parse("crates/shell/src/ui/settings/probe.rs", code.to_owned());
        let mut h = Hits::new();
        plain_widgets_calling_the_host(&s, &mut h);
        actions_without_done(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(probe("form::toggle_row(\"A\", on, move |v| save(v));"), 1);
    assert_eq!(
        probe("form::pending_toggle_row(\"A\", on, move |v, done| save(v, done));"),
        0
    );
    assert_eq!(probe("w::toggle(on, |v| label.set_text(v));"), 0);
    assert_eq!(probe("fn slider_row(a: f64) { apply(a) }"), 0);
    assert_eq!(probe("cc.act(Action::Wifi(on));"), 1);
    assert_eq!(probe("cc.act(Action::Wifi(on), move |r| done(r));"), 0);
    assert_eq!(
        probe("#[cfg(test)]\nmod tests { fn t() { form::toggle_row(a, b, |v| save(v)); } }"),
        0
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_01_confirmed_state.rs");
    rc::assert_doc_agrees("rules_rc_01_confirmed_state.rs", me, IDS, BASELINE);
}
