// SPDX-License-Identifier: BSD-2-Clause
//! Root-cause standard RC-39 (CRIT-2, QA-PERF-1; docs/STANDARDS.md section 9): every layer window
//! the shell presents paints something in its first frame. GTK never attaches a buffer to a
//! window that renders nothing, so its layer surface stays unmapped while its frame clock commits
//! it empty every 16 ms, and Hyprland answers each commit with a configure and a redraw of the
//! output (four cores of compositor with one panel open). RH-391 finds the layer windows the shell
//! makes (`window(app, ...)` of `ui`, `surface::place(`) and shows (`present()`, `set_visible` but
//! with `false`) in the same function with no child set there; one with nothing of its own to show
//! takes `kit::data::presence()`. A window handed back to be filled and shown elsewhere is the
//! caller's (it shows nothing until then).
#[path = "support/rc.rs"]
mod rc;
use rc::{Hits, Row, Src, assert_ratchet, hit};

/// Debt when the rule came (2026-11-04): the two click-catchers of `ui/transient.rs`, which
/// wave3/crit2 f705c45 gives `presence()`; the row goes when it merges. Only shrinks.
const BASELINE: &[Row] = &[("RH-391", "crates/shell/src/ui/transient.rs", 2, "C1")];

const IDS: &[&str] = &["RH-391"];

/// Calls that make a layer window and hand it back.
const MAKES: &[&str] = &["super::window(", "ui::window(", "surface::place("];

/// The name a `let NAME = <call>` binds, when the statement at `p` is one.
fn bound_name(code: &str, p: usize) -> Option<String> {
    let start = code[..p].rfind(['\n', ';', '{']).map_or(0, |i| i + 1);
    let stmt = code[start..p].trim_start();
    let rest = stmt.strip_prefix("let ")?;
    let rest = rest.strip_prefix("mut ").unwrap_or(rest);
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty() && rest[name.len()..].trim_start().starts_with(['=', ':'])).then_some(name)
}

/// The end of the function the offset `p` is in: the first line after it that closes at the
/// indentation of the `fn` line above it.
fn fn_end(code: &str, p: usize) -> usize {
    let line_start = |i: usize| code[..i].rfind('\n').map_or(0, |n| n + 1);
    let mut i = line_start(p);
    let indent = loop {
        let l = &code[i..code[i..].find('\n').map_or(code.len(), |n| i + n)];
        let t = l.trim_start();
        if t.starts_with("fn ") || t.starts_with("pub fn ") || t.starts_with("pub(crate) fn ") {
            break l.len() - t.len();
        }
        if i == 0 {
            break 0;
        }
        i = line_start(i - 1);
    };
    let close = format!("\n{}}}", " ".repeat(indent));
    code[p..].find(&close).map_or(code.len(), |n| p + n)
}

fn childless(s: &Src, h: &mut Hits) {
    let code = String::from_utf8_lossy(&s.code).into_owned();
    for m in MAKES {
        for (p, _) in code.match_indices(m) {
            let Some(name) = bound_name(&code, p) else {
                continue;
            };
            let body = &code[p..fn_end(&code, p)];
            let shown = body.contains(&format!("{name}.present()"))
                || body
                    .match_indices(&format!("{name}.set_visible("))
                    .any(|(i, t)| !body[i + t.len()..].trim_start().starts_with("false"));
            if shown && !body.contains(&format!("{name}.set_child(")) {
                hit(
                    h,
                    &s.rel,
                    format!("{} {name} = {m}..) is shown with no child", s.at(p)),
                );
            }
        }
    }
}

#[test]
fn rh_391_every_layer_window_paints_in_its_first_frame() {
    let mut h = Hits::new();
    for s in rc::sources(&["crates/shell/src"]) {
        childless(&s, &mut h);
    }
    assert_ratchet(
        "RH-391",
        "a layer window has a child (kit::data::presence() when it shows nothing)",
        BASELINE,
        &h,
    );
}

#[test]
fn rc_39_counter_counts_what_it_says() {
    let n = |code: &str| {
        let s = Src::parse("crates/shell/src/x.rs", code.to_owned());
        let mut h = Hits::new();
        childless(&s, &mut h);
        h.values().map(Vec::len).sum::<usize>()
    };
    let empty = "fn a(app: &App) {\n    let catcher = super::window(app, \"catcher\", spec);\n    catcher.present();\n}\n";
    assert_eq!(n(empty), 1);
    let shown = "fn a(app: &App) {\n    let c = super::window(app, \"c\", spec);\n    c.set_visible(open);\n}\n";
    assert_eq!(n(shown), 1);
    let marked = "fn a(app: &App) {\n    let c = super::window(app, \"c\", spec);\n    c.set_child(Some(&kit::data::presence()));\n    c.present();\n}\n";
    assert_eq!(n(marked), 0);
    let handed = "fn a(app: &App) -> W {\n    let c = super::window(app, \"c\", spec);\n    c.set_visible(false);\n    c\n}\n";
    assert_eq!(n(handed), 0, "hidden and handed back: the caller fills it");
}

#[test]
fn the_doc_and_the_tests_agree() {
    let me = include_str!("rules_rc_39_layer_paints.rs");
    rc::assert_doc_agrees("rules_rc_39_layer_paints.rs", me, IDS, BASELINE);
}
