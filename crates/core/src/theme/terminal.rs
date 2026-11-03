// SPDX-License-Identifier: BSD-2-Clause
//! One terminal palette per theme (ADR 0031 themes v2): the theme's own `terminal` list (ANSI 0-15)
//! when it has one, else derived from the tokens. foot, ghostty, zellij and the shell colour files
//! (`console`) all read this, so every terminal shows the same 16 colours.
use super::ResolvedTheme;
use super::apps::{mix, token_color};
use shared::theme::{Color, TERMINAL_COLORS, Variant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub cursor: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    /// ANSI 0-7 then the bright 8-15.
    pub ansi: [Color; TERMINAL_COLORS],
}

impl Palette {
    pub fn regular(&self) -> &[Color] {
        &self.ansi[..8]
    }
    pub fn bright(&self) -> &[Color] {
        &self.ansi[8..]
    }
}

pub fn palette(t: &ResolvedTheme) -> Palette {
    let c = |k| token_color(t, k);
    let bg = c("surface");
    let dark = t.variant == Variant::Dark;
    let ansi = t
        .terminal
        .as_ref()
        .and_then(|list| {
            let v: Vec<Color> = list
                .iter()
                .map(|s| Color::parse(s).map(|x| x.over(bg)))
                .collect::<Option<_>>()?;
            <[Color; TERMINAL_COLORS]>::try_from(v).ok()
        })
        .unwrap_or_else(|| derived(t, dark));
    Palette {
        background: bg,
        foreground: c("text"),
        cursor: c("accent"),
        selection_bg: c("accent"),
        selection_fg: c("accentText"),
        ansi,
    }
}

/// Token colours on the ANSI slots that carry meaning (red = danger, green = success, yellow =
/// warning, magenta = accent) and a fixed set tuned per variant for the rest; bright = a quarter
/// of the way to white (dark) or black (light).
fn derived(t: &ResolvedTheme, dark: bool) -> [Color; TERMINAL_COLORS] {
    let c = |k| token_color(t, k);
    let (black, white) = if dark {
        (Color::rgb(58, 58, 64), Color::rgb(229, 229, 234))
    } else {
        (Color::rgb(29, 29, 31), Color::rgb(199, 199, 204))
    };
    let blue = Color::rgb(10, 132, 255);
    let cyan = if dark {
        Color::rgb(100, 210, 255)
    } else {
        Color::rgb(0, 122, 168)
    };
    let regular = [
        black,
        c("danger"),
        c("success"),
        c("warning"),
        blue,
        c("accent"),
        cyan,
        white,
    ];
    let lift = if dark {
        Color::WHITE
    } else {
        Color::rgb(0, 0, 0)
    };
    let mut out = [Color::WHITE; TERMINAL_COLORS];
    for (i, col) in regular.iter().enumerate() {
        out[i] = *col;
        out[i + 8] = mix(*col, lift, 0.25);
    }
    out
}

/// `#rrggbb` without the `#`.
pub fn bare(c: Color) -> String {
    c.to_hex()[1..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ThemeCatalog, ThemeSpec};
    use std::path::Path;

    #[test]
    fn a_theme_may_bring_its_own_sixteen_colours() {
        let list: Vec<String> = (0..16).map(|i| format!("#0000{:02x}", i * 10)).collect();
        let json = serde_json::json!({
            "id": "t16", "variant": "dark", "terminal": list, "tokens": {}
        })
        .to_string();
        let spec = ThemeSpec::parse(&json, Path::new("")).unwrap();
        let cat = ThemeCatalog::builtin().with(spec);
        let p = palette(&cat.resolve("t16").unwrap());
        assert_eq!(p.ansi[15], Color::rgb(0, 0, 150));
        // A theme that extends it inherits the list.
        let child = ThemeSpec::parse(
            r#"{"id":"child","variant":"dark","extends":"t16","tokens":{}}"#,
            Path::new(""),
        )
        .unwrap();
        let t = cat.with(child).resolve("child").unwrap();
        assert_eq!(palette(&t).ansi[3], Color::rgb(0, 0, 30));
    }

    #[test]
    fn a_wrong_terminal_list_is_refused() {
        for bad in [
            r##"{"id":"x","variant":"dark","terminal":["#000"]}"##,
            r##"{"id":"x","variant":"dark","terminal":"#000"}"##,
        ] {
            assert!(ThemeSpec::parse(bad, Path::new("")).is_err(), "{bad}");
        }
        let mut sixteen = vec!["#000000"; 15];
        sixteen.push("nope");
        let j = serde_json::json!({"id":"x","variant":"dark","terminal":sixteen}).to_string();
        assert!(ThemeSpec::parse(&j, Path::new("")).is_err());
    }

    #[test]
    fn derived_colours_carry_the_tokens() {
        for s in ThemeCatalog::builtin().list() {
            let t = ThemeCatalog::builtin().resolve(&s.id).unwrap();
            let p = palette(&t);
            assert_eq!(p.ansi[1], token_color(&t, "danger"), "{}", s.id);
            assert_eq!(p.ansi[5], token_color(&t, "accent"), "{}", s.id);
            assert_eq!(p.background, token_color(&t, "surface"), "{}", s.id);
        }
    }
}
