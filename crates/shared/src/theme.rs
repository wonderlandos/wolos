//! Theme contract (IaraOS ADR 0016): 13 semantic color tokens, fixed scale, validation rules.

/// The only values a theme may change. Order is stable.
pub const TOKENS: [&str; 13] = [
    "surface",
    "sidebar",
    "card",
    "line",
    "text",
    "dim",
    "accent",
    "accentText",
    "glass",
    "control",
    "success",
    "warning",
    "danger",
];

/// Fixed scale (not themable).
pub const SCALE: [(&str, &str); 16] = [
    ("--radius-s", "6px"),
    ("--radius-m", "10px"),
    ("--radius-l", "14px"),
    ("--radius-xl", "20px"),
    ("--space-1", "4px"),
    ("--space-2", "8px"),
    ("--space-3", "12px"),
    ("--space-4", "16px"),
    ("--space-6", "24px"),
    ("--font-size", "13px"),
    ("--font-size-title", "17px"),
    ("--blur", "24px"),
    ("--motion-fast", "150ms"),
    ("--motion-normal", "250ms"),
    ("--motion-slow", "400ms"),
    ("--ease", "cubic-bezier(0.2, 0.8, 0.2, 1)"),
];

/// AppWindow scaffold metrics in px (HIG §1/§2, Kit `AppWindow` in the shell's kit.css): third-party
/// app themes (`wol_core::theme::apps`, GTK 3/4, Firefox) draw the same geometry. Tests hold the Kit
/// stylesheet and the Hyprland config to these values.
pub mod scaffold {
    pub const TOOLBAR_HEIGHT: i32 = 52;
    /// Toolbar side padding.
    pub const TOOLBAR_PADDING: i32 = 16;
    pub const SIDEBAR_WIDTH: i32 = 220;
    /// Sidebar source rows.
    pub const SOURCE_ROW: i32 = 28;
    /// Rows in card lists.
    pub const LIST_ROW: i32 = 40;
    /// Buttons, entries, menu items.
    pub const CONTROL_HEIGHT: i32 = 24;
    pub const RADIUS_CONTROL: i32 = 6;
    pub const RADIUS_CARD: i32 = 10;
    /// Window corners (Hyprland `decoration:rounding`).
    pub const WINDOW_RADIUS: i32 = 12;
    pub const BODY: i32 = 13;
    pub const SECONDARY: i32 = 11;
    pub const LARGE_TITLE: i32 = 22;
    /// Window buttons: 12 px circles, 22 px centre to centre, the first centred 23 px from the
    /// window's left edge (pixel edges), all on the toolbar's vertical centre.
    pub const LIGHT_SIZE: i32 = 12;
    pub const LIGHT_PITCH: i32 = 22;
    pub const LIGHT_FIRST_CENTRE: i32 = 23;
}

pub const TEXT_CONTRAST: f64 = 4.5;
pub const ACCENT_CONTRAST: f64 = 3.0;
/// Secondary (dim) text: WCAG large-text level.
pub const DIM_CONTRAST: f64 = 3.0;

/// Contrast rules: (foreground, background, minimum ratio). A translucent background is
/// measured composited over the surface (itself over white).
pub const CONTRAST_RULES: [(&str, &str, f64); 6] = [
    ("text", "surface", TEXT_CONTRAST),
    ("text", "card", TEXT_CONTRAST),
    ("text", "sidebar", TEXT_CONTRAST),
    ("dim", "surface", DIM_CONTRAST),
    ("dim", "card", DIM_CONTRAST),
    ("accentText", "accent", ACCENT_CONTRAST),
];

pub const SYSTEM_THEMES_DIR: &str = "/usr/share/wolos/themes";
/// Under `$XDG_CONFIG_HOME`.
pub const USER_THEMES_DIR: &str = "wolos/themes";
/// Under `$XDG_CONFIG_HOME`: one theme id.
pub const ACTIVE_FILE: &str = "wolos/theme";
/// Under `$XDG_CONFIG_HOME`: the desktop picture (one line: an image path, or `dynamic`).
pub const WALLPAPER_FILE: &str = "wolos/wallpaper";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Light,
    Dark,
}

impl Variant {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
    /// Built-in default theme for this variant.
    pub fn default_theme(self) -> &'static str {
        match self {
            Self::Light => "wol-light",
            Self::Dark => "wol-dark",
        }
    }
}

pub fn is_token(name: &str) -> bool {
    TOKENS.contains(&name)
}

/// CSS variable for a token: accentText -> --accent-text.
pub fn css_var(token: &str) -> String {
    let mut s = String::from("--");
    for c in token.chars() {
        if c.is_ascii_uppercase() {
            s.push('-');
            s.push(c.to_ascii_lowercase());
        } else {
            s.push(c);
        }
    }
    s
}

/// sRGB color with alpha: #rgb, #rrggbb, #rrggbbaa, rgb(), rgba().
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f64,
}

impl Color {
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 1.0,
    };

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub fn parse(css: &str) -> Option<Self> {
        let s = css.trim();
        if let Some(h) = s.strip_prefix('#') {
            if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            let h: String = if h.len() == 3 {
                h.chars().flat_map(|c| [c, c]).collect()
            } else {
                h.into()
            };
            if h.len() != 6 && h.len() != 8 {
                return None;
            }
            let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
            let a = if h.len() == 8 {
                f64::from(p(6)?) / 255.0
            } else {
                1.0
            };
            return Some(Self {
                r: p(0)?,
                g: p(2)?,
                b: p(4)?,
                a,
            });
        }
        if s.starts_with("rgb") && s.ends_with(')') {
            let inner = &s[s.find('(')? + 1..s.len() - 1];
            let p: Vec<&str> = inner.split(',').map(str::trim).collect();
            if p.len() != 3 && p.len() != 4 {
                return None;
            }
            let a = if p.len() == 4 {
                p[3].parse()
                    .ok()
                    .filter(|a: &f64| (0.0..=1.0).contains(a))?
            } else {
                1.0
            };
            return Some(Self {
                r: p[0].parse().ok()?,
                g: p[1].parse().ok()?,
                b: p[2].parse().ok()?,
                a,
            });
        }
        None
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// This color painted over an opaque background.
    pub fn over(self, bg: Color) -> Color {
        let m =
            |f: u8, b: u8| (f64::from(f) * self.a + f64::from(b) * (1.0 - self.a)).round() as u8;
        Color::rgb(m(self.r, bg.r), m(self.g, bg.g), m(self.b, bg.b))
    }

    pub fn luminance(self) -> f64 {
        let l = |c: u8| {
            let v = f64::from(c) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * l(self.r) + 0.7152 * l(self.g) + 0.0722 * l(self.b)
    }

    /// WCAG 2 contrast ratio (1..21); alpha ignored.
    pub fn contrast(a: Color, b: Color) -> f64 {
        let (x, y) = (a.luminance(), b.luminance());
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }
}

/// Readability check of a fully resolved token set. Empty = valid.
pub fn validate(get: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut errors = Vec::new();
    let mut colors = std::collections::HashMap::new();
    for n in TOKENS {
        match get(n) {
            None => errors.push(format!("{n}: missing")),
            Some(v) => match Color::parse(&v) {
                None => errors.push(format!("{n}: not a color ({v})")),
                Some(c) => {
                    colors.insert(n, c);
                }
            },
        }
    }
    let surface = colors
        .get("surface")
        .map_or(Color::WHITE, |s| s.over(Color::WHITE));
    for (fg, bg, min) in CONTRAST_RULES {
        if let (Some(f), Some(b)) = (colors.get(fg), colors.get(bg)) {
            let b = b.over(surface);
            let ratio = Color::contrast(f.over(b), b);
            if ratio < min {
                errors.push(format!("{fg}/{bg}: contrast {ratio:.2} < {min}"));
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirteen_tokens_css_vars() {
        assert_eq!(TOKENS.len(), 13);
        assert_eq!(css_var("accentText"), "--accent-text");
        assert_eq!(css_var("surface"), "--surface");
    }

    #[test]
    fn color_parse_table() {
        let cases = [
            ("#fff", Some((255, 255, 255, 1.0))),
            ("#112233", Some((0x11, 0x22, 0x33, 1.0))),
            ("#11223300", Some((0x11, 0x22, 0x33, 0.0))),
            ("rgb(1, 2, 3)", Some((1, 2, 3, 1.0))),
            ("rgba(1,2,3,0.5)", Some((1, 2, 3, 0.5))),
            ("#ggg", None),
            ("#12345", None),
            ("rgb(1,2)", None),
            ("rgb(256,0,0)", None),
            ("red", None),
        ];
        for (css, want) in cases {
            assert_eq!(
                Color::parse(css).map(|c| (c.r, c.g, c.b, c.a)),
                want,
                "{css}"
            );
        }
    }

    #[test]
    fn contrast_known_values() {
        let c = Color::contrast(Color::rgb(0, 0, 0), Color::WHITE);
        assert!((c - 21.0).abs() < 1e-9);
        assert!((Color::contrast(Color::WHITE, Color::WHITE) - 1.0).abs() < 1e-9);
    }

    /// Text must read on every background it is drawn on (cards and translucent sidebars
    /// composited over the surface), dim text >= 3:1, accent text >= 3:1.
    #[test]
    fn contrast_rules_cover_every_background() {
        let base = |t: &str| {
            Some(
                match t {
                    "text" | "accentText" => "#ffffff",
                    "dim" => "#a0a0a0",
                    _ => "#000000",
                }
                .to_string(),
            )
        };
        assert!(validate(base).is_empty(), "{:?}", validate(base));
        for (tok, val, rule) in [
            ("card", "#9a9a9a", "text/card"),
            ("sidebar", "rgba(200,200,200,0.9)", "text/sidebar"),
            ("dim", "#444444", "dim/surface"),
            ("card", "#5a5a5a", "dim/card"),
            ("accent", "#d0d0d0", "accentText/accent"),
        ] {
            let e = validate(|t| if t == tok { Some(val.into()) } else { base(t) });
            assert!(e.iter().any(|e| e.starts_with(rule)), "{tok}={val}: {e:?}");
        }
    }

    #[test]
    fn validate_rules() {
        let good = |t: &str| {
            Some(
                match t {
                    "text" | "accentText" => "#ffffff",
                    "dim" => "#a0a0a0",
                    "surface" | "accent" | "card" | "sidebar" => "#000000",
                    _ => "#808080",
                }
                .to_string(),
            )
        };
        assert!(validate(good).is_empty());
        let low = |t: &str| {
            Some(if t == "text" {
                "#111111".into()
            } else {
                good(t).unwrap()
            })
        };
        assert_eq!(validate(low).len(), 3, "surface, card and sidebar");
        let missing = |t: &str| if t == "danger" { None } else { good(t) };
        assert_eq!(validate(missing), vec!["danger: missing"]);
        let bad = |t: &str| {
            if t == "line" {
                Some("blue".into())
            } else {
                good(t)
            }
        };
        assert_eq!(validate(bad), vec!["line: not a color (blue)"]);
    }
}
