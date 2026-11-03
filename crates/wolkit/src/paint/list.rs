// SPDX-License-Identifier: BSD-2-Clause
//! The display list and its paint chunks.
//!
//! A [`DisplayList`] is a flat vector of [`Item`]s partitioned into [`Chunk`]s. A chunk is the
//! unit that is cached and moved as a whole: a scroll container's content, an overlay, a canvas,
//! a large static panel. Items inside a chunk are in chunk-local coordinates; the chunk's
//! `offset` is its transform (scrolling changes only the offset and re-records nothing). Chunks
//! draw in `z` order, then recording order.
//!
//! The list owns its storage and is meant to be cleared and re-filled each frame without
//! reallocating (`clear` keeps capacity).

use std::ops::Range;

use crate::paint::geom::{Color, Rect};
use crate::text::fontdb::FontId;

/// Corner radii: top-left, top-right, bottom-right, bottom-left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Radii(pub [f32; 4]);

impl Radii {
    pub const ZERO: Radii = Radii([0.0; 4]);

    pub fn uniform(r: f32) -> Radii {
        Radii([r; 4])
    }

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|r| *r <= 0.0)
    }

    pub fn max(&self) -> f32 {
        self.0.iter().copied().fold(0.0, f32::max)
    }

    /// Radii clamped so adjacent corners fit inside `rect` (CSS rule: scale all by the worst
    /// ratio).
    pub fn fitted(&self, w: f32, h: f32) -> Radii {
        let r = self.0.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
        let mut k = 1.0f32;
        for (a, b, len) in [(r[0], r[1], w), (r[3], r[2], w), (r[0], r[3], h), (r[1], r[2], h)] {
            let s = a + b;
            if s > len && s > 0.0 {
                k = k.min(len / s);
            }
        }
        Radii(r.map(|v| v * k))
    }
}

/// What a rectangle is filled with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Solid(Color),
    /// Two-stop linear gradient from point `p0` to `p1`, given as fractions of the rectangle.
    Linear { from: Color, to: Color, p0: (f32, f32), p1: (f32, f32) },
}

impl Fill {
    pub fn vertical(top: Color, bottom: Color) -> Fill {
        Fill::Linear { from: top, to: bottom, p0: (0.0, 0.0), p1: (0.0, 1.0) }
    }

    pub fn horizontal(left: Color, right: Color) -> Fill {
        Fill::Linear { from: left, to: right, p0: (0.0, 0.0), p1: (1.0, 0.0) }
    }
}

/// One positioned glyph: id and the pen position on the baseline, chunk-local pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Rect { rect: Rect, radii: Radii, fill: Fill },
    Border { rect: Rect, radii: Radii, width: f32, color: Color },
    /// A blurred rounded rectangle (`rect` is the casting box; the offset and spread are applied
    /// here). Drawn from a nine-slice texture on the GPU and as a blurred mask on the CPU.
    Shadow { rect: Rect, radii: Radii, blur: f32, spread: f32, dx: f32, dy: f32, color: Color },
    Glyphs { font: FontId, size: f32, color: Color, glyphs: Range<u32> },
    /// Intersects the clip with a (rounded) rectangle until the matching `PopClip`.
    PushClip { rect: Rect, radius: f32 },
    PopClip,
    /// Multiplies the alpha of everything until `PopOpacity`. With `overlap` false the factor is
    /// pushed down to the leaves; with `overlap` true (children overlap) a layer is composed.
    PushOpacity { alpha: u8, overlap: bool },
    PopOpacity,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    /// Caller's id (the scene node id of the chunk root).
    pub id: u32,
    /// Bumped by the caller whenever the content changed; the instance builder re-emits a chunk
    /// only when its version differs from the cached one.
    pub version: u64,
    /// Bounds in chunk-local coordinates.
    pub bounds: Rect,
    /// The chunk's translation (scroll position, overlay anchor).
    pub offset: (f32, f32),
    pub z: i32,
    pub items: Range<u32>,
}

impl Chunk {
    /// Bounds in window coordinates.
    pub fn window_bounds(&self) -> Rect {
        self.bounds.translate(self.offset.0, self.offset.1)
    }
}

#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    pub items: Vec<Item>,
    pub glyphs: Vec<PosGlyph>,
    pub chunks: Vec<Chunk>,
    open: Option<usize>,
}

impl DisplayList {
    pub fn new() -> DisplayList {
        DisplayList::default()
    }

    /// Empties the list but keeps the allocations.
    pub fn clear(&mut self) {
        self.items.clear();
        self.glyphs.clear();
        self.chunks.clear();
        self.open = None;
    }

    /// Starts a chunk; items pushed until `end_chunk` belong to it.
    pub fn begin_chunk(&mut self, id: u32, version: u64, bounds: Rect, offset: (f32, f32), z: i32) {
        self.end_chunk();
        let at = self.items.len() as u32;
        // `u32::MAX` marks the open chunk: it extends to the end of the item vector.
        self.chunks.push(Chunk { id, version, bounds, offset, z, items: at..u32::MAX });
        self.open = Some(self.chunks.len() - 1);
    }

    pub fn end_chunk(&mut self) {
        if let Some(i) = self.open.take() {
            let end = self.items.len() as u32;
            self.chunks[i].items.end = end;
        }
    }

    fn ensure_chunk(&mut self) {
        if self.open.is_none() {
            self.begin_chunk(0, 0, Rect::new(-1.0e6, -1.0e6, 2.0e6, 2.0e6), (0.0, 0.0), 0);
        }
    }

    pub fn push(&mut self, item: Item) {
        self.ensure_chunk();
        self.items.push(item);
    }

    pub fn rect(&mut self, rect: Rect, radii: Radii, fill: Fill) {
        self.push(Item::Rect { rect, radii, fill });
    }

    pub fn solid(&mut self, rect: Rect, color: Color) {
        self.rect(rect, Radii::ZERO, Fill::Solid(color));
    }

    pub fn border(&mut self, rect: Rect, radii: Radii, width: f32, color: Color) {
        self.push(Item::Border { rect, radii, width, color });
    }

    pub fn shadow(&mut self, rect: Rect, radii: Radii, blur: f32, spread: f32, dx: f32, dy: f32, color: Color) {
        self.push(Item::Shadow { rect, radii, blur, spread, dx, dy, color });
    }

    /// Adds a run of glyphs sharing a font, size and colour.
    pub fn glyph_run(&mut self, font: FontId, size: f32, color: Color, glyphs: impl IntoIterator<Item = PosGlyph>) {
        self.ensure_chunk();
        let a = self.glyphs.len() as u32;
        self.glyphs.extend(glyphs);
        let b = self.glyphs.len() as u32;
        if b > a {
            self.items.push(Item::Glyphs { font, size, color, glyphs: a..b });
        }
    }

    pub fn push_clip(&mut self, rect: Rect, radius: f32) {
        self.push(Item::PushClip { rect, radius });
    }

    pub fn pop_clip(&mut self) {
        self.push(Item::PopClip);
    }

    pub fn push_opacity(&mut self, alpha: u8, overlap: bool) {
        self.push(Item::PushOpacity { alpha, overlap });
    }

    pub fn pop_opacity(&mut self) {
        self.push(Item::PopOpacity);
    }

    /// Chunk indices in draw order (stable sort by `z`).
    pub fn draw_order(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.chunks.len()).collect();
        v.sort_by_key(|&i| self.chunks[i].z);
        v
    }

    /// The items of a chunk. Call after `end_chunk`.
    pub fn chunk_items(&self, c: &Chunk) -> &[Item] {
        let end = (c.items.end as usize).min(self.items.len());
        &self.items[(c.items.start as usize).min(end)..end]
    }

    /// Finalises an open chunk; call before reading chunk item ranges.
    pub fn finish(&mut self) {
        self.end_chunk();
    }

    pub fn glyph_slice(&self, r: &Range<u32>) -> &[PosGlyph] {
        let end = (r.end as usize).min(self.glyphs.len());
        &self.glyphs[(r.start as usize).min(end)..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radii_are_fitted_to_the_box() {
        let r = Radii::uniform(30.0).fitted(40.0, 100.0);
        assert!(r.0.iter().all(|v| (*v - 20.0).abs() < 1e-4), "{r:?}");
        assert_eq!(Radii::uniform(4.0).fitted(40.0, 40.0), Radii::uniform(4.0));
        assert!(Radii([f32::NAN, -1.0, 2.0, 3.0]).fitted(10.0, 10.0).0.iter().all(|v| *v >= 0.0));
    }

    #[test]
    fn chunks_partition_items() {
        let mut l = DisplayList::new();
        l.begin_chunk(1, 1, Rect::new(0.0, 0.0, 10.0, 10.0), (0.0, 0.0), 0);
        l.solid(Rect::new(0.0, 0.0, 5.0, 5.0), Color::rgb(1, 2, 3));
        l.begin_chunk(2, 1, Rect::new(0.0, 0.0, 10.0, 10.0), (0.0, 4.0), -1);
        l.solid(Rect::new(1.0, 1.0, 2.0, 2.0), Color::rgb(4, 5, 6));
        l.solid(Rect::new(3.0, 3.0, 2.0, 2.0), Color::rgb(4, 5, 6));
        l.finish();
        assert_eq!(l.chunk_items(&l.chunks[0]).len(), 1);
        assert_eq!(l.chunk_items(&l.chunks[1]).len(), 2);
        assert_eq!(l.draw_order(), vec![1, 0]);
    }

    #[test]
    fn clear_keeps_capacity() {
        let mut l = DisplayList::new();
        for _ in 0..100 {
            l.solid(Rect::new(0.0, 0.0, 1.0, 1.0), Color::rgb(0, 0, 0));
        }
        l.finish();
        let cap = l.items.capacity();
        l.clear();
        assert!(l.items.is_empty() && l.items.capacity() >= cap);
    }
}
