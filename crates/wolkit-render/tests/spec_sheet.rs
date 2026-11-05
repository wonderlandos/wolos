// SPDX-License-Identifier: BSD-2-Clause
//! One source (WOLKIT-1TO1): the numbers of the parts the Kit page draws live in the Design System
//! specs (`design/components/*.toml`, read through `catalog.json`), and the generated block of
//! `src/view.wolcss` is written from them. The block is checked here: a spec number that changes
//! without the sheet, or a hand edit of the sheet, fails. `WOL_SPEC_UPDATE=1` rewrites the block.
//!
//! The same specs hold the GTK Kit (`crates/shared/tests/components_design.rs`), so a number has
//! one home and two checked readers.
use serde_json::Value;
use std::path::PathBuf;

const BEGIN: &str = "/* BEGIN generated from design/components/catalog.json by tests/spec_sheet.rs (WOL_SPEC_UPDATE=1); do not edit */\n";
const END: &str = "/* END generated */\n";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn catalog() -> Option<Value> {
    // WOL_CATALOG points at another checkout's catalog (the design-system parts are specs of
    // DS-COMPONENTS until their branch is merged here).
    let p = std::env::var_os("WOL_CATALOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("design/components/catalog.json"));
    let text = std::fs::read_to_string(&p).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
}

/// A whole-px number of component `id` at `path` ("geometry.title_row").
fn n(cat: &Value, id: &str, path: &str) -> i64 {
    let spec = cat["components"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["id"] == id))
        .unwrap_or_else(|| panic!("no component {id} in the catalog"));
    let mut v = spec;
    for k in path.split('.') {
        v = &v[k];
    }
    v.as_i64()
        .unwrap_or_else(|| panic!("{id}.{path} is not a whole number"))
}

/// A line-height multiple, short and exact enough that `size * multiple` lands on the token's line.
fn fmt_ratio(r: f64) -> String {
    let s = format!("{r:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn type_tokens() -> Value {
    let p = root().join("design/tokens/type.json");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The generated rules, from the specs and the type tokens.
fn sheet(cat: &Value) -> String {
    let ty = type_tokens();
    let chrome = |k: &str| n(cat, "window-chrome", &format!("geometry.{k}"));
    let list = |k: &str| n(cat, "list-row", k);
    let (pad, gap) = (chrome("page_pad"), chrome("page_gap"));
    let row = list("density.standard.row");
    let mut s = String::new();
    s.push_str(&format!(
        "/* window-chrome: the page pads left, right and bottom; the title row is the bar plus the page gap. */\n\
         .page {{ padding: 0 {pad}px {pad}px {pad}px; gap: {gap}px; background: surface; }}\n\
         .titlebar {{ min-height: {}px; }}\n",
        chrome("title_row") - gap
    ));
    s.push_str(&format!(
        "/* list-row: the card and its rows at the standard density. */\n\
         .list {{ background: card; border-radius: {}px; padding: {}px {}px; box-shadow: sm; }}\n\
         .list-row {{ min-height: {row}px; align-items: center; gap: {}px; }}\n\
         .list-field {{ min-height: {row}px; justify-content: center; }}\n",
        list("geometry.card_radius"),
        list("geometry.card_pad_y"),
        list("geometry.card_pad_x"),
        list("geometry.gap"),
    ));
    s.push_str(&format!(
        "/* switch: the regular size. */\n\
         toggle {{ width: {}px; height: {}px; }}\n",
        n(cat, "switch", "sizes.regular.width"),
        n(cat, "switch", "sizes.regular.height"),
    ));
    // label: the six roles, as the Kit draws them (kit/data.rs role classes, kit.css), with the
    // numbers of design/tokens/type.json (label.toml says which token each role reads).
    s.push_str(
        "/* label: the six type roles (design/tokens/type.json). */\n.icon { color: text; }\n",
    );
    for (role, token, dim) in [
        ("display", "display", false),
        ("large", "large-title", false),
        ("title", "title", false),
        ("body", "body", false),
        ("secondary", "caption", true),
        ("caption", "eyebrow", false),
    ] {
        let t = &ty["roles"][token];
        let f = |k: &str| {
            t[k].as_f64()
                .unwrap_or_else(|| panic!("type.json roles.{token}.{k}"))
        };
        let (size, line, weight, track) = (f("size"), f("line"), f("weight"), f("tracking"));
        let mut rule = format!(
            ".role-{role} {{ font-size: {size}px; line-height: {}; font-weight: {weight};",
            fmt_ratio(line / size)
        );
        if track != 0.0 {
            rule.push_str(&format!(" letter-spacing: {track}px;"));
        }
        if dim {
            rule.push_str(" color: dim;");
        }
        rule.push_str(" }\n");
        s.push_str(&rule);
    }
    s
}

fn split(text: &str) -> (&str, &str, &str) {
    let b = text.find(BEGIN).expect("view.wolcss has the BEGIN line") + BEGIN.len();
    let e = text[b..].find(END).expect("view.wolcss has the END line") + b;
    (&text[..b], &text[b..e], &text[e..])
}

#[test]
fn the_view_sheet_is_generated_from_the_specs() {
    let Some(cat) = catalog() else {
        println!("note: no design/components/catalog.json (design submodule not checked out)");
        return;
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/view.wolcss");
    let text = std::fs::read_to_string(&path).expect("view.wolcss");
    let (head, have, tail) = split(&text);
    let want = sheet(&cat);
    if std::env::var_os("WOL_SPEC_UPDATE").is_some() {
        std::fs::write(&path, format!("{head}{want}{tail}")).expect("write view.wolcss");
        return;
    }
    assert_eq!(
        have, want,
        "src/view.wolcss is not what design/components says: run WOL_SPEC_UPDATE=1 cargo test -p wolkit-render --test spec_sheet"
    );
}

#[test]
fn a_changed_spec_number_is_caught() {
    let Some(mut cat) = catalog() else {
        return;
    };
    let before = sheet(&cat);
    for c in cat["components"].as_array_mut().into_iter().flatten() {
        if c["id"] == "list-row" {
            c["density"]["standard"]["row"] = Value::from(52);
        }
    }
    let after = sheet(&cat);
    assert_ne!(before, after);
    assert!(after.contains("min-height: 52px"), "{after}");
}

/// Whether the catalog has component `id`.
fn has(cat: &Value, id: &str) -> bool {
    cat["components"]
        .as_array()
        .is_some_and(|a| a.iter().any(|c| c["id"] == id))
}

/// The rule of `.class` in `parts.wolcss` (its declarations), the first one written.
fn rule<'a>(sheet: &'a str, class: &str) -> &'a str {
    let at = sheet
        .find(&format!("\n.{class} {{"))
        .unwrap_or_else(|| panic!("parts.wolcss has no .{class}"));
    let from = &sheet[at..];
    let open = from.find('{').unwrap() + 1;
    let close = from.find('}').unwrap();
    &from[open..close]
}

fn px(rule: &str, prop: &str) -> i64 {
    let at = rule
        .find(&format!(" {prop}: "))
        .unwrap_or_else(|| panic!("no {prop} in {rule}"));
    let v = &rule[at + prop.len() + 3..];
    v[..v.find("px").expect("px")]
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("{prop} in {rule}"))
}

/// The design-system parts of `src/parts.wolcss` hold the numbers the specs give as fields (their
/// radii, gaps and the hairline are anatomy prose). Skipped, with a note, while the checkout has
/// not got the specs (DS-COMPONENTS merges them).
#[test]
fn the_design_system_parts_follow_the_specs() {
    let Some(cat) = catalog() else {
        println!("note: no catalog.json");
        return;
    };
    let ids = [
        "checkbox",
        "radio",
        "chip",
        "tabs",
        "card",
        "disclosure",
        "divider",
    ];
    if !ids.iter().all(|i| has(&cat, i)) {
        println!(
            "note: the catalog lacks the design-system specs (WOL_CATALOG=...); parts not held"
        );
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/parts.wolcss");
    let sheet = std::fs::read_to_string(&path).expect("parts.wolcss");
    let r = |c: &str| rule(&sheet, c);
    // Checkbox: the box, and the row it sits in at the regular control height.
    assert_eq!(
        px(r("check"), "width"),
        n(&cat, "checkbox", "sizes.regular.box")
    );
    assert_eq!(
        px(r("check"), "height"),
        n(&cat, "checkbox", "sizes.regular.box")
    );
    assert_eq!(
        px(r("check-on"), "width"),
        n(&cat, "checkbox", "sizes.regular.box")
    );
    assert_eq!(
        px(r("part-row"), "min-height"),
        n(&cat, "checkbox", "sizes.regular.height")
    );
    assert_eq!(
        px(r("part-row"), "min-height"),
        n(&cat, "radio", "sizes.regular.height")
    );
    assert_eq!(
        px(r("radio"), "width"),
        n(&cat, "radio", "sizes.regular.ring")
    );
    assert_eq!(
        px(r("radio-on"), "height"),
        n(&cat, "radio", "sizes.regular.ring")
    );
    // Chip: the regular height.
    for c in ["chip", "chip-on", "chip-ghost"] {
        assert_eq!(
            px(r(c), "min-height"),
            n(&cat, "chip", "sizes.regular.height"),
            "{c}"
        );
    }
    // Tabs: the tab (its label row and the 2 px indicator under it) is the spec's tab height.
    assert_eq!(
        px(r("view-tab"), "min-height") + px(r("tab-line-on"), "min-height"),
        n(&cat, "tabs", "sizes.tab.height")
    );
    // Card: the inset on all four sides at the standard density.
    assert_eq!(
        px(r("surface-card"), "padding"),
        n(&cat, "card", "density.standard.inset")
    );
    // Disclosure: the header is a row of the standard density.
    assert_eq!(
        px(r("disclosure-head"), "min-height"),
        n(&cat, "disclosure", "density.standard.row")
    );
    // Divider: one pixel (the kit sheet's `divider` rule).
    assert_eq!(n(&cat, "divider", "sizes.line.height"), 1);
}
