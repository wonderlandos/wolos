//! The Kit AppWindow scaffold for third-party toolkits: `adwaita.css` (libadwaita / GTK 4) maps the
//! WolOS toolbar, sidebar, cards, controls, menus and sheets onto their widgets. The sheets name
//! colours as theme tokens (`var(--wol-*)`) and metrics as `@NAME@` (`shared::theme::scaffold`), so
//! one text serves every theme and the Kit's numbers have one source.

use super::ResolvedTheme;
use super::apps::{mix, token_color};
use shared::theme::{Color, TOKENS, css_var, scaffold as m};

pub const ADWAITA: &str = include_str!("adwaita.css");
pub const ADW_GTK3: &str = include_str!("adw-gtk3.css");
/// px a libadwaita header bar extends past each side of the window (measured: 402 px in 400).
pub const ADW_BAR_BLEED: i32 = 1;
/// Header bar box padding above and below: a 32 px row for titles, 24 px controls inside it.
const BAR_PADDING_V: i32 = 10;

/// `@NAME@` placeholders and their values (px).
pub fn metrics() -> Vec<(&'static str, i32)> {
    vec![
        ("TOOLBAR_HEIGHT", m::TOOLBAR_HEIGHT),
        // libadwaita lays a header bar 1 px past each window edge (it hides the bar's side
        // hairlines): the box pads that much more for the Kit's 16 px.
        ("BAR_PADDING", m::TOOLBAR_PADDING + ADW_BAR_BLEED),
        ("BAR_PADDING_V", BAR_PADDING_V),
        (
            "BAR_CONTROL_MARGIN",
            (m::TOOLBAR_HEIGHT - 2 * BAR_PADDING_V - m::CONTROL_HEIGHT) / 2,
        ),
        ("SIDEBAR_WIDTH", m::SIDEBAR_WIDTH),
        ("SOURCE_ROW", m::SOURCE_ROW),
        ("LIST_ROW", m::LIST_ROW),
        ("CONTROL_HEIGHT", m::CONTROL_HEIGHT),
        ("RADIUS_CONTROL", m::RADIUS_CONTROL),
        ("RADIUS_CARD", m::RADIUS_CARD),
        ("WINDOW_RADIUS", m::WINDOW_RADIUS),
        ("BODY", m::BODY),
        ("SECONDARY", m::SECONDARY),
        ("LARGE_TITLE", m::LARGE_TITLE),
        ("LIGHT_SIZE", m::LIGHT_SIZE),
        ("LIGHT_GAP", m::LIGHT_PITCH - m::LIGHT_SIZE),
        // First circle's left edge past the header bar's own padding.
        (
            "LIGHTS_INSET",
            m::LIGHT_FIRST_CENTRE - m::LIGHT_SIZE / 2 - m::TOOLBAR_PADDING,
        ),
    ]
}

/// `css` with every `@NAME@` metric replaced.
pub fn expand(css: &str) -> String {
    metrics().into_iter().fold(css.to_string(), |s, (k, v)| {
        s.replace(&format!("@{k}@"), &v.to_string())
    })
}

/// Token value for the toolkit sheets: opaque (flattened over the surface) except the hairline,
/// which stays translucent so it reads on any fill.
pub fn token_value(t: &ResolvedTheme, tok: &str) -> String {
    if tok == "line" {
        return t.tokens.get(tok).cloned().unwrap_or_default();
    }
    token_color(t, tok).to_hex()
}

/// `--wol-<token>` declarations (GTK 4.16+ custom properties).
pub fn gtk4_vars(t: &ResolvedTheme) -> String {
    TOKENS
        .iter()
        .map(|tok| format!("  --wol-{}: {};\n", &css_var(tok)[2..], token_value(t, tok)))
        .collect()
}

/// libadwaita's own knobs, set from the theme: window corners as Hyprland rounds them, hairlines
/// at the line token's strength, dimmed text as dark as the `dim` token.
pub fn adw_knobs(t: &ResolvedTheme) -> String {
    let line = Color::parse(&t.tokens["line"]).map_or(0.15, |c| if c.a < 1.0 { c.a } else { 0.15 });
    let line_css = &t.tokens["line"];
    let mut s = format!(
        "  --window-radius: {}px;\n  --border-opacity: {}%;\n  --dim-opacity: {}%;\n  --headerbar-shade-color: {line_css};\n  --headerbar-border-color: {line_css};\n  --sidebar-border-color: {line_css};\n  --sidebar-shade-color: {line_css};\n  --card-shade-color: {line_css};\n",
        m::WINDOW_RADIUS,
        (line * 100.0).round(),
        dim_opacity(t),
    );
    for (k, v) in accent_blues(t) {
        s += &format!("  --{k}: {v};\n");
    }
    s
}

/// libadwaita's named blues (`--accent-blue`, `--blue-1`..`--blue-5`, GTK 3 `@blue_1`..) as the
/// theme accent and its tints: apps that name "blue" get the WolOS accent, never GNOME's.
pub fn accent_blues(t: &ResolvedTheme) -> Vec<(&'static str, String)> {
    let a = token_color(t, "accent");
    let black = Color::rgb(0, 0, 0);
    vec![
        ("accent-blue", a.to_hex()),
        ("blue-1", mix(a, Color::WHITE, 0.45).to_hex()),
        ("blue-2", mix(a, Color::WHITE, 0.2).to_hex()),
        ("blue-3", a.to_hex()),
        ("blue-4", mix(a, black, 0.12).to_hex()),
        ("blue-5", mix(a, black, 0.25).to_hex()),
    ]
}

/// Opacity (%) at which `text` over the surface looks like the `dim` token (libadwaita dims text
/// with opacity, the Kit with a colour).
pub fn dim_opacity(t: &ResolvedTheme) -> u32 {
    let (s, x, d) = (
        token_color(t, "surface"),
        token_color(t, "text"),
        token_color(t, "dim"),
    );
    (40..=95)
        .min_by_key(|a| {
            let c = mix(s, x, f64::from(*a) / 100.0);
            [(c.r, d.r), (c.g, d.g), (c.b, d.b)]
                .iter()
                .map(|(p, q)| (i32::from(*p) - i32::from(*q)).abs())
                .sum::<i32>()
        })
        .unwrap_or(55)
}

/// The libadwaita / GTK 4 scaffold sheet (after the `:root` block).
pub fn gtk4(_t: &ResolvedTheme) -> String {
    expand(ADWAITA)
}

/// `@define-color wol_<token>` lines: GTK 3 has no custom properties.
pub fn gtk3_defines(t: &ResolvedTheme) -> String {
    TOKENS
        .iter()
        .map(|tok| {
            let name = css_var(tok)[2..].replace('-', "_");
            format!("@define-color wol_{name} {};\n", token_value(t, tok))
        })
        .collect()
}

/// The GTK 3 scaffold sheet (on top of adw-gtk3, after the colour definitions).
pub fn gtk3(_t: &ResolvedTheme) -> String {
    expand(ADW_GTK3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeCatalog;
    use crate::theme::apps::{ADW_ACCENTS, GTK3_THEME_CSS, render};
    use std::path::Path;

    fn repo(rel: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    fn themes() -> Vec<ResolvedTheme> {
        let cat = ThemeCatalog::builtin();
        cat.list()
            .iter()
            .map(|s| cat.resolve(&s.id).unwrap())
            .collect()
    }

    /// The Kit (kit.css, widgets.rs) and Hyprland draw the scaffold with the same numbers the
    /// toolkit sheets get: one source, held both ways.
    #[test]
    fn metrics_are_the_kits() {
        let kit = repo("crates/shell/src/kit/kit.css");
        for want in [
            format!(
                ".toolbar {{ min-height: {}px; padding: 0 {}px;",
                m::TOOLBAR_HEIGHT,
                m::TOOLBAR_PADDING
            ),
            format!(
                "list.sidebar row {{ background: none; padding: 0 8px; min-height: {}px; border-radius: {}px;",
                m::SOURCE_ROW,
                m::RADIUS_CONTROL
            ),
            format!(
                "button.light {{ min-width: {s}px; min-height: {s}px;",
                s = m::LIGHT_SIZE
            ),
            format!(".list-row {{ min-height: {}px;", m::LIST_ROW),
            format!(
                "button.btn {{ border: none; box-shadow: none; min-height: {}px; padding: 0 12px; border-radius: {}px; }}",
                m::CONTROL_HEIGHT,
                m::RADIUS_CONTROL
            ),
            format!("--glass-radius-s: {}px;", m::RADIUS_CARD),
            format!("font-size: {}px; }}", m::BODY),
            format!(
                "label.secondary, .secondary label {{ font-size: {}px;",
                m::SECONDARY
            ),
            format!("label.large-title {{ font-size: {}px;", m::LARGE_TITLE),
        ] {
            assert!(kit.contains(&want), "kit.css: {want}");
        }
        assert!(repo("crates/shell/src/kit/widgets.rs").contains(&format!(
            "pub const SIDEBAR_WIDTH: i32 = {};",
            m::SIDEBAR_WIDTH
        )));
        let hypr = repo("desktop/overlay/usr/share/wolos/hypr/hyprland.conf");
        for want in [
            format!("rounding = {}\n", m::WINDOW_RADIUS),
            format!(
                "bar_padding = {}\n",
                m::LIGHT_FIRST_CENTRE - m::LIGHT_SIZE / 2
            ),
            format!("bar_button_padding = {}\n", m::LIGHT_PITCH - m::LIGHT_SIZE),
        ] {
            assert!(hypr.contains(&want), "hyprland.conf: {want}");
        }
    }

    /// First `@NAME@` left in `css`, if any.
    fn unexpanded(css: &str) -> Option<String> {
        let parts: Vec<&str> = css.split('@').collect();
        parts[1..parts.len().saturating_sub(1).max(1)]
            .iter()
            .find(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
            .map(|w| w.to_string())
    }

    #[test]
    fn every_metric_and_token_resolves() {
        assert_eq!(
            unexpanded("a @TOOLBAR_HEIGHT@px"),
            Some("TOOLBAR_HEIGHT".into())
        );
        assert_eq!(unexpanded("@define-color wol_line @wol_text;"), None);
        for t in themes() {
            let files = render(&t, Path::new("/x"));
            let file = |rel: &str| files.iter().find(|f| f.rel == rel).unwrap().text.clone();
            let (css, gtk3) = (file("gtk-4.0/gtk.css"), file(GTK3_THEME_CSS));
            assert_eq!(unexpanded(&css), None, "{}: GTK 4 metric", t.id);
            assert_eq!(unexpanded(&gtk3), None, "{}: GTK 3 metric", t.id);
            for used in gtk3.split("@wol_").skip(1) {
                let name: String = used
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                assert!(
                    gtk3.contains(&format!("@define-color wol_{name} ")),
                    "{}: @wol_{name} undefined",
                    t.id
                );
            }
            assert!(gtk3.contains("@define-color wol_accent_text "));
            assert!(gtk3.contains(&format!(
                "headerbar {{ min-height: {}px;",
                m::TOOLBAR_HEIGHT
            )));
            for used in css.split("var(--wol-").skip(1) {
                let name = used.split(')').next().unwrap();
                assert!(
                    css.contains(&format!("  --wol-{name}: ")),
                    "{}: --wol-{name} undeclared",
                    t.id
                );
            }
            assert!(css.contains("--wol-accent-text: "));
        }
    }

    /// Apps that paint their own badges in fixed palette colours get the theme instead: Clocks'
    /// time pill (yellow/orange/blue by daylight) is accent-tinted, Calendar's month badge (a
    /// dark-grey chip) is a Kit secondary label.
    #[test]
    fn app_badges_follow_the_theme() {
        let css = gtk4(&themes()[0]);
        let rule = |sel: &str| {
            css.lines()
                .find(|l| l.starts_with(sel))
                .unwrap_or_else(|| panic!("{sel} not styled"))
                .to_string()
        };
        let pill = rule(".clock-time {");
        assert!(pill.contains("color: var(--accent-color);"), "{pill}");
        assert!(pill.contains("var(--wol-accent)"), "{pill}");
        let badge = rule("monthcell label.first-day");
        assert!(badge.contains("background: none;"), "{badge}");
        assert!(badge.contains("color: var(--wol-dim);"), "{badge}");
        assert!(
            badge.contains(&format!("font-size: {}px;", m::SECONDARY)),
            "{badge}"
        );
    }

    /// libadwaita's accent presets and named blues never reach a stylesheet: the accent is the
    /// theme's, and apps that ask for "blue" (Clocks' night pill, palette users) get it too.
    #[test]
    fn no_stock_adwaita_blue() {
        let stock: Vec<String> = ADW_ACCENTS
            .iter()
            .map(|(_, hex)| hex.to_string())
            .chain(["#99c1f1", "#62a0ea", "#1c71d8", "#1a5fb4"].map(String::from))
            .collect();
        for t in themes() {
            let accent = token_color(&t, "accent").to_hex();
            for f in render(&t, Path::new("/x")) {
                let text = f.text.to_ascii_lowercase();
                for hex in stock.iter().filter(|h| **h != accent) {
                    assert!(!text.contains(hex.as_str()), "{} {}: {hex}", t.id, f.rel);
                }
            }
            let g4 = gtk4_vars(&t) + &adw_knobs(&t);
            for k in ["--accent-blue", "--blue-3"] {
                assert!(g4.contains(&format!("{k}: {accent};")), "{}: {k}", t.id);
            }
        }
    }
}
