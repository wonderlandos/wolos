// SPDX-License-Identifier: BSD-2-Clause
//! Rasteriser + atlas behind the [`GlyphSource`] trait.

use crate::paint::glyphs::{GlyphRef, GlyphSource, Place};
use crate::text::atlas::Atlas;
use crate::text::fontdb::{FontDb, FontId};
use crate::text::glyph::{GlyphBitmap, GlyphKey, GlyphRasterizer};

/// Glyphs larger than this many pixels per em are drawn as one-off path fills, not cached.
pub const LARGE_GLYPH_PX: f32 = 160.0;
/// Most remembered "this glyph has no image" keys (spaces and the like) before the set resets.
const MAX_EMPTY_KEYS: usize = 65_536;

/// Owns the atlas and the rasteriser state; borrow it with a [`FontDb`] through [`Glyphs`].
#[derive(Default)]
pub struct GlyphCache {
    pub atlas: Atlas,
    rast: GlyphRasterizer,
    transient: Option<GlyphBitmap>,
    /// Keys known to have no image (spaces), so they are not re-rendered each frame.
    empty: std::collections::HashSet<GlyphKey>,
}

impl GlyphCache {
    pub fn new(atlas: Atlas) -> GlyphCache {
        GlyphCache {
            atlas,
            ..GlyphCache::default()
        }
    }

    /// Looks a glyph up, rendering and storing it on a miss.
    pub fn get(&mut self, db: &FontDb, key: GlyphKey) -> Option<GlyphRef<'_>> {
        if self.empty.contains(&key) {
            return None;
        }
        if key.size() > LARGE_GLYPH_PX {
            self.transient = self.rast.rasterize(db, key);
            let bm = self.transient.as_ref()?;
            return Some(GlyphRef {
                width: bm.width,
                height: bm.height,
                left: bm.left,
                top: bm.top,
                stride: bm.width as usize,
                data: &bm.data,
                place: Place::Transient,
            });
        }
        let slot = match self.atlas.get(&key) {
            Some(s) => s,
            None => match self.rast.rasterize(db, key) {
                None => {
                    if self.empty.len() >= MAX_EMPTY_KEYS {
                        self.empty.clear();
                    }
                    self.empty.insert(key);
                    return None;
                }
                Some(bm) => match self.atlas.insert(key, &bm) {
                    Some(s) => s,
                    None => {
                        self.transient = Some(bm);
                        let bm = self.transient.as_ref()?;
                        return Some(GlyphRef {
                            width: bm.width,
                            height: bm.height,
                            left: bm.left,
                            top: bm.top,
                            stride: bm.width as usize,
                            data: &bm.data,
                            place: Place::Transient,
                        });
                    }
                },
            },
        };
        let (stride, data) = self.atlas.slot_pixels(&slot)?;
        Some(GlyphRef {
            width: slot.w,
            height: slot.h,
            left: slot.left,
            top: slot.top,
            stride,
            data,
            place: Place::Atlas {
                page: slot.page,
                x: slot.x,
                y: slot.y,
            },
        })
    }

    /// Drops all cached images (Memory Saver trim).
    pub fn trim(&mut self) {
        self.atlas.trim();
        self.empty.clear();
        self.transient = None;
    }
}

/// A [`GlyphSource`] over a font set and a cache.
pub struct Glyphs<'a> {
    pub db: &'a FontDb,
    pub cache: &'a mut GlyphCache,
}

impl GlyphSource for Glyphs<'_> {
    fn glyph(&mut self, font: FontId, glyph: u32, size: f32, sub: u8) -> Option<GlyphRef<'_>> {
        self.cache
            .get(self.db, GlyphKey::new(font, glyph, size, sub))
    }
}
