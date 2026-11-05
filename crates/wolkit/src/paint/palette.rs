// SPDX-License-Identifier: BSD-2-Clause
//! The live token table: what each of the 13 theme colours is right now. Computed styles keep
//! `ColorRef`s (tokens, never literals, plan section 2.3); the painter resolves them here, so a
//! theme or accent change repaints without restyling.
//!
//! The colours come from a resolved theme (`wol_core::theme`, the same files the shell themes
//! from), with the person's accent and the increase-contrast remap of `style::tokens` on top.

#[cfg(feature = "native")]
use std::sync::OnceLock;

use crate::env::Environment;
use crate::num::sat_u8;
use crate::paint::geom::Color;
use crate::scene::ids::{Ix, to_u8};
use crate::style::tokens::{Token, contrast_remap};
use crate::style::value::ColorRef;
#[cfg(feature = "native")]
use shared::theme::Variant;
#[cfg(feature = "native")]
use wol_core::theme::{ResolvedTheme, ThemeCatalog, accent};

/// One colour per token, in `Token::ALL` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    colors: [Color; 13],
    contrast: bool,
}

/// A theme colour as the painter's `Color` (alpha rounded to a byte; unparsable is clear).
fn parse(css: &str) -> Color {
    shared::theme::Color::parse(css).map_or(Color::TRANSPARENT, |c| {
        // The alpha is 0..=1; the product is clamped to a byte.
        let a = sat_u8((c.a * 255.0).round() as f32);
        Color::rgba(c.r, c.g, c.b, a)
    })
}

#[cfg(feature = "native")]
fn from_theme(t: &ResolvedTheme) -> [Color; 13] {
    let mut out = [Color::TRANSPARENT; 13];
    for (slot, name) in out.iter_mut().zip(Token::NAMES) {
        *slot = t.tokens.get(name).map_or(Color::TRANSPARENT, |v| parse(v));
    }
    out
}

/// The built-in light and dark themes, read once.
#[cfg(feature = "native")]
fn builtin(dark: bool) -> &'static [Color; 13] {
    static BASE: OnceLock<[[Color; 13]; 2]> = OnceLock::new();
    let base = BASE.get_or_init(|| {
        let cat = ThemeCatalog::builtin();
        let get = |v: Variant| {
            cat.resolve(v.default_theme())
                .map_or([Color::TRANSPARENT; 13], |t| from_theme(&t))
        };
        [get(Variant::Light), get(Variant::Dark)]
    });
    &base[usize::from(dark)]
}

impl Palette {
    /// The built-in theme for the environment's appearance, with its accent and contrast.
    pub fn for_env(env: &Environment) -> Palette {
        Palette::new(*builtin(env.dark), env)
    }

    /// A resolved theme (the person's active one) under the environment's accent and contrast.
    #[cfg(feature = "native")]
    pub fn from_theme(theme: &ResolvedTheme, env: &Environment) -> Palette {
        Palette::new(from_theme(theme), env)
    }

    fn new(mut colors: [Color; 13], env: &Environment) -> Palette {
        let [_, r, g, b] = env.accent_rgb.to_be_bytes();
        let accent = Color::rgb(r, g, b);
        colors[Token::Accent.to_u8().ix()] = accent;
        let hex = format!("#{r:02x}{g:02x}{b:02x}");
        colors[Token::AccentText.to_u8().ix()] = parse(text_on(&hex));
        Palette {
            colors,
            contrast: env.contrast,
        }
    }

    /// The colour of a token (before the contrast remap).
    pub fn token(&self, t: Token) -> Color {
        self.colors[t.to_u8().ix()]
    }

    /// What a computed colour paints as.
    pub fn resolve(&self, c: ColorRef) -> Color {
        let c = if self.contrast { contrast_remap(c) } else { c };
        match c {
            ColorRef::Token(t) => self.token(t),
            ColorRef::Alpha(t, pct) => self.token(t).with_opacity(percent(pct)),
            ColorRef::Shade(pct) => Color::rgba(0, 0, 0, percent(pct)),
            ColorRef::Transparent => Color::TRANSPARENT,
        }
    }

    /// The window's own background.
    pub fn background(&self) -> Color {
        self.resolve(ColorRef::Token(Token::Surface))
    }
}

/// The ink on the accent (`wol_core::theme::accent::text_on`).
#[cfg(feature = "native")]
fn text_on(hex: &str) -> &'static str {
    accent::text_on(hex)
}

/// The portable build (no `wol_core`, e.g. wasm32): the same rule as
/// `wol_core::theme::accent::text_on`, held to it by `portable_tokens_match_the_theme_catalog`.
#[cfg(not(feature = "native"))]
fn text_on(hex: &str) -> &'static str {
    portable::text_on(hex)
}

/// The built-in themes without `wol_core`: the very files `ThemeCatalog::builtin` reads
/// (`crates/core/src/theme/builtin/wol-{light,dark}.json`), parsed once. A native test holds
/// both functions to the catalog, so the web build paints with the shell's tokens.
#[cfg_attr(feature = "native", allow(dead_code))]
pub(crate) mod portable {
    use super::{Color, Token, parse};
    use shared::theme::{ACCENT_CONTRAST, Color as ThemeColor};
    use std::sync::OnceLock;

    // Relative to this file: crates/core/src/theme/builtin, the folder `ThemeCatalog::builtin` embeds.
    const LIGHT: &str = include_str!("../../../core/src/theme/builtin/wol-light.json");
    const DARK: &str = include_str!("../../../core/src/theme/builtin/wol-dark.json");

    fn tokens(json: &str) -> [Color; 13] {
        let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
        let mut out = [Color::TRANSPARENT; 13];
        for (slot, name) in out.iter_mut().zip(Token::NAMES) {
            *slot = v["tokens"][name].as_str().map_or(Color::TRANSPARENT, parse);
        }
        out
    }

    pub fn builtin(dark: bool) -> &'static [Color; 13] {
        static BASE: OnceLock<[[Color; 13]; 2]> = OnceLock::new();
        &BASE.get_or_init(|| [tokens(LIGHT), tokens(DARK)])[usize::from(dark)]
    }

    pub fn text_on(hex: &str) -> &'static str {
        let a = ThemeColor::parse(hex).unwrap_or(ThemeColor::WHITE);
        if ThemeColor::contrast(ThemeColor::WHITE, a) >= ACCENT_CONTRAST {
            "#ffffff"
        } else {
            "#181521"
        }
    }
}

#[cfg(not(feature = "native"))]
fn builtin(dark: bool) -> &'static [Color; 13] {
    portable::builtin(dark)
}

/// 0 to 100 percent as 0 to 255.
fn percent(pct: u8) -> u8 {
    to_u8((u32::from(pct.min(100)) * 255 + 50) / 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_themes_fill_every_token() {
        for dark in [false, true] {
            let env = Environment {
                dark,
                ..Environment::default()
            };
            let p = Palette::for_env(&env);
            for t in Token::ALL {
                assert_ne!(p.token(t).a, 0, "{} dark={dark}", t.name());
            }
            assert_ne!(
                p.background(),
                Palette::for_env(&Environment { dark: !dark, ..env }).background()
            );
        }
    }

    #[test]
    fn the_accent_is_the_persons_and_its_text_stays_readable() {
        let env = Environment {
            accent_rgb: 0x00ff_e000,
            ..Environment::default()
        };
        let p = Palette::for_env(&env);
        assert_eq!(p.token(Token::Accent), Color::rgb(0xff, 0xe0, 0x00));
        // White on yellow fails the accent contrast rule, so the text is the theme ink.
        assert_ne!(p.token(Token::AccentText), Color::rgb(255, 255, 255));
    }

    #[test]
    #[cfg(feature = "native")]
    fn portable_tokens_match_the_theme_catalog() {
        for dark in [false, true] {
            assert_eq!(portable::builtin(dark), builtin(dark), "dark={dark}");
        }
        for hex in [
            "#7452f5", "#ffe000", "#ffffff", "#000000", "#00863c", "#c06400", "#3584e4",
        ] {
            assert_eq!(portable::text_on(hex), accent::text_on(hex), "{hex}");
        }
    }

    #[test]
    fn alpha_shade_and_contrast_resolve() {
        let env = Environment::default();
        let p = Palette::for_env(&env);
        assert_eq!(p.resolve(ColorRef::Shade(100)), Color::rgba(0, 0, 0, 255));
        assert_eq!(p.resolve(ColorRef::Shade(50)).a, 128);
        assert_eq!(p.resolve(ColorRef::Transparent), Color::TRANSPARENT);
        let half = p.resolve(ColorRef::Alpha(Token::Card, 50));
        assert_eq!((half.r, half.a), (p.token(Token::Card).r, 128));
        let hc = Palette::for_env(&Environment {
            contrast: true,
            ..env
        });
        assert_eq!(
            hc.resolve(ColorRef::Token(Token::Dim)),
            hc.token(Token::Text)
        );
    }
}
