// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-22 (ADR 0051, docs/STANDARDS.md section 9): every process the session
//! depends on is supervised and every wait has one named deadline. RH-221 counts long-running
//! units we ship without `Restart=` or with the default start limit (a unit that gives up after
//! five quick failures leaves the screen black for good); RH-222 counts waits written as a
//! literal at the call (`Duration::from_secs(5)`) instead of a named constant both ends share;
//! RH-223 counts spawns whose failure is a panic (`.spawn(..).expect(`). The kill journeys
//! `qa100-live-kill-shell` and `qa100-live-kill-wold` are the live half (brief B2).
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, after, assert_ratchet, close_of, hit};

/// Debt at the start of 0.13 (2026-11-04, mid-merge tree). Only shrinks.
const BASELINE: &[Row] = &[
    (
        "RH-221",
        "files/usr/lib/systemd/system/wolos-clam.service",
        2,
        "B2",
    ),
    (
        "RH-221",
        "files/usr/lib/systemd/system/wolos-xguard.service",
        1,
        "B2",
    ),
    ("RH-222", "api", 14, "B2"),
    ("RH-222", "audio", 7, "C9"),
    ("RH-222", "controller", 18, "A1"),
    ("RH-222", "core", 7, "B4"),
    ("RH-222", "proxy", 1, "C4"),
    ("RH-222", "realtime", 1, "B2"),
    ("RH-222", "shell", 34, "A3"),
    ("RH-222", "webengine", 2, "C4"),
    ("RH-222", "wol-lua", 3, "A10"),
    ("RH-222", "wolkit", 2, "C4"),
    ("RH-222", "wopkg", 3, "C2"),
    ("RH-222", "xguard", 3, "B1"),
];

const IDS: &[&str] = &["RH-221", "RH-222", "RH-223"];

/// Directories whose `*.service` files are our units.
const UNIT_DIRS: &[&str] = &["files/usr/lib/systemd/system", "files/usr/lib/systemd/user"];

/// `key=value` of a unit and its drop-ins, last one wins; `section` filters by `[Section]`.
fn unit_value(texts: &[String], section: &str, key: &str) -> Option<String> {
    let mut v = None;
    for t in texts {
        let mut cur = String::new();
        for l in t.lines() {
            let l = l.trim();
            if l.starts_with('[') {
                cur = l.to_owned();
            } else if cur == section
                && let Some(x) = l.strip_prefix(key).and_then(|r| r.strip_prefix('='))
            {
                v = Some(x.trim().to_owned());
            }
        }
    }
    v
}

/// What a long-running unit lacks: `Restart=` and `StartLimitIntervalSec=0`.
fn unsupervised(texts: &[String]) -> Vec<&'static str> {
    if unit_value(texts, "[Service]", "Type").is_some_and(|t| t == "oneshot") {
        return Vec::new();
    }
    let mut miss = Vec::new();
    let restart = unit_value(texts, "[Service]", "Restart");
    if !restart.is_some_and(|r| matches!(r.as_str(), "on-failure" | "always" | "on-abnormal")) {
        miss.push("Restart=on-failure");
    }
    if unit_value(texts, "[Unit]", "StartLimitIntervalSec").is_none_or(|v| v != "0") {
        miss.push("StartLimitIntervalSec=0");
    }
    miss
}

#[test]
fn rh_221_long_running_units_are_supervised() {
    let mut h = Hits::new();
    let mut seen = 0;
    for dir in UNIT_DIRS {
        let Ok(rd) = std::fs::read_dir(rc::root().join(dir)) else {
            continue;
        };
        let mut names: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".service"))
            .collect();
        names.sort();
        for n in names {
            seen += 1;
            let rel = format!("{dir}/{n}");
            let mut texts = vec![rc::read(&rel)];
            for d in rc::walk(&format!("{rel}.d"), &["conf"]) {
                texts.push(rc::read(&d));
            }
            for m in unsupervised(&texts) {
                hit(&mut h, &rel, format!("{rel}: no {m}"));
            }
        }
    }
    assert!(seen >= 10, "only {seen} units found");
    assert_ratchet(
        "RH-221",
        "a long-running unit restarts and has no start limit",
        BASELINE,
        &h,
    );
}

/// Waits written as a number at the call.
fn literal_waits(s: &Src, h: &mut Hits, key: &str) {
    for n in ["from_secs(", "from_millis("] {
        for p in rc::find_all(&s.code, n).filter(|&p| s.code[..p].ends_with(b"Duration::")) {
            let arg = after(&s.code, p + n.len());
            let line = s.line(s.line_of(p)).trim_start().to_owned();
            let named = line.starts_with("const ")
                || line.starts_with("pub const ")
                || line.starts_with("pub(crate) const ")
                || line.starts_with("static ")
                || line.starts_with("pub static ");
            if arg.first().is_some_and(u8::is_ascii_digit) && !named {
                hit(h, key, format!("{} Duration::{n}<number>)", s.at(p)));
            }
        }
    }
}

/// Spawns whose failure panics.
fn panicking_spawns(s: &Src, h: &mut Hits, key: &str) {
    for p in rc::find_all(&s.code, ".spawn(") {
        let open = p + ".spawn".len();
        let next = after(&s.code, close_of(&s.code, open) + 1);
        if next.starts_with(b".expect(") || next.starts_with(b".unwrap()") {
            hit(h, key, format!("{} .spawn(..) panics on failure", s.at(p)));
        }
    }
}

fn crate_of(rel: &str) -> String {
    rel.split('/').nth(1).unwrap_or("").to_owned()
}

#[test]
fn rh_222_every_wait_is_a_named_deadline() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates"]) {
        literal_waits(&s, &mut h, &crate_of(&s.rel));
    }
    assert_ratchet("RH-222", "a wait is a named constant", BASELINE, &h);
}

#[test]
fn rh_223_a_failed_spawn_is_an_error_not_a_panic() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates"]) {
        panicking_spawns(&s, &mut h, &crate_of(&s.rel));
    }
    assert_ratchet("RH-223", "a spawn returns its error", BASELINE, &h);
}

#[test]
fn rc_22_readers_read_what_they_say() {
    let u = |t: &str| unsupervised(&[t.to_owned()]);
    assert_eq!(
        u("[Service]\nType=oneshot\nExecStart=/x\n"),
        Vec::<&str>::new()
    );
    assert_eq!(
        u("[Service]\nExecStart=/x\n"),
        ["Restart=on-failure", "StartLimitIntervalSec=0"]
    );
    assert_eq!(
        u("[Unit]\nStartLimitIntervalSec=0\n[Service]\nRestart=always\n"),
        Vec::<&str>::new()
    );
    let drop_in = unsupervised(&[
        "[Service]\nRestart=always\n".to_owned(),
        "[Unit]\nStartLimitIntervalSec=0\n".to_owned(),
    ]);
    assert!(drop_in.is_empty(), "a drop-in counts");
    let n = |code: &str| {
        let s = Src::parse("crates/x/src/a.rs", code.to_owned());
        let mut h = Hits::new();
        literal_waits(&s, &mut h, "x");
        panicking_spawns(&s, &mut h, "x");
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(n("let t = Duration::from_secs(5);"), 1);
    assert_eq!(n("const DEADLINE: Duration = Duration::from_secs(5);"), 0);
    assert_eq!(n("let t = Duration::from_secs(DEADLINE_S);"), 0);
    assert_eq!(
        n("thread::Builder::new().name(n).spawn(move || f()).expect(\"thread\");"),
        1
    );
    assert_eq!(n("let c = cmd.spawn()?;"), 0);
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_22_supervised.rs");
    rc::assert_doc_agrees("rules_rc_22_supervised.rs", me, IDS, BASELINE);
}
