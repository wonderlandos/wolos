// SPDX-License-Identifier: BSD-2-Clause
//! One source for the parts of a Kit window (WOLKIT-1TO1): the numbers of the window chrome, the
//! toolbar, list rows, sidebar rows and the switch live in `design/components/*.toml` (read
//! through `catalog.json`). libWolkit's view sheet is generated from them
//! (`crates/wolkit-render/tests/spec_sheet.rs`); this test holds the GTK Kit to the same numbers:
//! `shared::theme::scaffold`, `kit/kit.css` and `kit/widgets.rs`. A drift fails with both values.
//!
//! What GTK cannot draw at the spec's number is frozen in [`KNOWN`] with the measured value
//! (growth-only: a row that no longer differs fails until it is removed). Kit app windows draw
//! through libWolkit by default, which draws the spec's numbers exactly.
#[path = "support/rc.rs"]
mod rc;
use serde_json::Value;
use shared::theme::scaffold;

const CSS: &str = "crates/shell/src/kit/kit.css";
const WIDGETS: &str = "crates/shell/src/kit/widgets.rs";

/// (what, what GTK draws): the GTK fallback's known deviations from the spec.
const KNOWN: &[(&str, f64)] = &[
    // GAP-TABLE M1: `.list-row { min-height: 40px; padding: 6px 12px }` draws 52 (GTK's
    // min-height is the content box). DS-GTK-1 moves it to min-height 28.
    ("list-row standard row drawn by kit.css .list-row", 52.0),
    // GAP-TABLE M2: GtkSwitch is two thumbs wide (36 asked, 44 drawn) and `min-height: 24px;
    // padding: 2px` draws 28.
    ("switch regular height drawn by kit.css switch.toggle", 28.0),
];

fn catalog() -> Value {
    serde_json::from_str(&rc::read("design/components/catalog.json"))
        .expect("design/components/catalog.json")
}

fn spec(cat: &Value, id: &str, path: &str) -> f64 {
    let c = cat["components"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["id"] == id))
        .unwrap_or_else(|| panic!("no component {id}"));
    let mut v = c;
    for k in path.split('.') {
        v = &v[k];
    }
    v.as_f64()
        .unwrap_or_else(|| panic!("{id}.{path} is not a number"))
}

/// The declarations of the first rule whose selector line is exactly `selector`.
fn css_rule(css: &str, selector: &str) -> Vec<(String, String)> {
    let needle = format!("\n{selector} {{");
    let i = css
        .find(&needle)
        .unwrap_or_else(|| panic!("kit.css has no rule `{selector}`"));
    let body = &css[i + needle.len()..];
    let end = body.find('}').expect("rule ends");
    body[..end]
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

fn decl(rule: &[(String, String)], k: &str) -> String {
    rule.iter()
        .find(|(n, _)| n == k)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("no `{k}` in the rule"))
}

fn px(v: &str) -> f64 {
    v.trim_end_matches("px")
        .parse()
        .unwrap_or_else(|_| panic!("`{v}` is not a px length"))
}

/// `padding: Y X` (or one value) as (y, x).
fn padding(v: &str) -> (f64, f64) {
    let p: Vec<f64> = v.split_whitespace().map(px).collect();
    match p.as_slice() {
        [a] => (*a, *a),
        [y, x, ..] => (*y, *x),
        _ => panic!("padding `{v}`"),
    }
}

fn rust_const(src: &str, name: &str) -> f64 {
    let key = format!("pub const {name}: ");
    let line = src
        .lines()
        .find(|l| l.starts_with(&key))
        .unwrap_or_else(|| panic!("no `{key}` in the source"));
    let (_, val) = line.split_once('=').expect("a value");
    let val = val.trim().trim_end_matches(';');
    // `INSET = S3`: an alias of another constant of the same file; `NARROW = SIDEBAR_WIDTH +
    // APP_NARROW.0`: a sum of such names and of a pair's `.0` / `.1`.
    val.split('+')
        .map(|term| {
            let term = term.trim();
            term.parse().unwrap_or_else(|_| match term.rsplit_once('.') {
                Some((pair, "0")) => rust_pair(src, pair).0,
                Some((pair, "1")) => rust_pair(src, pair).1,
                _ => rust_const(src, term),
            })
        })
        .sum()
}

/// `pub const NAME: (i32, i32) = (W, H);` as (W, H).
fn rust_pair(src: &str, name: &str) -> (f64, f64) {
    let key = format!("pub const {name}: ");
    let line = src
        .lines()
        .find(|l| l.starts_with(&key))
        .unwrap_or_else(|| panic!("no `{key}`"));
    let val = line
        .split_once('=')
        .expect("a value")
        .1
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(';')
        .trim_end_matches(')');
    let (w, h) = val.split_once(',').expect("a pair");
    (
        w.trim().parse().expect("a number"),
        h.trim().parse().expect("a number"),
    )
}

fn drift(cat: &Value, css: &str, widgets: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let mut seen = Vec::new();
    let mut same = |what: &str, code: f64, want: f64| {
        if let Some(k) = KNOWN.iter().find(|k| k.0 == what) {
            seen.push(what.to_owned());
            if (code - k.1).abs() > 1e-9 {
                bad.push(format!(
                    "{what}: GTK draws {code}, KNOWN says {}: lower or remove the KNOWN row (spec {want})",
                    k.1
                ));
            }
            return;
        }
        if (code - want).abs() > 1e-9 {
            bad.push(format!("{what}: the code has {code}, the spec says {want}"));
        }
    };
    let s = |id: &str, p: &str| spec(cat, id, p);
    // toolbar and window chrome
    let toolbar = css_rule(css, ".toolbar");
    let (_, tpx) = padding(&decl(&toolbar, "padding"));
    same(
        "toolbar height: kit.css .toolbar min-height",
        px(&decl(&toolbar, "min-height")),
        s("toolbar", "geometry.height"),
    );
    same(
        "toolbar height: scaffold::KIT_TOOLBAR_HEIGHT",
        f64::from(scaffold::KIT_TOOLBAR_HEIGHT),
        s("toolbar", "geometry.height"),
    );
    same(
        "toolbar pad_x: kit.css .toolbar padding",
        tpx,
        s("toolbar", "geometry.pad_x"),
    );
    same(
        "toolbar pad_x: scaffold::TOOLBAR_PADDING",
        f64::from(scaffold::TOOLBAR_PADDING),
        s("toolbar", "geometry.pad_x"),
    );
    same(
        "toolbar gap: widgets.rs S2 (AppWindow bar)",
        rust_const(widgets, "S2"),
        s("toolbar", "geometry.gap"),
    );
    same(
        "window-chrome title_row: scaffold::KIT_TOOLBAR_HEIGHT",
        f64::from(scaffold::KIT_TOOLBAR_HEIGHT),
        s("window-chrome", "geometry.title_row"),
    );
    let inset = decl(&toolbar, "background");
    let inset = inset
        .split("calc(100% - ")
        .nth(1)
        .and_then(|r| r.split("px)").next())
        .map_or(-1.0, px);
    same(
        "window-chrome divider_inset: kit.css .toolbar line calc(100% - 2 x inset)",
        inset / 2.0,
        s("window-chrome", "geometry.divider_inset"),
    );
    same(
        "window-chrome page_pad: widgets.rs S4 (app_frame margins)",
        rust_const(widgets, "S4"),
        s("window-chrome", "geometry.page_pad"),
    );
    // list rows
    let row = css_rule(css, ".list-row");
    let (py, rpx) = padding(&decl(&row, "padding"));
    same(
        "list-row standard row: scaffold::LIST_ROW",
        f64::from(scaffold::LIST_ROW),
        s("list-row", "density.standard.row"),
    );
    same(
        "list-row standard row drawn by kit.css .list-row",
        px(&decl(&row, "min-height")) + 2.0 * py,
        s("list-row", "density.standard.row"),
    );
    same(
        "list-row card_pad_x: kit.css .list-row padding",
        rpx,
        s("list-row", "geometry.card_pad_x"),
    );
    same(
        "list-row card_radius: scaffold::RADIUS_CARD",
        f64::from(scaffold::RADIUS_CARD),
        s("list-row", "geometry.card_radius"),
    );
    // sidebar rows
    let side = css_rule(css, "list.sidebar row");
    same(
        "sidebar-row sidebar_width: scaffold::SIDEBAR_WIDTH",
        f64::from(scaffold::SIDEBAR_WIDTH),
        s("sidebar-row", "geometry.sidebar_width"),
    );
    same(
        "sidebar-row sidebar_width: widgets.rs SIDEBAR_WIDTH",
        rust_const(widgets, "SIDEBAR_WIDTH"),
        s("sidebar-row", "geometry.sidebar_width"),
    );
    same(
        "sidebar-row narrow: widgets.rs NARROW",
        rust_const(widgets, "NARROW"),
        s("sidebar-row", "geometry.narrow"),
    );
    same(
        "sidebar-row row: scaffold::SOURCE_ROW",
        f64::from(scaffold::SOURCE_ROW),
        s("sidebar-row", "geometry.row"),
    );
    same(
        "sidebar-row row: kit.css list.sidebar row min-height",
        px(&decl(&side, "min-height")),
        s("sidebar-row", "geometry.row"),
    );
    same(
        "sidebar-row radius: kit.css list.sidebar row border-radius",
        px(&decl(&side, "border-radius")),
        s("sidebar-row", "geometry.radius"),
    );
    // switch
    let sw = css_rule(css, "switch.toggle");
    let (sp, _) = padding(&decl(&sw, "padding"));
    same(
        "switch regular width: kit.css switch.toggle min-width",
        px(&decl(&sw, "min-width")),
        s("switch", "sizes.regular.width"),
    );
    same(
        "switch regular height drawn by kit.css switch.toggle",
        px(&decl(&sw, "min-height")) + 2.0 * sp,
        s("switch", "sizes.regular.height"),
    );
    same(
        "switch padding: kit.css switch.toggle padding",
        sp,
        s("switch", "sizes.regular.padding"),
    );
    for k in KNOWN {
        if !seen.iter().any(|s| s == k.0) {
            bad.push(format!("KNOWN row `{}` was not measured: remove it", k.0));
        }
    }
    bad
}

#[test]
fn the_kit_holds_the_component_specs() {
    let bad = drift(&catalog(), &rc::read(CSS), &rc::read(WIDGETS));
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

#[test]
fn a_spec_change_is_caught() {
    let mut cat = catalog();
    for c in cat["components"].as_array_mut().into_iter().flatten() {
        if c["id"] == "toolbar" {
            c["geometry"]["height"] = Value::from(44);
        }
    }
    let bad = drift(&cat, &rc::read(CSS), &rc::read(WIDGETS));
    assert!(
        bad.iter()
            .any(|b| b.contains("toolbar height") && b.contains("44")),
        "{bad:?}"
    );
}
