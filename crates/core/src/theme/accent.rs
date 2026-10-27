//! Accent colours as theme variants (theme contract, IaraOS ADR 0016): `wol-<light|dark>-<accent>` extends the
//! WolOS base theme and overrides `accent` + `accentText`. `wol-auto-<accent>` follows the system appearance.
//! `accentText` is white unless that misses the contrast rule; then the darkest text that passes.
use super::ThemeSpec;
use shared::theme::{ACCENT_CONTRAST, Color, Variant};
use std::collections::BTreeMap;

/// (id, name, light hex, dark hex). First = brand default (the plain base themes).
pub const ACCENTS: [(&str, &str, &str, &str); 8] = [
    ("violet", "Violet", "#7c5cff", "#8b6cff"),
    ("blue", "Blue", "#007aff", "#0a84ff"),
    ("pink", "Pink", "#e3307a", "#ff4f8b"),
    ("red", "Red", "#e0352b", "#ff453a"),
    ("orange", "Orange", "#f57c00", "#ff9f0a"),
    ("yellow", "Yellow", "#f2b705", "#ffd60a"),
    ("green", "Green", "#28a745", "#30d158"),
    ("graphite", "Graphite", "#6e6e73", "#98989d"),
];

pub const DEFAULT: &str = "violet";
const AUTO: &str = "wol-auto";

fn base(v: Variant) -> &'static str {
    v.default_theme()
}

/// Theme id for an appearance (None = Auto) and accent.
pub fn theme_id(variant: Option<Variant>, accent: &str) -> Option<String> {
    let known = ACCENTS.iter().any(|a| a.0 == accent);
    match (variant, known) {
        (_, false) => None,
        (None, _) if accent == DEFAULT => None, // no file: follow appearance
        (None, _) => Some(format!("{AUTO}-{accent}")),
        (Some(v), _) if accent == DEFAULT => Some(base(v).into()),
        (Some(v), _) => Some(format!("{}-{accent}", base(v))),
    }
}

/// Auto appearance: dark from 19:00 to 06:59 local time.
pub const DAY_STARTS: u8 = 7;
pub const NIGHT_STARTS: u8 = 19;

pub fn auto_is_dark(hour: u8) -> bool {
    !(DAY_STARTS..NIGHT_STARTS).contains(&hour)
}

/// Seconds until the next light/dark switch.
pub fn secs_to_next_switch(hour: u8, minute: u8, second: u8) -> u32 {
    let now = hour as u32 * 3600 + minute as u32 * 60 + second as u32;
    let next = [
        DAY_STARTS as u32 * 3600,
        NIGHT_STARTS as u32 * 3600,
        (24 + DAY_STARTS as u32) * 3600,
    ]
    .into_iter()
    .find(|t| *t > now)
    .unwrap_or(0);
    next - now
}

/// No theme file or a `wol-auto-*` id: appearance follows the time of day.
pub fn is_auto(saved: Option<&str>) -> bool {
    saved.is_none_or(|s| s.starts_with(&format!("{AUTO}-")))
}

/// Saved id -> (appearance, accent). None appearance = Auto.
pub fn split(saved: Option<&str>) -> (Option<Variant>, &'static str) {
    let find = |a: &str| ACCENTS.iter().find(|x| x.0 == a).map(|x| x.0);
    let Some(id) = saved else {
        return (None, DEFAULT);
    };
    if let Some(a) = id.strip_prefix(&format!("{AUTO}-")).and_then(find) {
        return (None, a);
    }
    for v in [Variant::Light, Variant::Dark] {
        if id == base(v) {
            return (Some(v), DEFAULT);
        }
        if let Some(a) = id.strip_prefix(&format!("{}-", base(v))).and_then(find) {
            return (Some(v), a);
        }
    }
    (None, DEFAULT)
}

/// `wol-auto-<accent>` -> the variant id for the current appearance; other ids unchanged.
pub fn effective(saved: Option<&str>, dark: bool) -> Option<String> {
    let id = saved?;
    if id.starts_with(&format!("{AUTO}-")) {
        let (_, a) = split(Some(id));
        let v = if dark { Variant::Dark } else { Variant::Light };
        return theme_id(Some(v), a);
    }
    Some(id.into())
}

/// Accent hex for a variant.
pub fn color(variant: Variant, accent: &str) -> Option<&'static str> {
    ACCENTS
        .iter()
        .find(|a| a.0 == accent)
        .map(|a| if variant == Variant::Light { a.2 } else { a.3 })
}

/// White when it passes the accent contrast rule, else near-black.
pub fn text_on(accent_hex: &str) -> &'static str {
    let a = Color::parse(accent_hex).unwrap_or(Color::WHITE);
    if Color::contrast(Color::WHITE, a) >= ACCENT_CONTRAST {
        "#ffffff"
    } else {
        "#1d1d1f"
    }
}

/// Every non-default accent variant, for the catalog.
pub fn variants() -> Vec<ThemeSpec> {
    let mut v = Vec::new();
    for variant in [Variant::Light, Variant::Dark] {
        for (id, name, ..) in ACCENTS.iter().skip(1) {
            let hex = color(variant, id).unwrap_or_default();
            let tokens = BTreeMap::from([
                ("accent".to_string(), hex.to_string()),
                ("accentText".to_string(), text_on(hex).to_string()),
            ]);
            v.push(ThemeSpec {
                id: format!("{}-{id}", base(variant)),
                name: format!(
                    "WolOS {} {name}",
                    if variant == Variant::Light {
                        "Light"
                    } else {
                        "Dark"
                    }
                ),
                variant,
                extends: Some(base(variant).into()),
                tokens,
                wallpaper: None,
                icon_theme: None,
                dir: Default::default(),
            });
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_follows_time_of_day() {
        for (h, dark) in [
            (0, true),
            (6, true),
            (7, false),
            (12, false),
            (18, false),
            (19, true),
            (23, true),
        ] {
            assert_eq!(auto_is_dark(h), dark, "{h}");
        }
        assert_eq!(secs_to_next_switch(6, 59, 30), 30);
        assert_eq!(secs_to_next_switch(12, 0, 0), 7 * 3600);
        assert_eq!(secs_to_next_switch(23, 0, 0), 8 * 3600);
        assert!(is_auto(None));
        assert!(is_auto(Some(&format!("{AUTO}-pink"))));
        assert!(!is_auto(Some("wol-dark")));
        assert!(
            !is_auto(Some("solarized-dark")),
            "third-party themes are explicit"
        );
    }
    use crate::theme::ThemeCatalog;

    #[test]
    fn every_accent_variant_passes_the_contract() {
        let mut cat = ThemeCatalog::builtin();
        for s in variants() {
            cat = cat.with(s);
        }
        assert_eq!(variants().len(), 14);
        for v in [Variant::Light, Variant::Dark] {
            for (a, ..) in ACCENTS {
                let id = theme_id(Some(v), a).unwrap();
                let t = cat.resolve(&id).unwrap();
                assert_eq!(t.check(), Vec::<String>::new(), "{id}");
                assert_eq!(t.tokens["accent"], color(v, a).unwrap(), "{id}");
            }
        }
        assert_eq!(text_on("#ffd60a"), "#1d1d1f");
        assert_eq!(text_on("#007aff"), "#ffffff");
    }

    #[test]
    fn ids_round_trip() {
        for (v, a, id) in [
            (Some(Variant::Light), "blue", Some("wol-light-blue")),
            (Some(Variant::Dark), "violet", Some("wol-dark")),
            (None, "green", Some("wol-auto-green")),
            (None, "violet", None),
        ] {
            assert_eq!(theme_id(v, a).as_deref(), id);
            assert_eq!(split(id), (v, a));
        }
        assert_eq!(theme_id(None, "plaid"), None);
        assert_eq!(split(Some("nord")), (None, DEFAULT));
        assert_eq!(
            effective(Some("wol-auto-red"), true).as_deref(),
            Some("wol-dark-red")
        );
        assert_eq!(effective(Some("nord"), true).as_deref(), Some("nord"));
        assert_eq!(effective(None, true), None);
    }
}
