// SPDX-License-Identifier: BSD-2-Clause
//! The HIG numbers agree (ADR 0058 gap M4). `docs/modules/hig.md` section 1.1 (the table, the
//! drawing, the sidebar rule, the density table's standard column), `shared::theme::scaffold`, the
//! compositor strip, `kit.css` and the design repo's `brand-screens.md` and `brand-ui.md` state
//! one set of sizes: toolbar 40, sidebar 240, sidebar row 24 with a 4 px gap (pitch 28), list row 40,
//! control 24, switch 36 x 24 (compact 24 x 16). A drift in any of them fails here, with the file
//! that disagrees. The Kit's drawn sizes (the list row is 52 and the switch 44 x 28 today) are the
//! Kit test's `KNOWN` ratchet, not this test: it holds the numbers the documents promise.
//!
//! The brand documents live in the design repo (`design/docs/adr/`). A checkout whose design
//! submodule does not carry them yet skips that half and says so.
use shared::theme::scaffold as m;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> Option<String> {
    std::fs::read_to_string(root().join(rel)).ok()
}

/// The text of the files the numbers live in.
#[derive(Clone)]
struct Sources {
    hig: String,
    kit_css: String,
    titlebar: String,
    brand_screens: Option<String>,
    brand_ui: Option<String>,
}

fn load() -> Sources {
    let need = |rel: &str| read(rel).unwrap_or_else(|| panic!("{rel} is missing"));
    Sources {
        hig: need("docs/modules/hig.md"),
        kit_css: need("crates/shell/src/kit/kit.css"),
        titlebar: need("crates/realtime/src/titlebar.rs"),
        brand_screens: read("design/docs/adr/brand-screens.md"),
        brand_ui: read("design/docs/adr/brand-ui.md"),
    }
}

/// The cells of the table row whose first cell is `first` (trimmed, backticks kept), in `text`.
fn row<'a>(text: &'a str, first: &str) -> Option<Vec<&'a str>> {
    text.lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| {
            l.trim()
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>()
        })
        .find(|c| c.first().is_some_and(|f| f.trim_matches('`') == first))
}

/// The text of one `###` section of the HIG.
fn section<'a>(hig: &'a str, heading: &str) -> &'a str {
    let at = hig
        .find(&format!("### {heading}"))
        .unwrap_or_else(|| panic!("hig.md has no section `{heading}`"));
    let rest = &hig[at + 4..];
    let end = rest.find("\n##").unwrap_or(rest.len());
    &rest[..end]
}

/// `n` of the first `<n> x <m>` in `cell`, as (n, m).
fn pair(cell: &str) -> Option<(i32, i32)> {
    let mut it = cell.split_whitespace();
    let mut prev: Option<&str> = None;
    while let Some(w) = it.next() {
        if w == "x" {
            let a = prev?
                .trim_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .ok()?;
            let b = it
                .next()?
                .trim_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .ok()?;
            return Some((a, b));
        }
        prev = Some(w);
    }
    None
}

fn num(cell: &str) -> Option<i32> {
    cell.split_whitespace()
        .next()?
        .trim_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()
}

fn want(bad: &mut Vec<String>, name: &str, got: Option<i32>, expect: i32, from: &str) {
    if got != Some(expect) {
        bad.push(format!("{from}: {name} is {got:?}, expected {expect}"));
    }
}

/// Every disagreement, one sentence each. Empty when the sources agree.
fn problems(s: &Sources) -> Vec<String> {
    let mut bad = Vec::new();
    let t = section(&s.hig, "1.1");
    // The table of 1.1 against the constants.
    for (part, expect) in [
        ("Toolbar", m::KIT_TOOLBAR_HEIGHT),
        ("Sidebar width", m::SIDEBAR_WIDTH),
        ("Sidebar row (`SourceItem`)", m::SOURCE_ROW),
        ("List row", m::LIST_ROW),
        (
            "Control (button, entry, dropdown, menu item)",
            m::CONTROL_HEIGHT,
        ),
        ("Third-party app header bar", m::TOOLBAR_HEIGHT),
        ("Compositor strip", 28),
        ("Status bar", 24),
    ] {
        let got = row(t, part).and_then(|c| c.get(1).copied().and_then(num));
        want(&mut bad, part, got, expect, "docs/modules/hig.md 1.1");
    }
    match row(t, "Sidebar row (`SourceItem`)").and_then(|c| c.get(2).copied()) {
        Some(c) if c.contains(&format!("pitch is {}", m::SOURCE_ROW + 4)) => {}
        c => bad.push(format!(
            "docs/modules/hig.md 1.1: the sidebar row says {c:?}, expected a pitch of {}",
            m::SOURCE_ROW + 4
        )),
    }
    // The strip is the compositor's constant; the status bar and the switch are kit.css's.
    if !s.titlebar.contains("pub const HEIGHT: i32 = 28;") {
        bad.push(
            "crates/realtime/src/titlebar.rs: HEIGHT is no longer 28 (hig.md 1.1 strip)".into(),
        );
    }
    if !s.kit_css.contains("label.status-bar { min-height: 24px;") {
        bad.push("kit.css: label.status-bar is not 24 (hig.md 1.1 status bar)".into());
    }
    // The switch: 36 x 24, compact 24 x 16, in the HIG, the brand documents and kit.css.
    let hig_switch = row(t, "Switch").and_then(|c| c.get(1).copied().and_then(pair));
    if hig_switch != Some((36, 24)) {
        bad.push(format!(
            "docs/modules/hig.md 1.1: switch is {hig_switch:?}, expected (36, 24)"
        ));
    }
    if !row(t, "Switch")
        .and_then(|c| c.get(2).map(|c| c.contains("24 x 16")))
        .unwrap_or(false)
    {
        bad.push(
            "docs/modules/hig.md 1.1: the switch row does not name the 24 x 16 compact size".into(),
        );
    }
    for (file, text, col) in [
        ("brand-screens.md", s.brand_screens.as_deref(), 2usize),
        ("brand-ui.md", s.brand_ui.as_deref(), 2usize),
    ] {
        let Some(text) = text else {
            eprintln!("hig_numbers: design/docs/adr/{file} is not in this checkout; skipped");
            continue;
        };
        let got = row(text, "switch").and_then(|c| c.get(col).copied().and_then(pair));
        if got != Some((36, 24)) {
            bad.push(format!(
                "design/docs/adr/{file}: the switch row proposes {got:?}, expected (36, 24)"
            ));
        }
    }
    if let Some(ui) = s.brand_ui.as_deref() {
        let got =
            row(ui, "compact switch (detail head)").and_then(|c| c.get(2).copied().and_then(pair));
        if got != Some((24, 16)) {
            bad.push(format!(
                "design/docs/adr/brand-ui.md: the compact switch is {got:?}, expected (24, 16)"
            ));
        }
    }
    if !s.kit_css.contains("switch.toggle { background: var(--control-off); border: none; border-radius: 999px; min-width: 36px; min-height: 24px;")
        || !s.kit_css.contains(".detail-head switch.toggle { min-width: 24px; min-height: 16px; }")
    {
        bad.push("kit.css: switch.toggle is not 36 x 24 (compact 24 x 16) (hig.md 1.1 switch)".into());
    }
    // The drawing and the sidebar rule in section 1 repeat three of the numbers.
    for needle in [
        format!("Toolbar ({}px", m::KIT_TOOLBAR_HEIGHT),
        format!("{}px      │", m::SIDEBAR_WIDTH),
        format!(
            "rows {}px high with a 4px gap (a pitch of {})",
            m::SOURCE_ROW,
            m::SOURCE_ROW + 4
        ),
    ] {
        if !s.hig.contains(&needle) {
            bad.push(format!("docs/modules/hig.md section 1: `{needle}` is missing (the drawing or the sidebar rule drifted)"));
        }
    }
    // The density table's standard column is today's numbers.
    let d = section(&s.hig, "1.2");
    for (metric, expect) in [
        ("List row", m::LIST_ROW),
        ("Control height", m::CONTROL_HEIGHT),
        ("Sidebar row pitch", m::SOURCE_ROW + 4),
        ("Toolbar", m::KIT_TOOLBAR_HEIGHT),
    ] {
        let got = row(d, metric).and_then(|c| c.get(2).copied().and_then(num));
        want(
            &mut bad,
            metric,
            got,
            expect,
            "docs/modules/hig.md 1.2 (Standard column)",
        );
    }
    bad
}

#[test]
fn the_hig_scaffold_brand_and_kit_numbers_agree() {
    let bad = problems(&load());
    assert!(
        bad.is_empty(),
        "the HIG numbers disagree:\n{}",
        bad.join("\n")
    );
}

/// The test itself fails on drift: each edit below is the kind of change that left four sources
/// with four numbers.
#[test]
fn a_drifted_source_is_reported() {
    let base = load();
    assert!(problems(&base).is_empty(), "start from agreeing sources");
    let edit = |f: &dyn Fn(&mut Sources)| {
        let mut s = base.clone();
        f(&mut s);
        problems(&s)
    };
    let said = |p: Vec<String>, what: &str| {
        assert!(
            p.iter().any(|l| l.contains(what)),
            "expected a problem naming `{what}`, got {p:?}"
        );
    };
    said(
        edit(&|s| s.hig = s.hig.replace("| Toolbar | 40 |", "| Toolbar | 36 |")),
        "Toolbar",
    );
    said(
        edit(&|s| {
            s.hig = s
                .hig
                .replace("| Sidebar width | 240 |", "| Sidebar width | 220 |")
        }),
        "Sidebar width",
    );
    said(
        edit(&|s| s.hig = s.hig.replace("so the pitch is 28", "so the pitch is 24")),
        "pitch",
    );
    said(
        edit(&|s| {
            s.hig = s
                .hig
                .replace("| Switch | 36 x 24 |", "| Switch | 40 x 24 |")
        }),
        "switch",
    );
    said(
        edit(&|s| s.hig = s.hig.replace("Toolbar (40px", "Toolbar (36px")),
        "drifted",
    );
    said(
        edit(&|s| {
            s.hig = s
                .hig
                .replace("| List row | 32 | 40 | 48 |", "| List row | 32 | 52 | 48 |")
        }),
        "Standard column",
    );
    said(
        edit(&|s| {
            s.kit_css = s.kit_css.replace(
                "min-width: 36px; min-height: 24px;",
                "min-width: 40px; min-height: 24px;",
            )
        }),
        "kit.css",
    );
    said(
        edit(&|s| s.titlebar = s.titlebar.replace("HEIGHT: i32 = 28", "HEIGHT: i32 = 32")),
        "titlebar.rs",
    );
    if base.brand_ui.is_some() {
        said(
            edit(&|s| {
                s.brand_ui = s
                    .brand_ui
                    .take()
                    .map(|t| t.replace("| 36 x 24, padding 2", "| 40 x 24, padding 2"))
            }),
            "brand-ui.md",
        );
    }
    if base.brand_screens.is_some() {
        said(
            edit(&|s| {
                s.brand_screens = s
                    .brand_screens
                    .take()
                    .map(|t| t.replace("| 36 x 24, thumb 20", "| 40 x 24, thumb 20"))
            }),
            "brand-screens.md",
        );
    }
}
