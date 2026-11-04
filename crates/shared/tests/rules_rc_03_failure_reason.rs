// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-03 (ADR 0051, docs/STANDARDS.md section 9): every failure carries its
//! classified reason from where it happened to where it is shown. RH-031 counts command results
//! collapsed to a bool (`capture(..).is_some()`, `.status().is_ok()`), which drop the exit code and
//! the last stderr line; RH-032 counts copy sentences that guess the cause ("Check that you're
//! online") outside an offline arm. Counts are frozen per crate or file in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, after, assert_ratchet, close_of, find_words, hit};

/// Debt at the start of 0.13 (2026-11-04, mid-merge tree). Only shrinks.
const BASELINE: &[Row] = &[
    ("RH-031", "api", 4, "B1"),
    ("RH-031", "controller", 22, "C3"),
    ("RH-031", "realtime", 1, "B2"),
    ("RH-031", "shell", 4, "C1"),
    ("RH-031", "webengine", 1, "C4"),
    ("RH-031", "xguard", 1, "B1"),
    ("RH-032", "crates/shell/src/copy.rs", 5, "C1"),
    ("RH-032", "crates/shell/src/copy_settings.rs", 3, "A2"),
];

const IDS: &[&str] = &["RH-031", "RH-032"];

/// Calls that run a command or a privileged step.
const RUNS: &[&str] = &["capture(", "capture_input(", ".output(", ".status("];

/// What turns that result into a bare yes or no.
const COLLAPSE: &[&str] = &[
    ".is_some()",
    ".is_none()",
    ".is_ok()",
    ".is_err()",
    ".is_some_and(",
    ".is_ok_and(",
    ".unwrap_or_default()",
];

fn collapsed(s: &Src, h: &mut Hits, key: &str) {
    for n in RUNS {
        let hits = if n.starts_with('.') {
            rc::find_all(&s.code, n).collect::<Vec<_>>()
        } else {
            find_words(&s.code, n).collect()
        };
        for p in hits {
            if s.code[..p].ends_with(b"fn ") {
                continue;
            }
            let open = p + n.len() - 1;
            let next = after(&s.code, close_of(&s.code, open) + 1);
            if let Some(c) = COLLAPSE.iter().find(|c| next.starts_with(c.as_bytes())) {
                hit(h, key, format!("{} {n}..){c}", s.at(p)));
            }
        }
    }
}

fn guessed_cause(s: &Src, h: &mut Hits) {
    for (off, lit) in &s.lits {
        let l = lit.to_lowercase();
        let guess =
            l.contains("online") && (l.contains("try again") || l.contains("check that you"));
        if !guess {
            continue;
        }
        let n = s.line_of(*off);
        let arm = (n.saturating_sub(3)..=n).any(|k| s.line(k).contains("Offline"));
        if !arm {
            hit(
                h,
                &s.rel,
                format!("{} guesses the cause: {lit}", s.at(*off)),
            );
        }
    }
}

#[test]
fn rh_031_no_command_result_collapsed_to_a_bool() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates"]) {
        let krate = s.rel.split('/').nth(1).unwrap_or("").to_owned();
        collapsed(&s, &mut h, &krate);
    }
    assert_ratchet("RH-031", "a failure keeps its reason", BASELINE, &h);
}

#[test]
fn rh_032_no_copy_sentence_guesses_the_cause() {
    let mut h = Hits::new();
    for f in rc::walk("crates/shell/src", &["rs"]) {
        let file = f.rsplit('/').next().unwrap_or("");
        if file.starts_with("copy") {
            guessed_cause(&Src::load(&f), &mut h);
        }
    }
    assert_ratchet(
        "RH-032",
        "a cause is named only by its own reason",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_03_counters_count_what_they_say() {
    let n = |code: &str| {
        let s = Src::parse("crates/x/src/copy.rs", code.to_owned());
        let mut h = Hits::new();
        collapsed(&s, &mut h, "x");
        guessed_cause(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(n("let ok = env.capture(&[\"a\"]).is_some();"), 1);
    assert_eq!(
        n("let ok = Command::new(p).status().is_ok_and(|s| s.success());"),
        1
    );
    assert_eq!(n("let out = env.capture(&[\"a\"])?;"), 0);
    assert_eq!(n("fn capture(&self) -> Option<String> { None }"), 0);
    assert_eq!(
        n("const A: &str = \"Couldn't load. Check that you're online, then try again.\";"),
        1
    );
    assert_eq!(
        n("Reason::Offline => \"Check that you're online, then try again.\","),
        0
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_03_failure_reason.rs");
    rc::assert_doc_agrees("rules_rc_03_failure_reason.rs", me, IDS, BASELINE);
}
