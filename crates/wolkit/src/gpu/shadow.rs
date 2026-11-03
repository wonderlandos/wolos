// SPDX-License-Identifier: BSD-2-Clause
//! Nine-slice shadow masks for the GPU path (plan section 2.6).
//!
//! A shadow is the CPU rasteriser's own mask: rounded-rectangle coverage blurred by three box
//! blurs (`src/cpu/raster.rs`, `box_blur3`). The arithmetic below is a copy of that code, kept
//! identical by `tests/gpu_shadow.rs`, which compares every pixel of a stretched mask against a
//! CPU render. What makes it cheap is the stretch: away from the corners every column of the
//! mask is the same (the coverage there depends on `y` only, and three box blurs of radius `r`
//! reach `3r` columns), so the atlas keeps a *prototype*, the mask of the same shadow made
//! shorter by a whole number of columns and rows, and the shader repeats its middle column and
//! row. Prototypes are keyed by everything that shapes them (sub-pixel origin, reduced size,
//! radii, blur), so a card list of one style shares one prototype however long the cards are.

use std::collections::HashMap;

use crate::paint::geom::{PixelRect, Rect};
use super::{f2i, f2u};
use crate::scene::ids::{Ix, to_u16, to_u32};

/// Largest mask the CPU rasteriser draws (`raster.rs` skips bigger ones too).
const MAX_MASK_PX: i64 = 16_000_000;

#[inline]
fn cov_u8(c: f32) -> u8 {
    // The rasteriser's `(c.clamp(0.0, 1.0) * 255.0 + 0.5) as u32`.
    u8::try_from(f2u(c.clamp(0.0, 1.0) * 255.0 + 0.5)).unwrap_or(u8::MAX)
}

/// Coverage of the pixel centred at `(px, py)` by a rounded rectangle (the rasteriser's
/// `rrect_cov`, operation for operation).
#[inline]
pub fn rrect_cov(px: f32, py: f32, r: &Rect, radii: &[f32; 4]) -> f32 {
    let hw = r.w * 0.5;
    let hh = r.h * 0.5;
    let qx = px - (r.x + hw);
    let qy = py - (r.y + hh);
    let rad = match (qx < 0.0, qy < 0.0) {
        (true, true) => radii[0],
        (false, true) => radii[1],
        (false, false) => radii[2],
        (true, false) => radii[3],
    };
    let dx = qx.abs() - (hw - rad);
    let dy = qy.abs() - (hh - rad);
    let ox = dx.max(0.0);
    let oy = dy.max(0.0);
    let d = (ox * ox + oy * oy).sqrt() + dx.max(dy).min(0.0) - rad;
    (0.5 - d).clamp(0.0, 1.0)
}

/// The box radius the rasteriser blurs with for a blur of `blur` device pixels.
pub fn box_radius(blur: f32) -> usize {
    if blur <= 0.0 {
        return 0;
    }
    let sigma = blur * 0.5;
    let size = f2u((4.0 * sigma * sigma + 1.0).sqrt().floor()).ix();
    let size = if size.is_multiple_of(2) { size + 1 } else { size }.max(1);
    size / 2
}

fn blur_line(src: &[u8], dst: &mut [u8], r: usize) {
    let n = src.len();
    let size = to_u32(2 * r + 1);
    let mut sum: u32 = 0;
    for v in src.iter().take(r.min(n)) {
        sum += u32::from(*v);
    }
    for i in 0..n {
        if i + r < n {
            sum += u32::from(src[i + r]);
        }
        if i > r {
            sum -= u32::from(src[i - r - 1]);
        }
        dst[i] = u8::try_from((sum + size / 2) / size).unwrap_or(u8::MAX);
    }
}

/// Three box blurs approximating a Gaussian of standard deviation `blur / 2` (`raster.rs`).
pub fn box_blur3(mask: &mut [u8], w: usize, h: usize, blur: f32) {
    if blur <= 0.0 || w == 0 || h == 0 {
        return;
    }
    let r = box_radius(blur);
    let mut tmp = vec![0u8; mask.len()];
    let mut col = vec![0u8; h];
    let mut out = vec![0u8; h];
    for _ in 0..3 {
        for y in 0..h {
            blur_line(&mask[y * w..(y + 1) * w], &mut tmp[y * w..(y + 1) * w], r);
        }
        for x in 0..w {
            for y in 0..h {
                col[y] = tmp[y * w + x];
            }
            blur_line(&col, &mut out, r);
            for y in 0..h {
                mask[y * w + x] = out[y];
            }
        }
    }
}

/// The mask the rasteriser draws for a shadow cast by `cast` (window device pixels): its pixel
/// rectangle and the coverage bytes, row-major.
pub fn mask(cast: &Rect, radii: &[f32; 4], blur: f32) -> Option<(PixelRect, Vec<u8>)> {
    let blur = blur.clamp(0.0, 256.0);
    let ext = extent(cast, blur)?;
    let (mw, mh) = (ext.width(), ext.height());
    let mut m = vec![0u8; usize::try_from(mw).ok()? * usize::try_from(mh).ok()?];
    let wu = usize::try_from(mw).ok()?;
    for (y, row) in m.chunks_mut(wu).enumerate() {
        let py = y as f32 + 0.5;
        for (x, v) in row.iter_mut().enumerate() {
            *v = cov_u8(rrect_cov(
                ext.x0 as f32 + x as f32 + 0.5,
                ext.y0 as f32 + py,
                cast,
                radii,
            ));
        }
    }
    box_blur3(&mut m, wu, usize::try_from(mh).ok()?, blur);
    Some((ext, m))
}

fn margin(blur: f32) -> i32 {
    f2i((blur * 1.5).ceil()) + 2
}

/// The pixel rectangle the mask of a shadow covers (the rasteriser's `ext`).
pub fn extent(cast: &Rect, blur: f32) -> Option<PixelRect> {
    if cast.is_empty() {
        return None;
    }
    let m = margin(blur.clamp(0.0, 256.0));
    let a = cast.to_pixels();
    let ext = PixelRect {
        x0: a.x0 - m,
        y0: a.y0 - m,
        x1: a.x1 + m,
        y1: a.y1 + m,
    };
    (i64::from(ext.width()) * i64::from(ext.height()) <= MAX_MASK_PX).then_some(ext)
}

/// How a shadow maps onto its prototype: columns `< split` are taken as they are, the next
/// `stretch` columns repeat column `split`, the rest shift left by `stretch` (rows the same).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slicing {
    pub split_x: u32,
    pub split_y: u32,
    pub stretch_x: u32,
    pub stretch_y: u32,
}

/// The constant band along one axis: `(first, last)` mask indices of identical columns.
fn band(start: f32, len: f32, rad_lo: f32, rad_hi: f32, ext0: i32, n: i32, r: usize) -> (i32, i32) {
    let reach = (3 * r) as f32 + 1.5;
    let lo = (start + rad_lo + reach - ext0 as f32 - 0.5).ceil();
    let hi = (start + len - rad_hi - reach - ext0 as f32 - 0.5).floor();
    (f2i(lo).max(0), f2i(hi).min(n - 1))
}

/// The prototype shadow (a shorter cast) and the slicing that stretches it back.
pub fn prototype(cast: &Rect, radii: &[f32; 4], blur: f32) -> Option<(Rect, Slicing, PixelRect)> {
    let blur = blur.clamp(0.0, 256.0);
    let ext = extent(cast, blur)?;
    let r = box_radius(blur);
    let (bx0, bx1) = band(cast.x, cast.w, radii[0].max(radii[3]), radii[1].max(radii[2]), ext.x0, ext.width(), r);
    let (by0, by1) = band(cast.y, cast.h, radii[0].max(radii[1]), radii[3].max(radii[2]), ext.y0, ext.height(), r);
    let nx = (bx1 - bx0).max(0);
    let ny = (by1 - by0).max(0);
    let proto = Rect::new(cast.x, cast.y, cast.w - nx as f32, cast.h - ny as f32);
    let s = Slicing {
        split_x: u32::try_from(bx0.max(0)).unwrap_or(0),
        split_y: u32::try_from(by0.max(0)).unwrap_or(0),
        stretch_x: u32::try_from(nx).unwrap_or(0),
        stretch_y: u32::try_from(ny).unwrap_or(0),
    };
    Some((proto, s, ext))
}

/// Where a prototype sits in the shadow atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub slicing: Slicing,
}

struct Shelf {
    y: u32,
    h: u32,
    x: u32,
}

/// An R8 atlas of shadow prototypes with a shelf packer. When full it starts over (the
/// generation changes and every prototype is made again on next use).
pub struct ShadowAtlas {
    size: u32,
    pub pixels: Vec<u8>,
    shelves: Vec<Shelf>,
    next_y: u32,
    map: HashMap<[u32; 10], Placed>,
    pub generation: u64,
    /// Pixels changed since the last [`ShadowAtlas::take_dirty`].
    dirty: Option<PixelRect>,
    pub stats: ShadowStats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShadowStats {
    pub hits: u64,
    pub made: u64,
    /// Shadows too large for the atlas even when empty (not drawn on the GPU).
    pub dropped: u64,
}

const PAD: u32 = 1;

impl ShadowAtlas {
    pub fn new(size: u32) -> ShadowAtlas {
        let size = size.clamp(64, 8192);
        ShadowAtlas {
            size,
            pixels: vec![0; size.ix() * size.ix()],
            shelves: Vec::new(),
            next_y: 0,
            map: HashMap::new(),
            generation: 1,
            dirty: None,
            stats: ShadowStats::default(),
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    fn reset(&mut self) {
        self.shelves.clear();
        self.next_y = 0;
        self.map.clear();
        self.generation += 1;
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (w, h) = (w + 2 * PAD, h + 2 * PAD);
        if w > self.size || h > self.size {
            return None;
        }
        for s in &mut self.shelves {
            if h <= s.h && s.x + w <= self.size {
                let x = s.x;
                s.x += w;
                return Some((x + PAD, s.y + PAD));
            }
        }
        if self.next_y + h > self.size {
            return None;
        }
        let y = self.next_y;
        self.next_y += h;
        self.shelves.push(Shelf { y, h, x: w });
        Some((PAD, y + PAD))
    }

    /// The prototype for a shadow cast by `cast` (window device pixels), made on a miss, and
    /// the mask's pixel rectangle on screen.
    pub fn get(&mut self, cast: &Rect, radii: &[f32; 4], blur: f32) -> Option<(Placed, PixelRect)> {
        let (proto, slicing, ext) = prototype(cast, radii, blur)?;
        let key = [
            (cast.x - cast.x.floor()).to_bits(),
            (cast.y - cast.y.floor()).to_bits(),
            proto.w.to_bits(),
            proto.h.to_bits(),
            radii[0].to_bits(),
            radii[1].to_bits(),
            radii[2].to_bits(),
            radii[3].to_bits(),
            blur.clamp(0.0, 256.0).to_bits(),
            slicing.split_x | (slicing.split_y << 16),
        ];
        if let Some(p) = self.map.get(&key) {
            self.stats.hits += 1;
            let mut p = *p;
            p.slicing = slicing;
            return Some((p, ext));
        }
        let (pext, m) = mask(&proto, radii, blur)?;
        let (w, h) = (u32::try_from(pext.width()).ok()?, u32::try_from(pext.height()).ok()?);
        let at = match self.alloc(w, h) {
            Some(a) => a,
            None => {
                self.reset();
                match self.alloc(w, h) {
                    Some(a) => a,
                    None => {
                        self.stats.dropped += 1;
                        return None;
                    }
                }
            }
        };
        for (row, line) in m.chunks(w.ix()).enumerate() {
            let o = (at.1.ix() + row) * self.size.ix() + at.0.ix();
            self.pixels[o..o + w.ix()].copy_from_slice(line);
        }
        let rect = PixelRect {
            x0: i32::try_from(at.0).ok()?,
            y0: i32::try_from(at.1).ok()?,
            x1: i32::try_from(at.0 + w).ok()?,
            y1: i32::try_from(at.1 + h).ok()?,
        };
        self.dirty = Some(match self.dirty {
            None => rect,
            Some(d) => PixelRect {
                x0: d.x0.min(rect.x0),
                y0: d.y0.min(rect.y0),
                x1: d.x1.max(rect.x1),
                y1: d.y1.max(rect.y1),
            },
        });
        let placed = Placed {
            x: to_u16(at.0),
            y: to_u16(at.1),
            w: to_u16(w),
            h: to_u16(h),
            slicing,
        };
        self.map.insert(key, placed);
        self.stats.made += 1;
        Some((placed, ext))
    }

    /// The rectangle to upload since the last call.
    pub fn take_dirty(&mut self) -> Option<PixelRect> {
        self.dirty.take()
    }

    /// Drops every prototype (Memory Saver trim).
    pub fn trim(&mut self) {
        self.reset();
    }
}

/// Reads the stretched mask back on the CPU, the way the shader samples it (tests and checks).
pub fn sample(atlas: &ShadowAtlas, p: &Placed, mx: u32, my: u32) -> u8 {
    let s = |m: u32, split: u32, n: u32| {
        if m < split {
            m
        } else if m < split + n {
            split
        } else {
            m - n
        }
    };
    let qx = s(mx, p.slicing.split_x, p.slicing.stretch_x);
    let qy = s(my, p.slicing.split_y, p.slicing.stretch_y);
    let i = (u32::from(p.y) + qy).ix() * atlas.size.ix() + (u32::from(p.x) + qx).ix();
    atlas.pixels.get(i).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stretched_equals_full(cast: Rect, radii: [f32; 4], blur: f32) {
        let mut a = ShadowAtlas::new(1024);
        let (p, ext) = a.get(&cast, &radii, blur).unwrap_or_else(|| unreachable_none());
        let (fext, full) = mask(&cast, &radii, blur).unwrap_or_else(|| unreachable_none());
        assert_eq!(ext, fext);
        let w = u32::try_from(ext.width()).unwrap_or(0);
        let h = u32::try_from(ext.height()).unwrap_or(0);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    sample(&a, &p, x, y),
                    full[(y * w + x).ix()],
                    "pixel {x},{y} of {cast:?} r={radii:?} blur={blur}"
                );
            }
        }
    }

    fn unreachable_none<T>() -> T {
        unreachable!("a shadow of this size always has a mask")
    }

    #[test]
    fn stretched_prototype_equals_the_full_mask() {
        stretched_equals_full(Rect::new(20.0, 30.0, 300.0, 120.0), [8.0; 4], 12.0);
        stretched_equals_full(Rect::new(20.5, 30.25, 211.3, 97.6), [4.0, 9.0, 2.0, 0.0], 7.0);
        stretched_equals_full(Rect::new(3.0, 4.0, 40.0, 30.0), [6.0; 4], 0.0);
        stretched_equals_full(Rect::new(10.0, 10.0, 18.0, 400.0), [9.0; 4], 24.0);
    }

    #[test]
    fn same_style_cards_share_one_prototype() {
        let mut a = ShadowAtlas::new(1024);
        for i in 0..20u16 {
            let w = 200.0 + f32::from(i) * 13.0;
            let y = 10.0 + f32::from(i) * 90.0;
            assert!(a.get(&Rect::new(16.0, y, w, 60.0), &[8.0; 4], 12.0).is_some());
        }
        assert_eq!(a.stats.made, 1, "{:?}", a.stats);
        assert_eq!(a.stats.hits, 19);
    }

    #[test]
    fn full_atlas_starts_over_and_reports_it() {
        let mut a = ShadowAtlas::new(128);
        let g = a.generation;
        for i in 0..12u16 {
            let _ = a.get(&Rect::new(0.0, 0.0, 60.0, 60.0), &[f32::from(i); 4], 4.0);
        }
        assert!(a.generation > g);
        assert!(a.get(&Rect::new(0.0, 0.0, 600.0, 600.0), &[290.0; 4], 0.0).is_none());
        assert_eq!(a.stats.dropped, 1);
    }

    #[test]
    fn box_radius_matches_the_rasteriser_formula() {
        for (blur, want) in [(0.0, 0), (1.0, 0), (2.0, 1), (12.0, 6), (24.0, 12), (7.0, 3)] {
            assert_eq!(box_radius(blur), want, "blur {blur}");
        }
    }
}
