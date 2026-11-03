// SPDX-License-Identifier: BSD-2-Clause
//! One palette, every consumer (ADR 0031 themes v2). [`consumers`] is the full list of files a
//! theme switch writes (GTK 3/4, Qt, foot, Firefox, GtkSourceView, zsh, the compositor, and the
//! console files); `apps::install` writes it. [`from_samples`] makes a theme from a picture's
//! colours (theme-from-wallpaper) that always passes the contrast contract (`ResolvedTheme::check`).
use super::apps::{File, hue_sat, mix, readable};
use super::{ThemeCatalog, ThemeSpec};
use shared::theme::{Color, Variant, validate};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every generated file for `t` (relative to the toolkit dir `dir`).
pub fn consumers(t: &super::ResolvedTheme, dir: &Path) -> Vec<File> {
    let mut v = super::apps::render(t, dir);
    v.extend(super::console::render(t));
    v
}

/// About this many colours are read from a picture (a grid; enough for a palette).
pub const SAMPLES: usize = 32 * 32;

/// Grid samples of an 8-bit RGB(A) image (`stride` bytes per row; `channels` 3 or 4).
/// Transparent pixels are skipped. Empty for a malformed buffer.
pub fn samples(
    pixels: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    channels: usize,
) -> Vec<Color> {
    if width == 0
        || height == 0
        || !(3..=4).contains(&channels)
        || stride < width * channels
        || pixels.len() < stride * (height - 1) + width * channels
    {
        return Vec::new();
    }
    let side = (SAMPLES as f64).sqrt() as usize;
    let mut out = Vec::with_capacity(SAMPLES);
    for gy in 0..side {
        for gx in 0..side {
            let (x, y) = (
                (gx * 2 + 1) * width / (side * 2),
                (gy * 2 + 1) * height / (side * 2),
            );
            let i = y * stride + x * channels;
            if channels == 4 && pixels[i + 3] < 128 {
                continue;
            }
            out.push(Color::rgb(pixels[i], pixels[i + 1], pixels[i + 2]));
        }
    }
    out
}

fn mean(c: &[Color]) -> Color {
    if c.is_empty() {
        return Color::rgb(128, 128, 128);
    }
    let n = c.len() as f64;
    let avg =
        |f: fn(&Color) -> u8| (c.iter().map(|x| f64::from(f(x))).sum::<f64>() / n).round() as u8;
    Color::rgb(avg(|x| x.r), avg(|x| x.g), avg(|x| x.b))
}

fn lightness(c: Color) -> f64 {
    let [r, g, b] = [c.r, c.g, c.b].map(|v| f64::from(v) / 255.0);
    (r.max(g).max(b) + r.min(g).min(b)) / 2.0
}

/// The most colourful sample that is neither near black nor near white.
fn accent_of(samples: &[Color]) -> Option<Color> {
    samples
        .iter()
        .map(|c| {
            let (_, s) = hue_sat(*c);
            let l = lightness(*c);
            (*c, s * (1.0 - ((l - 0.5).abs() * 1.6).min(1.0)), s)
        })
        .filter(|x| x.2 >= 0.25)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|x| x.0)
}

fn rgba(c: Color, a: f64) -> String {
    format!("rgba({},{},{},{a})", c.r, c.g, c.b)
}

/// A theme from a picture's colours: surfaces tinted by the average, the accent the most
/// colourful colour, status colours from the WolOS theme of the variant. `variant` None = dark
/// for a dark picture. Every text colour is pushed until it reads; if anything still failed, the
/// WolOS theme's own tokens are used, so the result always passes `check()`.
pub fn from_samples(
    id: &str,
    name: &str,
    samples: &[Color],
    variant: Option<Variant>,
) -> ThemeSpec {
    let avg = mean(samples);
    let variant = variant.unwrap_or(if avg.luminance() < 0.18 {
        Variant::Dark
    } else {
        Variant::Light
    });
    let base = ThemeCatalog::builtin()
        .resolve(variant.default_theme())
        .map(|t| t.tokens)
        .unwrap_or_default();
    let dark = variant == Variant::Dark;
    let (black, white) = (Color::rgb(0, 0, 0), Color::WHITE);
    let surface = if dark {
        mix(avg, black, 0.82)
    } else {
        mix(avg, white, 0.9)
    };
    let card = if dark {
        mix(surface, white, 0.07)
    } else {
        mix(surface, white, 0.6)
    };
    let ink = if dark {
        Color::rgb(245, 245, 247)
    } else {
        Color::rgb(29, 29, 31)
    };
    let worst = |c: Color| {
        [surface, card]
            .into_iter()
            .min_by(|a, b| Color::contrast(c, *a).total_cmp(&Color::contrast(c, *b)))
            .unwrap_or(surface)
    };
    let text = mix(ink, avg, 0.05);
    let text = readable(text, worst(text), 7.0);
    let dim = mix(text, surface, 0.4);
    let dim = readable(dim, worst(dim), 3.2);
    let accent = accent_of(samples).unwrap_or_else(|| {
        base.get("accent")
            .and_then(|a| Color::parse(a))
            .unwrap_or(Color::rgb(124, 92, 255))
    });
    let on = if Color::contrast(white, accent) >= Color::contrast(black, accent) {
        white
    } else {
        black
    };
    // The accent moves away from its text colour until the label reads (>= 3:1).
    let away = if on == white { black } else { white };
    let accent = (0..=20)
        .map(|i| mix(accent, away, f64::from(i) / 20.0))
        .find(|c| Color::contrast(on, *c) >= 3.2)
        .unwrap_or(away);
    let mut tokens: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in [
        ("surface", surface.to_hex()),
        ("sidebar", surface.to_hex()),
        ("card", card.to_hex()),
        ("line", mix(surface, text, 0.15).to_hex()),
        ("text", text.to_hex()),
        ("dim", dim.to_hex()),
        ("accent", accent.to_hex()),
        ("accentText", on.to_hex()),
        ("glass", rgba(surface, 0.72)),
        ("control", mix(surface, text, 0.1).to_hex()),
    ] {
        tokens.insert(k.into(), v);
    }
    for k in ["success", "warning", "danger"] {
        if let Some(v) = base.get(k) {
            tokens.insert(k.into(), v.clone());
        }
    }
    if !validate(|t| tokens.get(t).cloned()).is_empty() {
        tokens = base;
    }
    ThemeSpec {
        id: id.into(),
        name: name.into(),
        variant,
        extends: None,
        tokens,
        wallpaper: None,
        icon_theme: None,
        terminal: None,
        dir: PathBuf::new(),
    }
}

/// `theme.json` text for a spec (what `ThemeSpec::parse` reads back).
pub fn to_json(s: &ThemeSpec) -> String {
    let mut v = serde_json::json!({
        "id": s.id,
        "name": s.name,
        "variant": s.variant.name(),
        "tokens": s.tokens,
    });
    if let Some(w) = &s.wallpaper {
        v["wallpaper"] = w.clone().into();
    }
    if let Some(e) = &s.extends {
        v["extends"] = e.clone().into();
    }
    if let Some(t) = &s.terminal {
        v["terminal"] = t.clone().into();
    }
    serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(spec: &ThemeSpec) -> Vec<String> {
        ThemeCatalog::builtin()
            .with(spec.clone())
            .resolve(&spec.id)
            .unwrap()
            .check()
    }

    /// Pictures of every kind (dark, light, grey, one loud colour, noise) give a readable theme.
    #[test]
    fn a_theme_from_any_picture_passes_the_contrast_contract() {
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) as u8
        };
        let noise: Vec<Color> = (0..400).map(|_| Color::rgb(rnd(), rnd(), rnd())).collect();
        let sets: Vec<(&str, Vec<Color>)> = vec![
            (
                "night",
                vec![
                    Color::rgb(10, 12, 40),
                    Color::rgb(30, 60, 160),
                    Color::rgb(5, 5, 5),
                ],
            ),
            (
                "beach",
                vec![
                    Color::rgb(240, 220, 180),
                    Color::rgb(80, 180, 230),
                    Color::rgb(255, 255, 255),
                ],
            ),
            ("grey", vec![Color::rgb(128, 128, 128); 10]),
            ("white", vec![Color::WHITE; 10]),
            ("black", vec![Color::rgb(0, 0, 0); 10]),
            ("yellow", vec![Color::rgb(255, 230, 0); 10]),
            ("empty", vec![]),
            ("noise", noise),
        ];
        for (name, s) in &sets {
            for v in [None, Some(Variant::Light), Some(Variant::Dark)] {
                let spec = from_samples("pic", name, s, v);
                assert_eq!(spec.tokens.len(), 13, "{name}");
                assert!(check(&spec).is_empty(), "{name} {v:?}: {:?}", check(&spec));
                let back = ThemeSpec::parse(&to_json(&spec), Path::new("")).unwrap();
                assert_eq!(back.tokens, spec.tokens);
            }
        }
        assert_eq!(
            from_samples("n", "n", &sets[0].1, None).variant,
            Variant::Dark
        );
        assert_eq!(
            from_samples("b", "b", &sets[1].1, None).variant,
            Variant::Light
        );
    }

    #[test]
    fn the_accent_comes_from_the_picture() {
        let s = vec![Color::rgb(20, 20, 24); 30]
            .into_iter()
            .chain([Color::rgb(230, 60, 40)])
            .collect::<Vec<_>>();
        let spec = from_samples("x", "x", &s, Some(Variant::Dark));
        let a = Color::parse(&spec.tokens["accent"]).unwrap();
        assert!(a.r > a.g && a.r > a.b, "{}", spec.tokens["accent"]);
    }

    #[test]
    fn samples_read_a_grid_and_refuse_short_buffers() {
        let (w, h) = (64, 48);
        let mut px = vec![0u8; w * h * 4];
        for (i, p) in px.chunks_mut(4).enumerate() {
            p.copy_from_slice(&[(i % 256) as u8, 100, 200, if i < w * 4 { 0 } else { 255 }]);
        }
        let s = samples(&px, w, h, w * 4, 4);
        assert!(
            !s.is_empty() && s.len() < SAMPLES,
            "the transparent top rows are skipped"
        );
        assert!(s.iter().all(|c| c.g == 100 && c.b == 200));
        assert!(samples(&px[..10], w, h, w * 4, 4).is_empty());
        assert!(samples(&px, w, h, w * 4, 2).is_empty());
        assert!(samples(&px, 0, h, 0, 4).is_empty());
    }

    #[test]
    fn consumers_are_every_toolkit_and_console_file_once() {
        let t = ThemeCatalog::builtin().resolve("wol-dark").unwrap();
        let files = consumers(&t, Path::new("/x"));
        let mut rels: Vec<&str> = files.iter().map(|f| f.rel).collect();
        for want in super::super::console::FILES.into_iter().chain([
            shared::theme::HYPR_THEME_FILE,
            "gtk-4.0/gtk.css",
            "foot/foot.ini",
            "zsh/colors.zsh",
        ]) {
            assert!(rels.contains(&want), "{want}");
        }
        let n = rels.len();
        rels.sort_unstable();
        rels.dedup();
        assert_eq!(rels.len(), n, "no file twice");
    }
}
