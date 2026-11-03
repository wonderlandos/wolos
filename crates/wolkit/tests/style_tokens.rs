// SPDX-License-Identifier: BSD-2-Clause
//! Drift tests: the style system's tokens against the places they come from. When the theme
//! contract, the Kit stylesheet or the design tokens move, these fail until the style system
//! follows (or the other way round).
use shared::theme::{CONTRAST_RULES, SCALE, TOKENS, scaffold};
use wolkit::style::tokens::*;
use wolkit::style::value::ColorRef;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let p = root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn scale(var: &str) -> &'static str {
    SCALE
        .iter()
        .find(|(k, _)| *k == var)
        .unwrap_or_else(|| panic!("{var} is not in shared::theme::SCALE"))
        .1
}

fn px(s: &str) -> f32 {
    s.trim_end_matches("px")
        .trim_end_matches("ms")
        .parse()
        .unwrap()
}

#[test]
fn the_thirteen_tokens_are_the_theme_contract() {
    assert_eq!(Token::NAMES.len(), TOKENS.len());
    for (i, name) in TOKENS.iter().enumerate() {
        assert_eq!(Token::NAMES[i], *name, "token {i}");
        assert_eq!(Token::ALL[i].name(), *name);
        assert_eq!(Token::parse(name), Some(Token::ALL[i]));
        assert_eq!(
            Token::from_u8(i as u8),
            Some(Token::ALL[i]),
            "ids are positions"
        );
    }
    assert_eq!(Token::from_u8(13), None);
    assert_eq!(
        wolkit::scene::prop::TOKENS as usize,
        TOKENS.len(),
        "the wire accepts exactly the 13 ids"
    );
}

#[test]
fn every_builtin_theme_defines_all_thirteen() {
    let dir = root().join("crates/core/src/theme/builtin");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let toks = v["tokens"]
            .as_object()
            .unwrap_or_else(|| panic!("{}: no tokens", p.display()));
        for t in TOKENS {
            assert!(toks.contains_key(t), "{} lacks `{t}`", p.display());
        }
        assert_eq!(
            toks.len(),
            TOKENS.len(),
            "{} has tokens outside the contract",
            p.display()
        );
        n += 1;
    }
    assert!(n >= 20, "found {n} themes");
}

#[test]
fn scales_mirror_the_theme_scale() {
    for s in SPACE.iter().chain(RADIUS.iter()).chain(MOTION.iter()) {
        if let Some(var) = s.var {
            assert_eq!(px(scale(var)), s.value, "{} vs {var}", s.name);
        }
    }
    // Every spacing, radius and motion variable of the contract has a name.
    for (k, _) in SCALE {
        if k.starts_with("--space-") || k.starts_with("--radius-") || k.starts_with("--motion-") {
            let named = SPACE
                .iter()
                .chain(RADIUS.iter())
                .chain(MOTION.iter())
                .any(|s| s.var == Some(k));
            assert!(named, "{k} has no name in style::tokens");
        }
    }
    assert_eq!(motion_ms("normal"), Some(250));
    // The ease is the theme's.
    assert_eq!(scale("--ease"), "cubic-bezier(0.2, 0.8, 0.2, 1)");
    let b = wolkit::style::transition::bezier(wolkit::style::value::Ease::Standard).unwrap();
    assert_eq!(b, [0.2, 0.8, 0.2, 1.0]);
}

#[test]
fn type_roles_mirror_the_scaffold() {
    assert_eq!(role("body").unwrap().size, scaffold::BODY as f32);
    assert_eq!(role("secondary").unwrap().size, scaffold::SECONDARY as f32);
    assert_eq!(role("large").unwrap().size, scaffold::LARGE_TITLE as f32);
    assert_eq!(role("title").unwrap().size, px(scale("--font-size-title")));
    assert_eq!(role("body").unwrap().size, px(scale("--font-size")));
    assert!(
        MIN_FONT_PX <= ROLES.iter().map(|r| r.size).fold(f32::MAX, f32::min),
        "no role is below the smallest font (RU-059)"
    );
    // Roles are the same set as shared::view::Role.
    for r in &ROLES {
        assert!(
            shared::view::Role::parse(r.name).is_some(),
            "{} is not a view role",
            r.name
        );
    }
    assert_eq!(ROLES.len(), 6);
}

#[test]
fn design_tokens_agree() {
    let v: serde_json::Value = serde_json::from_str(&read("design/tokens/colors.json")).unwrap();
    assert_eq!(
        px(v["radius"]["sm"].as_str().unwrap()),
        radius_value("s").unwrap()
    );
    assert_eq!(
        px(v["radius"]["md"].as_str().unwrap()),
        radius_value("m").unwrap()
    );
    assert_eq!(
        px(v["radius"]["lg"].as_str().unwrap()),
        radius_value("l").unwrap()
    );
    assert_eq!(
        px(v["font"]["size"].as_str().unwrap()),
        role("body").unwrap().size
    );
    // The accent of the built-in themes is the design accent.
    for (theme, key) in [("wol-light", "light"), ("wol-dark", "dark")] {
        let t: serde_json::Value = serde_json::from_str(&read(&format!(
            "crates/core/src/theme/builtin/{theme}.json"
        )))
        .unwrap();
        assert_eq!(
            t["tokens"]["accent"].as_str().unwrap().to_lowercase(),
            v["accent"][key].as_str().unwrap().to_lowercase(),
            "{theme}"
        );
    }
}

// ---- contrast.css

/// `--name: value;` declarations inside `:root { ... }`.
fn root_vars(css: &str) -> Vec<(String, String)> {
    let start = css.find(":root {").expect(":root block");
    let body = &css[start + 7..];
    let end = body.find('}').unwrap();
    body[..end]
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, _)| k.starts_with("--"))
        .collect()
}

#[test]
fn contrast_overrides_mirror_contrast_css() {
    let css = read("crates/shell/src/kit/contrast.css");
    let vars = root_vars(&css);
    let mut seen = vec![];
    for (k, v) in &vars {
        let name = k.trim_start_matches("--");
        let Some(tok) = Token::parse(name) else {
            continue;
        }; // --control-off is not a token
        let o = CONTRAST.iter().find(|o| o.token == tok).unwrap_or_else(|| {
            panic!("contrast.css overrides `{name}`; add it to style::tokens::CONTRAST")
        });
        let want = if v == &format!("var(--{})", o.from.name()) {
            100
        } else if let Some(rest) = v.strip_prefix("color-mix(in srgb, var(--") {
            // color-mix(in srgb, var(--text) 55%, transparent)
            let (from, pct) = rest.split_once(") ").unwrap();
            assert_eq!(from, o.from.name(), "{name}");
            pct.split('%').next().unwrap().parse::<u8>().unwrap()
        } else {
            panic!("contrast.css has a form of `{name}` this test does not know: {v}");
        };
        assert_eq!(want, o.pct, "{name}");
        seen.push(tok);
    }
    // glass panels turn opaque.
    assert!(css.contains(".glass-panel { background: var(--surface); }"));
    for o in &CONTRAST {
        if o.token != Token::Glass {
            assert!(
                seen.contains(&o.token),
                "style::tokens::CONTRAST has `{}` but contrast.css does not",
                o.token.name()
            );
        }
    }
    assert!(
        CONTRAST
            .iter()
            .any(|o| o.token == Token::Glass && o.from == Token::Surface && o.pct == 100)
    );
}

#[test]
fn contrast_remap_follows_the_table() {
    assert_eq!(
        contrast_remap(ColorRef::Token(Token::Dim)),
        ColorRef::Token(Token::Text)
    );
    assert_eq!(
        contrast_remap(ColorRef::Token(Token::Line)),
        ColorRef::Alpha(Token::Text, 55)
    );
    assert_eq!(
        contrast_remap(ColorRef::Alpha(Token::Line, 50)),
        ColorRef::Alpha(Token::Text, 27)
    );
    assert_eq!(
        contrast_remap(ColorRef::Token(Token::Glass)),
        ColorRef::Token(Token::Surface)
    );
    assert_eq!(
        contrast_remap(ColorRef::Token(Token::Card)),
        ColorRef::Token(Token::Card)
    );
    assert_eq!(contrast_remap(ColorRef::Transparent), ColorRef::Transparent);
    assert_eq!(contrast_remap(ColorRef::Shade(20)), ColorRef::Shade(20));
}

// ---- WCAG contrast of the built-in themes, with and without @contrast

fn parse_color(s: &str) -> (f64, f64, f64, f64) {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let v = u32::from_str_radix(h, 16).unwrap();
        let (r, g, b) = if h.len() == 3 {
            (((v >> 8) & 15) * 17, ((v >> 4) & 15) * 17, (v & 15) * 17)
        } else {
            ((v >> 16) & 255, (v >> 8) & 255, v & 255)
        };
        return (f64::from(r), f64::from(g), f64::from(b), 1.0);
    }
    let inner = s.split('(').nth(1).unwrap().trim_end_matches(')');
    let p: Vec<f64> = inner
        .split(',')
        .map(|x| x.trim().parse().unwrap())
        .collect();
    (p[0], p[1], p[2], p.get(3).copied().unwrap_or(1.0))
}

fn over(fg: (f64, f64, f64, f64), bg: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let a = fg.3;
    (
        fg.0 * a + bg.0 * (1.0 - a),
        fg.1 * a + bg.1 * (1.0 - a),
        fg.2 * a + bg.2 * (1.0 - a),
        1.0,
    )
}

fn lum(c: (f64, f64, f64, f64)) -> f64 {
    let f = |v: f64| {
        let v = v / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
}

fn ratio(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> f64 {
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[test]
fn contrast_rules_hold_with_and_without_the_contrast_remap() {
    let white = (255.0, 255.0, 255.0, 1.0);
    for theme in ["wol-light", "wol-dark"] {
        let t: serde_json::Value = serde_json::from_str(&read(&format!(
            "crates/core/src/theme/builtin/{theme}.json"
        )))
        .unwrap();
        let tok = |name: &str| parse_color(t["tokens"][name].as_str().unwrap());
        let surface = over(tok("surface"), white);
        let flat = |name: &str| over(tok(name), surface);
        for (fg, bg, min) in CONTRAST_RULES {
            let r = ratio(flat(fg), flat(bg));
            assert!(r >= min, "{theme}: {fg} on {bg} is {r:.2}, needs {min}");
            // Under @contrast the remapped foreground must be at least as legible.
            let remap = |name: &str| {
                let t0 = Token::parse(name).unwrap();
                match contrast_remap(ColorRef::Token(t0)) {
                    ColorRef::Token(t) => flat(t.name()),
                    ColorRef::Alpha(t, a) => {
                        let mut c = tok(t.name());
                        c.3 *= f64::from(a) / 100.0;
                        over(c, surface)
                    }
                    other => panic!("{other:?}"),
                }
            };
            let rr = ratio(remap(fg), remap(bg));
            assert!(
                rr + 0.01 >= r.min(min),
                "{theme}: under @contrast {fg} on {bg} is {rr:.2}, was {r:.2}"
            );
        }
    }
}
