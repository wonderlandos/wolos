// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-23 (ADR 0051, docs/STANDARDS.md section 9): no surface has a fixed size
//! larger than the smallest supported logical screen, 640x400 at any scale, and the compositor
//! scale is never pinned. RH-231 counts size constants and literal sizes above that screen in the
//! shell and the window kit; RH-232 counts compositor configs whose monitor rule pins a number
//! as the scale. The layout check and the golden size matrix are the other half (ADR 0051, D1 and
//! D2a). Counts are frozen per file in [`BASELINE`].
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, close_of, find_words, hit};

/// Debt at the start of 0.13 (2026-11-04, mid-merge tree), per file. Only shrinks.
const BASELINE: &[Row] = &[
    ("RH-231", "crates/shell/src/kit/surface.rs", 3, "D2a"),
    ("RH-231", "crates/shell/src/kit/widgets.rs", 1, "D2a"),
    ("RH-231", "crates/shell/src/ui/app_view.rs", 1, "D2a"),
    ("RH-231", "crates/shell/src/ui/drop_shelf.rs", 1, "D2a"),
    ("RH-231", "crates/shell/src/ui/shortcut_sheet.rs", 2, "D2a"),
    (
        "RH-232",
        "desktop/overlay/usr/share/wolos/hypr/hyprland.conf",
        1,
        "D2a",
    ),
    (
        "RH-232",
        "desktop/overlay/usr/share/wolos/hypr/hyprland.lua",
        1,
        "D2a",
    ),
    (
        "RH-232",
        "desktop/overlay/usr/share/wolos/hypr/installer.conf",
        1,
        "D2a",
    ),
    (
        "RH-232",
        "desktop/overlay/usr/share/wolos/hypr/installer.lua",
        1,
        "D2a",
    ),
    (
        "RH-232",
        "desktop/overlay/usr/share/wolos/hypr/login.conf",
        1,
        "D2a",
    ),
    (
        "RH-232",
        "desktop/overlay/usr/share/wolos/hypr/login.lua",
        1,
        "D2a",
    ),
    (
        "RH-232",
        "desktop/overlay/usr/share/wolos/hypr/setup.conf",
        1,
        "D2a",
    ),
    (
        "RH-232",
        "desktop/overlay/usr/share/wolos/hypr/setup.lua",
        1,
        "D2a",
    ),
];

const IDS: &[&str] = &["RH-231", "RH-232"];

/// The smallest supported logical screen.
const MIN_W: i64 = 640;
const MIN_H: i64 = 400;

/// Which axis a constant's name measures, if any.
fn axis(name: &str) -> Option<char> {
    let n = name.trim_start_matches("MAX_").trim_start_matches("MIN_");
    if n.ends_with("WIDTH") || n.ends_with("_W") || n == "W" {
        Some('w')
    } else if n.ends_with("HEIGHT") || n.ends_with("_H") || n == "H" {
        Some('h')
    } else {
        None
    }
}

fn too_big(axis: char, v: i64) -> bool {
    (axis == 'w' && v > MIN_W) || (axis == 'h' && v > MIN_H)
}

/// The integer literal at the start of `t`, if it is one.
fn int_at(t: &str) -> Option<i64> {
    let d: String = t
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '_')
        .collect();
    d.replace('_', "").parse().ok()
}

fn oversized(s: &Src, h: &mut Hits) {
    let code = String::from_utf8_lossy(&s.code).into_owned();
    for p in find_words(&s.code, "const ") {
        let rest = &code[p + 6..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let Some(ax) = axis(&name) else { continue };
        let Some(eq) = rest.find('=') else { continue };
        if rest[..eq].contains(';') {
            continue;
        }
        if let Some(v) = int_at(&rest[eq + 1..])
            && too_big(ax, v)
        {
            hit(h, &s.rel, format!("{} {name} = {v}", s.at(p)));
        }
    }
    for n in ["set_size_request(", "set_default_size("] {
        for p in rc::find_all(&s.code, n) {
            let open = p + n.len() - 1;
            let args = s.code_str(open + 1, close_of(&s.code, open));
            let mut it = args.split(',');
            let (w, ht) = (it.next().and_then(int_at), it.next().and_then(int_at));
            if w.is_some_and(|v| too_big('w', v)) || ht.is_some_and(|v| too_big('h', v)) {
                hit(h, &s.rel, format!("{} {n}{})", s.at(p), args.trim()));
            }
        }
    }
}

/// Monitor rules of a compositor config that pin a number as the scale.
fn pinned(rel: &str, text: &str, h: &mut Hits) {
    for (i, l) in text.lines().enumerate() {
        let t = l.trim_start();
        if t.starts_with('#') || t.starts_with("--") {
            continue;
        }
        let conf = t.starts_with("monitor") && t.contains('=') && {
            let last = t.rsplit(',').next().unwrap_or("").trim();
            last.parse::<f64>().is_ok() && t.split(',').count() >= 4
        };
        let lua = t.contains("monitor(")
            && t.split("scale").nth(1).is_some_and(|r| {
                let v = r.trim_start().trim_start_matches('=').trim_start();
                v.starts_with(|c: char| c.is_ascii_digit())
            });
        if conf || lua {
            hit(h, rel, format!("{rel}:{} {}", i + 1, t));
        }
    }
}

#[test]
fn rh_231_no_fixed_size_above_the_smallest_screen() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates/shell/src", "crates/wolkit/src"]) {
        oversized(&s, &mut h);
    }
    assert_ratchet(
        "RH-231",
        "a fixed size fits 640x400 or is clamped to the screen",
        BASELINE,
        &h,
    );
}

#[test]
fn rh_232_the_compositor_scale_is_not_pinned() {
    let mut h = Hits::new();
    for f in rc::walk("desktop", &["conf", "lua"])
        .into_iter()
        .chain(rc::walk("files", &["conf", "lua"]))
    {
        if f.contains("/hypr/") {
            pinned(&f, &rc::read(&f), &mut h);
        }
    }
    assert_ratchet("RH-232", "the monitor scale is auto", BASELINE, &h);
}

#[test]
fn rc_23_counters_count_what_they_say() {
    let n = |code: &str| {
        let s = Src::parse("crates/shell/src/x.rs", code.to_owned());
        let mut h = Hits::new();
        oversized(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    assert_eq!(n("pub const WIDTH: i32 = 960;"), 1);
    assert_eq!(n("pub const MAX_HEIGHT: i32 = 640;"), 1);
    assert_eq!(n("pub const SHEET_WIDTH: i32 = 360;"), 0);
    assert_eq!(
        n("const CARD: (i32, i32) = (700, 300);"),
        0,
        "a tuple is read by the layout check"
    );
    assert_eq!(n("w.set_default_size(1280, 800);"), 1);
    assert_eq!(n("w.set_size_request(w::ICON, 24);"), 0);
    let mut h = Hits::new();
    pinned(
        "x.conf",
        "monitor = , preferred, auto, 1\nmonitor = , preferred, auto, auto\n# monitor = ,a,b,1",
        &mut h,
    );
    pinned(
        "x.lua",
        "hl.monitor({ output = \"\", scale = 1 })\nhl.monitor({ scale = \"auto\" })",
        &mut h,
    );
    assert_eq!(h.values().map(Vec::len).sum::<usize>(), 2);
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_23_fits_the_screen.rs");
    rc::assert_doc_agrees("rules_rc_23_fits_the_screen.rs", me, IDS, BASELINE);
}
