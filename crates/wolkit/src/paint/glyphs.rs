// SPDX-License-Identifier: BSD-2-Clause
//! The seam between painting and text: how the rasteriser and the batcher obtain glyph images.

use crate::text::fontdb::FontId;

/// Where a glyph image lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// In atlas page `page` at texel `(x, y)`.
    Atlas { page: u16, x: u16, y: u16 },
    /// A one-off image that is not kept (very large glyphs: drawn as path fills).
    Transient,
}

/// A glyph image borrowed from its source for the duration of one draw.
#[derive(Clone, Copy, Debug)]
pub struct GlyphRef<'a> {
    pub width: u16,
    pub height: u16,
    /// Pixels from the pen x (after the quarter-pixel phase) to the left edge.
    pub left: i16,
    /// Pixels from the baseline up to the top edge.
    pub top: i16,
    /// Bytes per row of `data`.
    pub stride: usize,
    /// Coverage, row-major, starting at the glyph's top-left.
    pub data: &'a [u8],
    pub place: Place,
}

/// Supplies glyph images by (face, glyph, size, quarter-pixel phase).
pub trait GlyphSource {
    fn glyph(&mut self, font: FontId, glyph: u32, size: f32, sub: u8) -> Option<GlyphRef<'_>>;
}

/// A source with no glyphs (scenes without text).
pub struct NoGlyphs;

impl GlyphSource for NoGlyphs {
    fn glyph(&mut self, _: FontId, _: u32, _: f32, _: u8) -> Option<GlyphRef<'_>> {
        None
    }
}
