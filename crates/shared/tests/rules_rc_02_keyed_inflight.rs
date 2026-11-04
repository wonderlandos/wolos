// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-02 (ADR 0051, docs/STANDARDS.md section 9): in-flight state is keyed by
//! request (a queue for approvals, a generation for async results, an exclusive lock for a
//! transaction), never one slot a second request overwrites. RH-021 counts fields and statics
//! that hold one in-flight item (`pending: RefCell<Option<Action>>`, `password: Cell<Option<..>>`)
//! in the shell, the daemons and the command line. Timer handles (`SourceId`, `JoinHandle`) are
//! not requests and do not count. Counts are frozen per file in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, hit};

/// Debt at the start of 0.13 (2026-11-04, mid-merge tree), per file. Only shrinks.
const BASELINE: &[Row] = &[
    (
        "RH-021",
        "crates/shell/src/ui/control_center/mod.rs",
        1,
        "A3",
    ),
    ("RH-021", "crates/shell/src/ui/settings/mod.rs", 1, "A2"),
];

const IDS: &[&str] = &["RH-021"];

/// Names of in-flight things.
const INFLIGHT: &[&str] = &[
    "pending",
    "password",
    "secret",
    "action",
    "request",
    "query",
    "txn",
    "transaction",
];

/// Holders of exactly one optional value.
const SLOT: &[&str] = &[
    "RefCell<Option<",
    "Cell<Option<",
    "Mutex<Option<",
    "RwLock<Option<",
];

/// What may sit in a slot: handles of a timer, a thread or a source, not requests.
const HANDLES: &[&str] = &["SourceId", "JoinHandle", "Instant"];

fn single_slots(s: &Src, h: &mut Hits) {
    let code = String::from_utf8_lossy(&s.code).into_owned();
    for (i, l) in code.lines().enumerate() {
        let t = l.trim_start();
        if t.starts_with("let ") || t.starts_with("fn ") || t.contains(" fn ") {
            continue; // a local or a parameter is not state that outlives the request
        }
        let Some((name, ty)) = t.split_once(':') else {
            continue;
        };
        let name = name
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub ")
            .trim_start_matches("static ");
        let name = name.trim().to_lowercase();
        if name.contains(' ') || !INFLIGHT.iter().any(|w| name.contains(w)) {
            continue;
        }
        if SLOT.iter().any(|w| ty.contains(w)) && !HANDLES.iter().any(|w| ty.contains(w)) {
            hit(
                h,
                &s.rel,
                format!("{}:{} {}", s.rel, i + 1, t.trim_end_matches(',')),
            );
        }
    }
}

#[test]
fn rh_021_no_single_in_flight_slot() {
    let mut h = Hits::new();
    for s in rc::sources(&[
        "crates/shell/src",
        "crates/api/src",
        "crates/controller/src",
        "crates/core/src",
    ]) {
        single_slots(&s, &mut h);
    }
    assert_ratchet(
        "RH-021",
        "in-flight state is keyed by request",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_02_counter_counts_what_it_says() {
    let n = |code: &str| {
        let s = Src::parse("crates/shell/src/ui/x.rs", code.to_owned());
        let mut h = Hits::new();
        single_slots(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(n("struct A {\n    pending: RefCell<Option<Action>>,\n}"), 1);
    assert_eq!(
        n("struct A {\n    pub(crate) password: Cell<Option<String>>,\n}"),
        1
    );
    assert_eq!(
        n("struct A {\n    pending: RefCell<Option<glib::SourceId>>,\n}"),
        0
    );
    assert_eq!(n("struct A {\n    queue: RefCell<AuthQueue>,\n}"), 0);
    assert_eq!(
        n("fn f() {\n    let pending: RefCell<Option<Action>> = Default::default();\n}"),
        0
    );
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_02_keyed_inflight.rs");
    rc::assert_doc_agrees("rules_rc_02_keyed_inflight.rs", me, IDS, BASELINE);
}
