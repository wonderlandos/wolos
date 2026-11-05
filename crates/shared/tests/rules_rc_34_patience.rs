// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-34 (QA-ACC-6 and the sites that each answered it alone; docs/STANDARDS.md
//! section 9 and "Origins of the 0.13 bugs"): a wait the person is put on is the person's to
//! lengthen. "Give me more time" (`shared::patience`, `kit::patience` in the shell) stretches the
//! approval sheet, the banners, the toasts, the offers and the countdowns, and wol and wold stretch
//! the waits behind those sheets by the same figure. A one-shot timer written as a bare constant
//! (`timeout_add_local_once(TOAST_MS, ..)`) is the bug: it dismisses what the person is still
//! reading. RH-341 finds every one-shot GLib timer in the shell and wants one of two things in the
//! call or in the forty lines above it (the wait is worked out before the timer is armed): the name `patience` (the wait is stretched), or a
//! `// fixed wait: <why>` comment (an animation, a debounce, a settle delay or a probe: nobody is
//! reading anything while it runs). Counts of timers that are neither are frozen per file in
//! [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, find_words, hit};

/// Timers not yet sorted into "stretched" or "fixed, and why" (2026-11-04, merged tree up to
/// wd-store). Each is a wait the sweep must read once: a dismissal gets `patience`, anything else a
/// `// fixed wait:` line. Only shrinks.
const BASELINE: &[Row] = &[
    ("RH-341", "crates/shell/src/kit/form.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/kit/lifetime.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/kit/mod.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/kit/reveal.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/kit/style.rs", 2, "STD-7"),
    ("RH-341", "crates/shell/src/kit/surface.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/dock.rs", 3, "STD-7"),
    ("RH-341", "crates/shell/src/ui/force_quit.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/ipc.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/lock.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/maker/editor.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/panels.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/plugins.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/searchbar.rs", 3, "STD-7"),
    ("RH-341", "crates/shell/src/ui/settings/lock.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/settings/mod.rs", 1, "STD-7"),
    (
        "RH-341",
        "crates/shell/src/ui/settings/sound.rs",
        2,
        "STD-7",
    ),
    ("RH-341", "crates/shell/src/ui/store.rs", 1, "STD-7"),
    ("RH-341", "crates/shell/src/ui/widgets.rs", 1, "STD-7"),
    (
        "RH-341",
        "crates/shell/src/ui/widgets_builtin.rs",
        1,
        "STD-7",
    ),
];

const IDS: &[&str] = &["RH-341"];

/// The one-shot forms: a repeating timer is a poll (RH-071, RH-222), not a wait on the person.
const ONE_SHOT: &[&str] = &[
    "timeout_add_local_once(",
    "timeout_add_seconds_local_once(",
    "timeout_add_once(",
    "timeout_add_seconds_once(",
];

/// Lines above the call that may carry the stretch or the reason.
const LOOK_UP: usize = 40;

/// The one-shot timers of `s` that neither read patience nor say why not.
fn unsorted(s: &Src, h: &mut Hits) {
    for n in ONE_SHOT {
        for p in find_words(&s.code, n) {
            if s.code[..p].ends_with(b"fn ") {
                continue;
            }
            let line = s.line_of(p);
            let from = line.saturating_sub(LOOK_UP).max(1);
            let window: String = (from..=line + 3)
                .map(|l| s.line(l))
                .collect::<Vec<_>>()
                .join("\n");
            if window.contains("patience") || window.contains("// fixed wait:") {
                continue;
            }
            hit(
                h,
                &s.rel,
                format!("{} {n}..) neither patience nor `// fixed wait:`", s.at(p)),
            );
        }
    }
}

#[test]
fn rh_341_a_wait_on_the_person_reads_patience() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates/shell"]) {
        unsorted(&s, &mut h);
    }
    assert_ratchet(
        "RH-341",
        "a one-shot timer reads patience or says why it is fixed",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_34_counter_counts_what_it_says() {
    let n = |code: &str| {
        let s = Src::parse("crates/shell/src/ui/x.rs", code.to_owned());
        let mut h = Hits::new();
        unsorted(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(
        n("fn f() { glib::timeout_add_local_once(TOAST, move || hide()); }"),
        1,
        "a bare constant is the bug"
    );
    assert_eq!(
        n(
            "fn f() {\n let stay = kit::patience::current().bounded(TOAST);\n glib::timeout_add_local_once(stay, move || hide());\n}"
        ),
        0,
        "patience above the call stretches it"
    );
    assert_eq!(
        n(
            "fn f() {\n // fixed wait: debounce of the search box\n glib::timeout_add_local_once(D, move || go());\n}"
        ),
        0,
        "a reason is accepted"
    );
    assert_eq!(
        n("fn f() { glib::timeout_add_local(POLL, move || tick()); }"),
        0,
        "a repeating timer is a poll, not a wait"
    );
    assert_eq!(
        n("fn timeout_add_local_once(x: u32) {}"),
        0,
        "a definition is not a call"
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_34_patience.rs");
    rc::assert_doc_agrees("rules_rc_34_patience.rs", me, IDS, BASELINE);
}
