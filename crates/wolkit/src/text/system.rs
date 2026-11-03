// SPDX-License-Identifier: BSD-2-Clause
//! The text system an engine owns: fonts with their fallback chain, the shaped-run cache and the
//! glyph cache, behind a few calls (`font`, `layout`, `measure`, `glyphs`).
//!
//! The UI typeface order is the one the Kit already uses (Inter, Cantarell, Noto Sans), then a
//! Latin fallback, then per-script fallbacks (Arabic, Hebrew, CJK, Thai, Devanagari), then a
//! visible missing-glyph box. Installed fonts come from an mtime-keyed index, so start-up reads no
//! font file until text needs it.

use std::path::{Path, PathBuf};

use harfrust::Script;

use crate::text::cache::ShapeCache;
use crate::text::fontdb::{FontDb, FontId, FontIndex};
use crate::text::glyphcache::{GlyphCache, Glyphs};
use crate::text::layout::{LayoutOptions, TextLayout, TextStyle, layout_text};

/// The two families a style can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Ui,
    Mono,
}

/// Families tried for the UI typeface, in order.
pub const UI_FAMILIES: [&str; 6] = [
    "Inter",
    "Cantarell",
    "Noto Sans",
    "Adwaita Sans",
    "DejaVu Sans",
    "Liberation Sans",
];
/// Families tried for the monospace typeface, in order.
pub const MONO_FAMILIES: [&str; 5] = [
    "Adwaita Mono",
    "JetBrains Mono",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
];

/// Fallback families by script, in preference order.
const SCRIPT_FALLBACKS: &[(&str, &[&str])] = &[
    (
        "Arab",
        &[
            "Noto Sans Arabic",
            "Noto Naskh Arabic",
            "Noto Kufi Arabic",
            "DejaVu Sans",
        ],
    ),
    ("Hebr", &["Noto Sans Hebrew", "DejaVu Sans"]),
    (
        "Hani",
        &[
            "Noto Sans CJK SC",
            "Noto Sans CJK JP",
            "Noto Sans SC",
            "Droid Sans Fallback",
            "WenQuanYi Zen Hei",
        ],
    ),
    (
        "Hira",
        &["Noto Sans CJK JP", "Noto Sans JP", "Droid Sans Fallback"],
    ),
    (
        "Kana",
        &["Noto Sans CJK JP", "Noto Sans JP", "Droid Sans Fallback"],
    ),
    (
        "Hang",
        &["Noto Sans CJK KR", "Noto Sans KR", "Droid Sans Fallback"],
    ),
    ("Thai", &["Noto Sans Thai", "Noto Serif Thai"]),
    ("Deva", &["Noto Sans Devanagari"]),
    ("Beng", &["Noto Sans Bengali"]),
    ("Taml", &["Noto Sans Tamil"]),
];

/// Where installed fonts live.
pub fn default_font_roots() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        v.push(PathBuf::from(home).join(".local/share/fonts"));
    }
    v
}

/// Where the font index is cached.
pub fn default_index_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("wolkit/fonts.idx")
}

pub struct TextSystem {
    pub db: FontDb,
    pub shapes: ShapeCache,
    pub glyphs: GlyphCache,
}

impl TextSystem {
    /// A system over an existing font set; the caller has set the fallback chain.
    pub fn from_db(db: FontDb) -> TextSystem {
        TextSystem {
            db,
            shapes: ShapeCache::new(2048),
            glyphs: GlyphCache::default(),
        }
    }

    /// Indexes `roots` (using the cache file when it is still valid) and builds the chain.
    pub fn with_system_fonts(roots: &[PathBuf], index_cache: &Path) -> TextSystem {
        let (index, _) = FontIndex::load_or_scan(roots, index_cache);
        let mut db = FontDb::new();
        index.register(&mut db);
        let mut sys = TextSystem::from_db(db);
        sys.build_chain();
        sys
    }

    /// Sets the general chain and the per-script lists from the families that are installed.
    pub fn build_chain(&mut self) {
        let pick = |db: &FontDb, names: &[&str]| -> Vec<FontId> {
            names
                .iter()
                .filter_map(|n| db.find(n, 400, false))
                .collect()
        };
        let mut chain = pick(&self.db, &UI_FAMILIES);
        for (_, names) in SCRIPT_FALLBACKS {
            for id in pick(&self.db, names) {
                if !chain.contains(&id) {
                    chain.push(id);
                }
            }
        }
        self.db.set_chain(&chain);
        for (tag, names) in SCRIPT_FALLBACKS {
            let Some(script) = script_of_tag(tag) else {
                continue;
            };
            let list = pick(&self.db, names);
            if !list.is_empty() {
                self.db.set_script_fallback(script, &list);
            }
        }
    }

    /// The face for a family, weight and slant, or `None` when nothing suitable is installed.
    pub fn font(&self, family: Family, weight: u16, italic: bool) -> Option<FontId> {
        let names: &[&str] = match family {
            Family::Ui => &UI_FAMILIES,
            Family::Mono => &MONO_FAMILIES,
        };
        self.db.select(names, weight, italic)
    }

    /// Lays text out in `style`.
    pub fn layout(&mut self, text: &str, style: &TextStyle, opts: &LayoutOptions) -> TextLayout {
        layout_text(&self.db, &mut self.shapes, text, style, opts)
    }

    /// The size of `text` laid out at most `max_w` wide (`None`: one line per paragraph).
    pub fn measure(&mut self, text: &str, style: &TextStyle, max_w: Option<f32>) -> (f32, f32) {
        let opts = LayoutOptions {
            max_width: max_w.unwrap_or(f32::INFINITY),
            ..LayoutOptions::default()
        };
        let l = self.layout(text, style, &opts);
        (l.width, l.height)
    }

    /// The glyph source for the rasteriser and the instance builder.
    pub fn glyph_source(&mut self) -> Glyphs<'_> {
        Glyphs {
            db: &self.db,
            cache: &mut self.glyphs,
        }
    }

    /// Drops the glyph images and shaped runs (Memory Saver trim).
    pub fn trim(&mut self) {
        self.glyphs.trim();
        self.shapes.clear();
    }
}

fn script_of_tag(tag: &str) -> Option<Script> {
    tag.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_tags_parse() {
        assert_eq!(script_of_tag("Arab"), Some(Script::ARABIC));
        assert_eq!(script_of_tag("Hani"), Some(Script::HAN));
        assert_eq!(script_of_tag("Hira"), Some(Script::HIRAGANA));
        assert_eq!(script_of_tag("Hang"), Some(Script::HANGUL));
    }

    #[test]
    fn empty_system_still_measures_without_panicking() {
        let mut s = TextSystem::from_db(FontDb::new());
        let st = TextStyle::new(FontId(0), 13.0);
        let (w, h) = s.measure("hello", &st, None);
        assert!(w > 0.0 && h > 0.0, "{w} {h}");
    }
}
