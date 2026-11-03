// SPDX-License-Identifier: BSD-2-Clause
//! Text as the layout and paint stages see it: the shared font set, the mapping from a computed
//! text style to the shaper's style, and the per-window cache of laid-out paragraphs keyed by
//! (text, width constraint, style) (plan section 2.4). Layout measures through the cache and
//! paint draws the same entries, so a paragraph is shaped once per text, width and style.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::style::value::{
    FEATURE_TNUM, FontFamily, FontStyle, TextAlign, TextStyle as Computed, WhiteSpace,
};
use crate::text::fontdb::{FontDb, FontId};
use crate::text::layout::{Align, LayoutOptions, TextLayout, TextStyle};
use crate::text::shape::FontFeature;
use crate::text::system::{Family, TextSystem};

/// The process's font set and caches, shared by every engine and window (one atlas budget).
pub type SharedText = Arc<Mutex<TextSystem>>;

static FONTS: OnceLock<SharedText> = OnceLock::new();
static ROOTS: OnceLock<(Vec<PathBuf>, PathBuf)> = OnceLock::new();

/// Use the fonts under `roots` (their index cached in `index`) as the process's font set. Only
/// remembered here: the index is read when the first window needs text. The first call wins;
/// `wolkit_engine_create` makes it with the person's font folders.
pub fn install(roots: &[PathBuf], index: &Path) {
    let _ = ROOTS.set((roots.to_vec(), index.to_path_buf()));
}

/// The process's font set. Without [`install`] it is empty: every glyph is the missing-glyph box,
/// which keeps engines made in-process (tests, the conformance suite) the same on every machine.
pub fn shared() -> SharedText {
    FONTS
        .get_or_init(|| {
            let sys = match ROOTS.get() {
                Some((roots, index)) => TextSystem::with_system_fonts(roots, index),
                None => TextSystem::from_db(FontDb::new()),
            };
            Arc::new(Mutex::new(sys))
        })
        .clone()
}

/// Lock the shared text system. A panic while it was held leaves only caches behind, so a
/// poisoned lock is taken over rather than propagated.
pub fn lock(t: &SharedText) -> MutexGuard<'_, TextSystem> {
    t.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The shaper's style for a computed text style.
pub fn shaper_style(sys: &TextSystem, t: &Computed) -> TextStyle {
    let family = match t.family {
        FontFamily::Ui => Family::Ui,
        FontFamily::Mono => Family::Mono,
    };
    let font = sys
        .font(family, t.weight, t.style == FontStyle::Italic)
        .unwrap_or(FontId(0));
    let mut s = TextStyle::new(font, t.size);
    if t.line_height > 0.0 {
        s.line_height = t.size * t.line_height;
    }
    if t.feature & FEATURE_TNUM != 0 {
        s.features.push(FontFeature::new(b"tnum", 1));
    }
    s
}

/// Line breaking and alignment for a computed text style at a width constraint.
pub fn options(t: &Computed, max_w: Option<f32>) -> LayoutOptions {
    let max_width = match (t.white_space, max_w) {
        (WhiteSpace::NoWrap, _) | (_, None) => f32::INFINITY,
        (_, Some(w)) => w.max(0.0),
    };
    LayoutOptions {
        max_width,
        align: match t.align {
            TextAlign::Start | TextAlign::Justify => Align::Start,
            TextAlign::Center => Align::Center,
            TextAlign::End => Align::End,
        },
        max_lines: usize::from(t.max_lines),
        ..LayoutOptions::default()
    }
}

/// What of a computed style changes how text lays out (colour and decoration do not).
#[derive(Clone, Copy, Debug, PartialEq)]
struct StyleKey {
    family: FontFamily,
    size: f32,
    weight: u16,
    style: FontStyle,
    line_height: f32,
    align: TextAlign,
    white_space: WhiteSpace,
    max_lines: u16,
    feature: u8,
}

impl StyleKey {
    fn of(t: &Computed) -> StyleKey {
        StyleKey {
            family: t.family,
            size: t.size,
            weight: t.weight,
            style: t.style,
            line_height: t.line_height,
            align: t.align,
            white_space: t.white_space,
            max_lines: t.max_lines,
            feature: t.feature,
        }
    }

    fn hash_into(&self, h: &mut impl Hasher) {
        self.family.hash(h);
        self.size.to_bits().hash(h);
        self.weight.hash(h);
        self.style.hash(h);
        self.line_height.to_bits().hash(h);
        self.align.hash(h);
        self.white_space.hash(h);
        self.max_lines.hash(h);
        self.feature.hash(h);
    }
}

struct Entry {
    text: Box<str>,
    style: StyleKey,
    width: u32,
    layout: Arc<TextLayout>,
}

/// Paragraphs a window keeps, most `CAP` of them; past that the cache starts over (a frame's
/// working set is far smaller, and starting over is bounded and allocation-free to decide).
#[derive(Default)]
pub struct TextCache {
    map: HashMap<u64, Entry>,
    pub hits: u64,
    pub misses: u64,
}

impl TextCache {
    pub const CAP: usize = 4096;

    pub fn new() -> TextCache {
        TextCache::default()
    }

    /// `text` in `style` laid out at most `max_w` wide (`None`: one line per paragraph).
    pub fn layout(
        &mut self,
        sys: &mut TextSystem,
        text: &str,
        style: &Computed,
        max_w: Option<f32>,
    ) -> Arc<TextLayout> {
        let opts = options(style, max_w);
        let key = StyleKey::of(style);
        let width = opts.max_width.to_bits();
        let mut h = std::hash::DefaultHasher::new();
        text.hash(&mut h);
        key.hash_into(&mut h);
        width.hash(&mut h);
        let k = h.finish();
        if let Some(e) = self.map.get(&k)
            && e.width == width
            && e.style == key
            && *e.text == *text
        {
            self.hits += 1;
            return e.layout.clone();
        }
        self.misses += 1;
        let layout = Arc::new(sys.layout(text, &shaper_style(sys, style), &opts));
        if self.map.len() >= Self::CAP {
            self.map.clear();
        }
        self.map.insert(
            k,
            Entry {
                text: text.into(),
                style: key,
                width,
                layout: layout.clone(),
            },
        );
        layout
    }

    /// The width and height of `text` (rounded up to whole pixels, so painting at the measured
    /// width never wraps a line the measure kept).
    pub fn measure(
        &mut self,
        sys: &mut TextSystem,
        text: &str,
        style: &Computed,
        max_w: Option<f32>,
    ) -> (f32, f32) {
        let l = self.layout(sys, text, style, max_w);
        (l.width.ceil(), l.height.ceil())
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Drop every paragraph (Memory Saver trim, a font change).
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_measure_is_a_cache_hit() {
        let mut sys = TextSystem::from_db(FontDb::new());
        let mut c = TextCache::new();
        let st = Computed::default();
        let a = c.measure(&mut sys, "hello world", &st, Some(200.0));
        let b = c.measure(&mut sys, "hello world", &st, Some(200.0));
        assert_eq!(a, b);
        assert_eq!((c.hits, c.misses, c.len()), (1, 1, 1));
        // Another width, text or style is another entry.
        c.measure(&mut sys, "hello world", &st, Some(20.0));
        c.measure(&mut sys, "hello", &st, Some(200.0));
        let big = Computed {
            size: 26.0,
            ..Computed::default()
        };
        c.measure(&mut sys, "hello world", &big, Some(200.0));
        assert_eq!((c.misses, c.len()), (4, 4));
    }

    #[test]
    fn no_wrap_ignores_the_width_and_colour_shares_entries() {
        let mut sys = TextSystem::from_db(FontDb::new());
        let mut c = TextCache::new();
        let nowrap = Computed {
            white_space: WhiteSpace::NoWrap,
            ..Computed::default()
        };
        assert_eq!(options(&nowrap, Some(10.0)).max_width, f32::INFINITY);
        c.measure(&mut sys, "a b c", &nowrap, Some(10.0));
        c.measure(&mut sys, "a b c", &nowrap, Some(99.0));
        let tinted = Computed {
            color: crate::style::value::ColorRef::Transparent,
            white_space: WhiteSpace::NoWrap,
            ..Computed::default()
        };
        c.measure(&mut sys, "a b c", &tinted, None);
        assert_eq!((c.hits, c.misses), (2, 1));
    }

    #[test]
    fn the_cache_is_bounded() {
        let mut sys = TextSystem::from_db(FontDb::new());
        let mut c = TextCache::new();
        let st = Computed::default();
        for i in 0..TextCache::CAP + 10 {
            c.measure(&mut sys, &i.to_string(), &st, None);
        }
        assert!(c.len() <= TextCache::CAP);
    }

    #[test]
    fn tabular_figures_and_line_height_reach_the_shaper() {
        let sys = TextSystem::from_db(FontDb::new());
        let t = Computed {
            feature: FEATURE_TNUM,
            line_height: 1.5,
            size: 20.0,
            ..Computed::default()
        };
        let s = shaper_style(&sys, &t);
        assert_eq!(s.line_height, 30.0);
        assert_eq!(s.features, vec![FontFeature::new(b"tnum", 1)]);
    }
}
