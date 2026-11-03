// SPDX-License-Identifier: BSD-2-Clause
//! Glyph outline to coverage bitmap, with quarter-pixel horizontal positioning.
//!
//! [`rasterize_glyph`] is a pure function of its inputs (no global state), so the engine can run
//! it on a worker thread and upload the result asynchronously; this module owns no threads.
//! Rendering is unhinted grayscale: no LCD fringing, one coverage byte per pixel.

use crate::num::{round_u32, to_i16, to_i32, trunc_i32};
use crate::scene::ids::{Ix, to_u8, to_u16};
use std::collections::HashMap;

use skrifa::{DrawMemory, GlyphId, PathSink, ScalerFont};

use crate::cpu::coverage::{Accumulator, Flattener};
use crate::text::fontdb::{FontDb, FontId};

/// Number of horizontal sub-pixel positions (quarter pixels).
pub const SUBPIXEL_STEPS: u8 = 4;
/// Largest bitmap side the glyph path will produce; bigger glyphs return `None`.
pub const MAX_GLYPH_SIDE: i32 = 2048;

/// A rendered glyph: coverage bytes plus its placement relative to the pen position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphBitmap {
    pub width: u16,
    pub height: u16,
    /// Pixels from the pen x (after the sub-pixel shift) to the left edge of the bitmap.
    pub left: i16,
    /// Pixels from the baseline up to the top edge of the bitmap.
    pub top: i16,
    pub data: Vec<u8>,
}

/// What identifies one rendered glyph image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font: FontId,
    pub glyph: u32,
    /// Size in 1/64 pixel.
    pub size_q: u32,
    /// Quarter-pixel phase, `0..SUBPIXEL_STEPS`.
    pub sub: u8,
}

impl GlyphKey {
    pub fn new(font: FontId, glyph: u32, size: f32, sub: u8) -> GlyphKey {
        GlyphKey {
            font,
            glyph,
            size_q: round_u32(size.max(0.0) * 64.0),
            sub: sub % SUBPIXEL_STEPS,
        }
    }

    pub fn size(&self) -> f32 {
        self.size_q as f32 / 64.0
    }
}

/// Splits a pen x into whole pixels and a quarter-pixel phase, rounding down (`floor(4x)`).
pub fn split_subpixel(x: f32) -> (i32, u8) {
    let q = (x * f32::from(SUBPIXEL_STEPS)).floor();
    let q = i64::from(trunc_i32(q.clamp(-1.0e9, 1.0e9)));
    (to_i32(q >> 2), to_u8(q & 3))
}

#[derive(Default)]
struct Recorder {
    ops: Vec<Op>,
    min: (f32, f32),
    max: (f32, f32),
    any: bool,
}

#[derive(Clone, Copy)]
enum Op {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

impl Recorder {
    fn clear(&mut self) {
        self.ops.clear();
        self.any = false;
    }

    fn see(&mut self, x: f32, y: f32) {
        if !self.any {
            self.min = (x, y);
            self.max = (x, y);
            self.any = true;
        } else {
            self.min = (self.min.0.min(x), self.min.1.min(y));
            self.max = (self.max.0.max(x), self.max.1.max(y));
        }
    }
}

impl PathSink for Recorder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.see(x, y);
        self.ops.push(Op::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.see(x, y);
        self.ops.push(Op::Line(x, y));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.see(cx0, cy0);
        self.see(x, y);
        self.ops.push(Op::Quad(cx0, cy0, x, y));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.see(cx0, cy0);
        self.see(cx1, cy1);
        self.see(x, y);
        self.ops.push(Op::Cubic(cx0, cy0, cx1, cy1, x, y));
    }
    fn close(&mut self) {
        self.ops.push(Op::Close);
    }
}

/// Reusable scratch for rendering glyphs: scaler fonts per (face, size), the path recorder and
/// the accumulation buffer.
#[derive(Default)]
pub struct GlyphRasterizer {
    scalers: HashMap<(FontId, u32), ScalerFont>,
    rec: Recorder,
    acc: Accumulator,
}

impl GlyphRasterizer {
    pub fn new() -> GlyphRasterizer {
        GlyphRasterizer::default()
    }

    /// Renders one glyph, or `None` for an empty outline (a space) or an oversized glyph.
    pub fn rasterize(&mut self, db: &FontDb, key: GlyphKey) -> Option<GlyphBitmap> {
        let size = key.size();
        if !(size > 0.0 && size.is_finite()) {
            return None;
        }
        let font = db.face(key.font)?.font()?;
        let scaler = self
            .scalers
            .entry((key.font, key.size_q))
            .or_insert_with(|| ScalerFont::unhinted(font.clone(), Some(size)));
        let outline = scaler.outline(GlyphId::new(key.glyph))?;
        self.rec.clear();
        outline.draw(DrawMemory::Scratch, &mut self.rec).ok()?;
        if !self.rec.any {
            return None;
        }
        let shift = f32::from(key.sub) / f32::from(SUBPIXEL_STEPS);
        let minx = (self.rec.min.0 + shift).floor();
        let maxx = (self.rec.max.0 + shift).ceil();
        let miny = self.rec.min.1.floor();
        let maxy = self.rec.max.1.ceil();
        let w = trunc_i32(maxx - minx);
        let h = trunc_i32(maxy - miny);
        if w <= 0 || h <= 0 || w > MAX_GLYPH_SIDE || h > MAX_GLYPH_SIDE {
            return None;
        }
        self.acc.reset(w.ix(), h.ix());
        {
            let mut f = Flattener::new(&mut self.acc, 1.0, 1.0, shift - minx, maxy);
            for op in &self.rec.ops {
                match *op {
                    Op::Move(x, y) => f.move_to(x, y),
                    Op::Line(x, y) => f.line_to(x, y),
                    Op::Quad(a, b, x, y) => f.quad_to(a, b, x, y),
                    Op::Cubic(a, b, c, d, x, y) => f.curve_to(a, b, c, d, x, y),
                    Op::Close => f.close(),
                }
            }
            f.close();
        }
        let mut data = vec![0u8; (w * h).ix()];
        self.acc.finish(&mut data);
        Some(GlyphBitmap {
            width: to_u16(w),
            height: to_u16(h),
            left: to_i16(trunc_i32(minx)),
            top: to_i16(trunc_i32(maxy)),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subpixel_split_rounds_down() {
        assert_eq!(split_subpixel(3.0), (3, 0));
        assert_eq!(split_subpixel(3.24), (3, 0));
        assert_eq!(split_subpixel(3.25), (3, 1));
        assert_eq!(split_subpixel(3.99), (3, 3));
        assert_eq!(split_subpixel(-0.1), (-1, 3));
        assert_eq!(split_subpixel(-1.0), (-1, 0));
    }
}
