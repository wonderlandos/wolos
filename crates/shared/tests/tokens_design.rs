// SPDX-License-Identifier: BSD-2-Clause
//! Design Tokens as data (DS-TOK, ADR 0058): `design/tokens/{type,space,elevation,motion,density}.json`
//! are the one source of the type, space, radius, elevation, motion and density numbers. This test
//! holds every place that still carries a copy to them: `shared::theme::{SCALE, scaffold}`, the
//! `:root` block and the type rules of the Kit stylesheet (`crates/shell/src/kit/kit.css`), the
//! constants of `kit/widgets.rs` and the durations and curves of `kit/motion.rs`, plus the radius
//! block of `colors.json`. A constant that drifts fails with the file, the name and both values.
//! No value moved when the JSON was written: it records what the code already drew.
#[path = "support/rc.rs"]
mod rc;
use serde_json::Value;
use shared::theme::{SCALE, scaffold};

const CSS: &str = "crates/shell/src/kit/kit.css";
const WIDGETS: &str = "crates/shell/src/kit/widgets.rs";
const MOTION: &str = "crates/shell/src/kit/motion.rs";

struct Tokens {
    ty: Value,
    space: Value,
    elev: Value,
    motion: Value,
    density: Value,
    colors: Value,
}

fn json(name: &str) -> Value {
    let text = rc::read(&format!("design/tokens/{name}.json"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("design/tokens/{name}.json: {e}"))
}

fn tokens() -> Tokens {
    Tokens {
        ty: json("type"),
        space: json("space"),
        elev: json("elevation"),
        motion: json("motion"),
        density: json("density"),
        colors: json("colors"),
    }
}

/// A number of the JSON at `path` ("space.s3"), as f64; panics when it is missing.
fn num(v: &Value, path: &str) -> f64 {
    let mut cur = v;
    for key in path.split('.') {
        cur = match key.parse::<usize>() {
            Ok(i) if cur.is_array() => &cur[i],
            _ => &cur[key],
        };
    }
    cur.as_f64()
        .unwrap_or_else(|| panic!("tokens: {path} is not a number"))
}

/// `a` equals `b` as numbers; records a sentence otherwise.
fn same(bad: &mut Vec<String>, what: &str, code: f64, token: f64) {
    if (code - token).abs() > 1e-9 {
        bad.push(format!(
            "{what}: the code has {code}, the token says {token}"
        ));
    }
}

/// `--name: value;` pairs of the first `:root { ... }` block, value with all spaces removed.
fn css_root(css: &str) -> Vec<(String, String)> {
    let start = css.find(":root {").expect("kit.css has a :root block") + ":root {".len();
    let body = &css[start..];
    let end = body.find("\n}").expect(":root block ends");
    let mut out = Vec::new();
    let mut text = String::new();
    let mut depth_comment = false;
    for line in body[..end].lines() {
        // Comments span lines; keep only code.
        let mut code = String::new();
        let mut rest = line;
        loop {
            if depth_comment {
                match rest.find("*/") {
                    Some(i) => {
                        rest = &rest[i + 2..];
                        depth_comment = false;
                    }
                    None => break,
                }
            } else {
                match rest.find("/*") {
                    Some(i) => {
                        code.push_str(&rest[..i]);
                        rest = &rest[i + 2..];
                        depth_comment = true;
                    }
                    None => {
                        code.push_str(rest);
                        break;
                    }
                }
            }
        }
        text.push_str(&code);
        text.push(' ');
    }
    for decl in text.split(';') {
        if let Some((k, v)) = decl.split_once(':') {
            let k = k.trim();
            if k.starts_with("--") {
                out.push((
                    k.to_owned(),
                    v.split_whitespace().collect::<Vec<_>>().join(""),
                ));
            }
        }
    }
    out
}

/// The declarations of the first rule whose selector list is exactly `selector`.
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

fn px(v: &str) -> f64 {
    v.trim_end_matches("px")
        .parse()
        .unwrap_or_else(|_| panic!("`{v}` is not a px length"))
}

fn ms(v: &str) -> f64 {
    v.trim_end_matches("ms")
        .parse()
        .unwrap_or_else(|_| panic!("`{v}` is not a duration"))
}

/// `pub const NAME: i32 = N;` in a Rust source text.
fn rust_const(src: &str, name: &str) -> f64 {
    let key = format!("pub const {name}: ");
    let line = src
        .lines()
        .find(|l| l.starts_with(&key))
        .unwrap_or_else(|| panic!("no `{key}` in the source"));
    let (_, val) = line.split_once('=').unwrap();
    let val = val.trim().trim_end_matches(';');
    // `INSET = S3`: an alias of another constant of the same file.
    val.parse().unwrap_or_else(|_| rust_const(src, val))
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
        .unwrap()
        .1
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(';')
        .trim_end_matches(')');
    let (w, h) = val.split_once(',').unwrap();
    (w.trim().parse().unwrap(), h.trim().parse().unwrap())
}

/// `[a, b, c, d]` inside the match arm `Curve::<Name> => Some([..])`.
fn curve_arm(src: &str, name: &str) -> Option<Vec<f64>> {
    let key = format!("Curve::{name} => ");
    // The arm of `bezier()`; `name()` and `css()` have arms for the same variants.
    let line = src.lines().find(|l| {
        let l = l.trim_start();
        l.starts_with(&key) && (l.contains("Some([") || l.contains("=> None"))
    })?;
    let rest = line.split_once("=>").unwrap().1.trim();
    if rest.starts_with("None") {
        return None;
    }
    let inner = rest.trim_start_matches("Some([").split(']').next().unwrap();
    Some(
        inner
            .split(',')
            .map(|x| x.trim().parse().unwrap())
            .collect(),
    )
}

fn scale(var: &str) -> &'static str {
    SCALE
        .iter()
        .find(|(k, _)| *k == var)
        .unwrap_or_else(|| panic!("{var} is not in SCALE"))
        .1
}

/// Every drift between the tokens and the code, as sentences (empty when they agree).
fn drift(t: &Tokens, css: &str, widgets: &str, motion_rs: &str) -> Vec<String> {
    let mut bad = Vec::new();

    // shared::theme::SCALE
    for (var, tok) in [
        ("--radius-s", "radius.control"),
        ("--radius-m", "radius.card"),
        ("--radius-l", "radius.panel"),
        ("--radius-xl", "radius.sheet"),
        ("--space-1", "space.s1"),
        ("--space-2", "space.s2"),
        ("--space-3", "space.s3"),
        ("--space-4", "space.s4"),
        ("--space-6", "space.s6"),
        ("--blur", "blur"),
    ] {
        same(
            &mut bad,
            &format!("shared::theme::SCALE {var}"),
            px(scale(var)),
            num(&t.space, tok),
        );
    }
    same(
        &mut bad,
        "shared::theme::SCALE --font-size",
        px(scale("--font-size")),
        num(&t.ty, "theme-contract.font-size"),
    );
    same(
        &mut bad,
        "shared::theme::SCALE --font-size-title",
        px(scale("--font-size-title")),
        num(&t.ty, "theme-contract.font-size-title"),
    );
    for (var, tok) in [
        ("--motion-fast", "fast"),
        ("--motion-normal", "normal"),
        ("--motion-slow", "slow"),
    ] {
        same(
            &mut bad,
            &format!("shared::theme::SCALE {var}"),
            ms(scale(var)),
            num(&t.motion, &format!("duration.{tok}")),
        );
    }
    let ease: Vec<f64> = scale("--ease")
        .trim_start_matches("cubic-bezier(")
        .trim_end_matches(')')
        .split(',')
        .map(|x| x.trim().parse().unwrap())
        .collect();
    for (i, v) in ease.iter().enumerate() {
        same(
            &mut bad,
            &format!("shared::theme::SCALE --ease[{i}]"),
            *v,
            num(&t.motion, &format!("curve.ease.{i}")),
        );
    }

    // shared::theme::scaffold
    let d = |m: &str| num(&t.density, &format!("metrics.{m}.standard"));
    for (what, code, tok) in [
        (
            "TOOLBAR_HEIGHT",
            scaffold::TOOLBAR_HEIGHT as f64,
            num(&t.space, "surface.toolbar-compositor"),
        ),
        (
            "KIT_TOOLBAR_HEIGHT",
            scaffold::KIT_TOOLBAR_HEIGHT as f64,
            num(&t.space, "surface.toolbar"),
        ),
        (
            "KIT_TOOLBAR_HEIGHT (density toolbar)",
            scaffold::KIT_TOOLBAR_HEIGHT as f64,
            d("toolbar"),
        ),
        (
            "TOOLBAR_PADDING",
            scaffold::TOOLBAR_PADDING as f64,
            num(&t.space, "surface.toolbar-padding"),
        ),
        (
            "SIDEBAR_WIDTH",
            scaffold::SIDEBAR_WIDTH as f64,
            num(&t.space, "surface.sidebar"),
        ),
        (
            "SOURCE_ROW + gap (density sidebar-pitch)",
            scaffold::SOURCE_ROW as f64 + num(&t.space, "space.s1"),
            d("sidebar-pitch"),
        ),
        (
            "LIST_ROW (density row)",
            scaffold::LIST_ROW as f64,
            d("row"),
        ),
        (
            "CONTROL_HEIGHT (density control)",
            scaffold::CONTROL_HEIGHT as f64,
            d("control"),
        ),
        (
            "RADIUS_CONTROL",
            scaffold::RADIUS_CONTROL as f64,
            num(&t.space, "radius.control"),
        ),
        (
            "RADIUS_CARD",
            scaffold::RADIUS_CARD as f64,
            num(&t.space, "radius.card"),
        ),
        (
            "WINDOW_RADIUS",
            scaffold::WINDOW_RADIUS as f64,
            num(&t.space, "radius.window"),
        ),
        ("BODY", scaffold::BODY as f64, num(&t.ty, "roles.body.size")),
        (
            "SECONDARY",
            scaffold::SECONDARY as f64,
            num(&t.ty, "roles.caption.size"),
        ),
        (
            "HEADING",
            scaffold::HEADING as f64,
            num(&t.ty, "roles.heading.size"),
        ),
        (
            "LARGE_TITLE",
            scaffold::LARGE_TITLE as f64,
            num(&t.ty, "roles.large-title.size"),
        ),
    ] {
        same(
            &mut bad,
            &format!("shared::theme::scaffold::{what}"),
            code,
            tok,
        );
    }

    // colors.json radius block (tokens.py)
    for (k, tok) in [
        ("sm", "control"),
        ("md", "card"),
        ("lg", "panel"),
        ("window", "window"),
        ("dock", "sheet"),
    ] {
        let v = t.colors["radius"][k]
            .as_str()
            .unwrap_or_else(|| panic!("colors.json radius.{k}"));
        same(
            &mut bad,
            &format!("colors.json radius.{k}"),
            px(v),
            num(&t.space, &format!("radius.{tok}")),
        );
    }

    // kit.css :root
    let root = css_root(css);
    let get = |k: &str| -> &str {
        &root
            .iter()
            .find(|(n, _)| n == k)
            .unwrap_or_else(|| panic!("kit.css :root has no {k}"))
            .1
    };
    for (var, tok) in [
        ("--glass-radius", "radius.sheet"),
        ("--glass-radius-m", "radius.panel"),
        ("--glass-radius-s", "radius.card"),
        ("--inset", "inset"),
    ] {
        same(
            &mut bad,
            &format!("kit.css :root {var}"),
            px(get(var)),
            num(&t.space, tok),
        );
    }
    for (var, tok) in [
        ("--motion-fast", "fast"),
        ("--motion-normal", "normal"),
        ("--motion-slow", "slow"),
        ("--glass-motion", "glass"),
    ] {
        same(
            &mut bad,
            &format!("kit.css :root {var}"),
            ms(get(var)),
            num(&t.motion, &format!("duration.{tok}")),
        );
    }
    let ink: Vec<String> = t.elev["ink"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.to_string())
        .collect();
    for lvl in ["1", "2", "3"] {
        let l = &t.elev["levels"][lvl];
        let want = format!(
            "{}px{}px{}pxrgba({},{})",
            l["x"],
            l["y"],
            l["blur"],
            ink.join(","),
            l["alpha"]
        );
        let have = get(&format!("--elev-{lvl}"));
        if have != want {
            bad.push(format!(
                "kit.css :root --elev-{lvl}: the code has {have}, the token says {want}"
            ));
        }
    }
    for (var, curve) in t.motion["css"].as_object().unwrap() {
        let name = curve.as_str().unwrap();
        let c: Vec<f64> = t.motion["curve"][name]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        let have: Vec<f64> = get(var)
            .trim_start_matches("cubic-bezier(")
            .trim_end_matches(')')
            .split(',')
            .map(|x| x.parse().unwrap())
            .collect();
        if have != c {
            bad.push(format!(
                "kit.css :root {var}: the code has {have:?}, the token curve `{name}` is {c:?}"
            ));
        }
    }

    // kit.css type rules
    let body = css_rule(css, "window, popover, tooltip");
    let find = |rule: &[(String, String)], k: &str| {
        rule.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    };
    same(
        &mut bad,
        "kit.css body font-size",
        px(&find(&body, "font-size").unwrap()),
        num(&t.ty, "roles.body.size"),
    );
    same(
        &mut bad,
        "kit.css body line-height",
        px(&find(&body, "line-height").unwrap()),
        num(&t.ty, "roles.body.line"),
    );
    same(
        &mut bad,
        "kit.css label.title font-weight",
        find(&css_rule(css, "label.title"), "font-weight")
            .unwrap()
            .parse()
            .unwrap(),
        num(&t.ty, "roles.title.weight"),
    );
    for (sel, role, has_weight, has_track) in [
        ("label.secondary, .secondary label", "caption", false, false),
        ("label.eyebrow", "eyebrow", true, true),
        ("label.heading", "heading", true, false),
        ("label.large-title", "large-title", true, true),
        (".empty-state label.large-title", "empty-state", true, true),
    ] {
        let r = css_rule(css, sel);
        let what = format!("kit.css `{sel}`");
        same(
            &mut bad,
            &format!("{what} font-size"),
            px(&find(&r, "font-size").unwrap()),
            num(&t.ty, &format!("roles.{role}.size")),
        );
        same(
            &mut bad,
            &format!("{what} line-height"),
            px(&find(&r, "line-height").unwrap()),
            num(&t.ty, &format!("roles.{role}.line")),
        );
        if has_weight {
            same(
                &mut bad,
                &format!("{what} font-weight"),
                find(&r, "font-weight").unwrap().parse().unwrap(),
                num(&t.ty, &format!("roles.{role}.weight")),
            );
        }
        if has_track {
            same(
                &mut bad,
                &format!("{what} letter-spacing"),
                px(&find(&r, "letter-spacing").unwrap()),
                num(&t.ty, &format!("roles.{role}.tracking")),
            );
        }
    }
    // kit/widgets.rs
    for (name, tok) in [
        ("S1", "space.s1"),
        ("S2", "space.s2"),
        ("S3", "space.s3"),
        ("S4", "space.s4"),
        ("S6", "space.s6"),
        ("S10", "space.s10"),
        ("S15", "space.s15"),
        ("S21", "space.s21"),
        ("INSET", "inset"),
        ("ICON", "icon.row"),
        ("ICON_TOOL", "icon.tool"),
        ("ICON_M", "icon.m"),
        ("ICON_L", "icon.l"),
        ("ICON_XL", "icon.xl"),
        ("DOCK_TILE", "surface.dock-tile"),
        ("MENUBAR_HEIGHT", "surface.menubar"),
        ("CONTROL_CENTER_WIDTH", "surface.control-center"),
        ("SEARCHBAR_WIDTH", "surface.searchbar"),
        ("SIDEBAR_WIDTH", "surface.sidebar"),
        ("NARROW", "surface.narrow-column"),
    ] {
        same(
            &mut bad,
            &format!("kit/widgets.rs {name}"),
            rust_const(widgets, name),
            num(&t.space, tok),
        );
    }
    for (name, tok) in [("APP_SIZE", "app"), ("APP_NARROW", "app-narrow")] {
        let (w, h) = rust_pair(widgets, name);
        same(
            &mut bad,
            &format!("kit/widgets.rs {name} width"),
            w,
            num(&t.space, &format!("surface.{tok}.0")),
        );
        same(
            &mut bad,
            &format!("kit/widgets.rs {name} height"),
            h,
            num(&t.space, &format!("surface.{tok}.1")),
        );
    }

    // kit/motion.rs
    let line = motion_rs
        .lines()
        .find(|l| l.starts_with("pub const DURATIONS_MS"))
        .expect("DURATIONS_MS");
    let ds: Vec<f64> = line
        .split('[')
        .nth(2)
        .unwrap()
        .split(']')
        .next()
        .unwrap()
        .split(',')
        .map(|x| x.trim().parse().unwrap())
        .collect();
    for (i, tok) in ["fast", "normal", "slow"].iter().enumerate() {
        same(
            &mut bad,
            &format!("kit/motion.rs DURATIONS_MS[{i}]"),
            ds[i],
            num(&t.motion, &format!("duration.{tok}")),
        );
    }
    for (arm, name) in [
        ("Ease", "ease"),
        ("Linear", "linear"),
        ("In", "in"),
        ("Out", "out"),
        ("InOut", "in-out"),
        ("Glass", "glass"),
        ("Overshoot", "overshoot"),
        ("Soft", "soft"),
    ] {
        let code = curve_arm(motion_rs, arm);
        let tok = t.motion["curve"][name]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>());
        // f32 literals in the code are the same decimals; compare as written.
        if code != tok {
            bad.push(format!("kit/motion.rs Curve::{arm}: the code has {code:?}, the token curve `{name}` is {tok:?}"));
        }
    }
    bad
}

#[test]
fn the_code_holds_the_design_tokens() {
    let bad = drift(
        &tokens(),
        &rc::read(CSS),
        &rc::read(WIDGETS),
        &rc::read(MOTION),
    );
    assert!(
        bad.is_empty(),
        "design/tokens/*.json and the code disagree:\n  {}",
        bad.join("\n  ")
    );
}

/// The check cannot pass on nothing: each plant below is a drift in one place, and it must be named.
#[test]
fn drift_is_caught() {
    let (css, widgets, motion) = (rc::read(CSS), rc::read(WIDGETS), rc::read(MOTION));
    let plants: [(&str, &str, &str, &str); 6] = [
        (
            "--inset: 12px",
            "--inset: 16px",
            "kit.css :root --inset",
            "css",
        ),
        (
            "--elev-2: 2px 4px 12px",
            "--elev-2: 2px 4px 14px",
            "kit.css :root --elev-2",
            "css",
        ),
        (
            "label.heading { font-size: 16px",
            "label.heading { font-size: 17px",
            "`label.heading` font-size",
            "css",
        ),
        (
            "pub const S4: i32 = 16;",
            "pub const S4: i32 = 20;",
            "kit/widgets.rs S4",
            "widgets",
        ),
        (
            "[150, 250, 400]",
            "[150, 250, 500]",
            "DURATIONS_MS[2]",
            "motion",
        ),
        (
            "Some([0.4, 0.0, 1.0, 1.0])",
            "Some([0.5, 0.0, 1.0, 1.0])",
            "Curve::In",
            "motion",
        ),
    ];
    for (from, to, want, which) in plants {
        let (c, w, m) = match which {
            "css" => (css.replacen(from, to, 1), widgets.clone(), motion.clone()),
            "widgets" => (css.clone(), widgets.replacen(from, to, 1), motion.clone()),
            _ => (css.clone(), widgets.clone(), motion.replacen(from, to, 1)),
        };
        assert!(
            [&c, &w, &m]
                .iter()
                .zip([&css, &widgets, &motion])
                .any(|(a, b)| *a != b),
            "plant `{from}` did not apply: the source changed"
        );
        let bad = drift(&tokens(), &c, &w, &m);
        assert!(
            bad.iter().any(|s| s.contains(want)),
            "plant `{to}` was not caught ({want}): {bad:?}"
        );
    }
    // A token that drifts from the code is caught from the other side.
    let mut t = tokens();
    t.space["radius"]["card"] = serde_json::json!(16);
    let bad = drift(&t, &css, &widgets, &motion);
    assert!(
        bad.iter().any(|s| s.contains("RADIUS_CARD"))
            && bad.iter().any(|s| s.contains("--radius-m")),
        "{bad:?}"
    );
}
