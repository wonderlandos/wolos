// SPDX-License-Identifier: BSD-2-Clause
//! The CPU reference rasteriser.
//!
//! It turns a [`DisplayList`] into a premultiplied RGBA8 [`Image`]. Work is split into 64x64
//! tiles; each tile is composed in a local buffer from the primitives that touch it, in z order.
//!
//! Bit-exactness: coverage is computed with IEEE `+ - * /`, `sqrt`, `floor`, `abs`, `min`, `max`
//! (no libm transcendental, no fused multiply-add), blending and blurs use integer arithmetic
//! with an exact `round(x / 255)`. The same list therefore renders to the same bytes on every
//! machine; `tests/golden` holds the reference images. The tile size does not change the output
//! (every pixel is a function of the pixel position alone), which a test checks.
//!
//! The rasteriser is the default on machines without 3D, the fallback after a device loss, and
//! the oracle the GPU path is compared against with a tolerance.

use crate::num::{ceil_i32, floor_i32, to_i32, trunc_u32, trunc_usize};
use crate::scene::ids::{Ix, to_u8, to_u32};
use std::ops::Range;

use crate::paint::geom::{Color, PixelRect, Rect, div255, mix};
use crate::paint::glyphs::GlyphSource;
use crate::paint::list::{DisplayList, Fill, Item, Radii};
use crate::text::glyph::split_subpixel;

/// Most soft (rounded or fractional) clips applied to one primitive.
const MAX_SOFT_CLIPS: usize = 8;

/// Tile edge in pixels.
pub const TILE: i32 = 64;

/// A premultiplied RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32, fill: Color) -> Image {
        let px = fill.premul();
        let mut data = Vec::with_capacity(width.ix() * height.ix() * 4);
        for _ in 0..(width.ix() * height.ix()) {
            data.extend_from_slice(&px);
        }
        Image {
            width,
            height,
            data,
        }
    }

    /// Straight-alpha RGBA bytes (un-premultiplied with exact rounding; alpha 0 gives zeros).
    pub fn to_straight(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for p in out.as_chunks_mut::<4>().0 {
            let a = u32::from(p[3]);
            if a == 0 {
                p.fill(0);
            } else if a != 255 {
                for c in &mut p[..3] {
                    *c = to_u8(((u32::from(*c) * 255 + a / 2) / a).min(255));
                }
            }
        }
        out
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4).ix();
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }
}

/// Options for [`render`].
#[derive(Clone, Copy, Debug)]
pub struct RasterOptions {
    /// Device pixels per logical pixel.
    pub scale: f32,
    /// What the image starts as.
    pub clear: Color,
    /// Tile edge; only for tests (`TILE` in production).
    pub tile: i32,
}

impl Default for RasterOptions {
    fn default() -> Self {
        RasterOptions {
            scale: 1.0,
            clear: Color::TRANSPARENT,
            tile: TILE,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Coverage primitives
// ---------------------------------------------------------------------------------------------

#[inline]
fn cov_u8(c: f32) -> u32 {
    trunc_u32(c.clamp(0.0, 1.0) * 255.0 + 0.5)
}

/// Coverage (0..=1) of the pixel centred at `(px, py)` by a rounded rectangle.
#[inline]
fn rrect_cov(px: f32, py: f32, r: &Rect, radii: &[f32; 4]) -> f32 {
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

// ---------------------------------------------------------------------------------------------
// Prepared primitives (device space)
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum PFill {
    Solid(Color),
    Grad {
        c0: Color,
        c1: Color,
        p0: (f32, f32),
        d: (f32, f32),
        inv: f32,
    },
}

enum Kind {
    Rrect {
        rect: Rect,
        radii: [f32; 4],
        fill: PFill,
    },
    Border {
        outer: Rect,
        ro: [f32; 4],
        inner: Rect,
        ri: [f32; 4],
        color: Color,
    },
    Mask {
        ox: i32,
        oy: i32,
        w: i32,
        h: i32,
        off: usize,
        color: Color,
    },
    LayerBegin {
        alpha: u8,
        end: usize,
    },
    LayerEnd {
        alpha: u8,
    },
}

struct Prim {
    kind: Kind,
    bounds: PixelRect,
    clip: u32,
}

#[derive(Clone)]
struct Clip {
    bounds: PixelRect,
    soft: Range<u32>,
}

#[derive(Default)]
struct Prepared {
    prims: Vec<Prim>,
    clips: Vec<Clip>,
    soft: Vec<(Rect, [f32; 4])>,
    masks: Vec<u8>,
}

fn is_aligned(r: &Rect) -> bool {
    r.x.fract() == 0.0 && r.y.fract() == 0.0 && r.w.fract() == 0.0 && r.h.fract() == 0.0
}

fn scale_rect(r: &Rect, s: f32, ox: f32, oy: f32) -> Rect {
    Rect::new((r.x + ox) * s, (r.y + oy) * s, r.w * s, r.h * s)
}

fn scale_radii(r: &Radii, s: f32, w: f32, h: f32) -> [f32; 4] {
    Radii(r.0.map(|v| v * s)).fitted(w, h).0
}

/// Where items of the chunk being prepared land: its scale and offset, the folded opacity of
/// non-overlapping groups and the current clip.
#[derive(Clone, Copy)]
struct Cur {
    scale: f32,
    ox: f32,
    oy: f32,
    opacity: u32,
    clip: u32,
    clip_bounds: PixelRect,
}

impl Cur {
    fn rect(&self, r: &Rect) -> Rect {
        scale_rect(r, self.scale, self.ox, self.oy)
    }

    fn color(&self, c: Color) -> Color {
        c.with_opacity(to_u8(self.opacity))
    }
}

/// Per-chunk stacks: clips, group opacities and open layers.
struct Stacks {
    clip: Vec<u32>,
    /// (alpha, is a layer)
    alpha: Vec<(u8, bool)>,
    /// Prim index of each open `LayerBegin`.
    layer: Vec<usize>,
    opacity: Vec<u32>,
}

fn prepare(
    list: &DisplayList,
    glyphs: &mut dyn GlyphSource,
    w: i32,
    h: i32,
    scale: f32,
) -> Prepared {
    let mut p = Prepared::default();
    let full = PixelRect {
        x0: 0,
        y0: 0,
        x1: w,
        y1: h,
    };
    for ci in list.draw_order() {
        let chunk = &list.chunks[ci];
        let (ox, oy) = chunk.offset;
        p.clips.push(Clip {
            bounds: full,
            soft: 0..0,
        });
        let root_clip = to_u32(p.clips.len() - 1);
        let cb = scale_rect(&chunk.bounds, scale, ox, oy).to_pixels();
        // Chunk bounds clip its content.
        let mut chunk_clip = p.clips[root_clip.ix()].clone();
        chunk_clip.bounds = chunk_clip.bounds.intersect(&cb);
        p.clips.push(chunk_clip);
        let mut st = Stacks {
            clip: vec![root_clip, to_u32(p.clips.len() - 1)],
            alpha: Vec::new(),
            layer: Vec::new(),
            opacity: Vec::new(),
        };
        // Folded product for non-overlap groups.
        let mut opacity: u32 = 255;
        for item in list.chunk_items(chunk) {
            let clip = *st.clip.last().unwrap_or(&root_clip);
            let cur = Cur {
                scale,
                ox,
                oy,
                opacity,
                clip,
                clip_bounds: p.clips[clip.ix()].bounds,
            };
            match item {
                Item::PushClip { rect, radius } => {
                    let id = p.push_clip(&cur, rect, *radius);
                    st.clip.push(id);
                }
                Item::PopClip => {
                    if st.clip.len() > 2 {
                        st.clip.pop();
                    }
                }
                Item::PushOpacity { alpha, overlap } => {
                    if *overlap {
                        st.layer.push(p.begin_layer(*alpha, clip));
                        st.alpha.push((*alpha, true));
                    } else {
                        st.opacity.push(opacity);
                        opacity = div255(opacity * u32::from(*alpha));
                        st.alpha.push((*alpha, false));
                    }
                }
                Item::PopOpacity => match st.alpha.pop() {
                    Some((alpha, true)) => {
                        if let Some(b) = st.layer.pop() {
                            p.end_layer(b, alpha);
                        }
                    }
                    Some((_, false)) => opacity = st.opacity.pop().unwrap_or(255),
                    None => {}
                },
                Item::Rect { rect, radii, fill } => p.rrect(&cur, rect, radii, fill),
                Item::Border {
                    rect,
                    radii,
                    width,
                    color,
                } => p.border(&cur, rect, radii, *width, *color),
                Item::Shadow {
                    rect,
                    radii,
                    blur,
                    spread,
                    dx,
                    dy,
                    color,
                } => {
                    let sh = crate::paint::list::Shadow::new(*blur, *spread, *dx, *dy, *color);
                    p.shadow(&cur, rect, radii, &sh);
                }
                Item::Glyphs {
                    font,
                    size,
                    color,
                    glyphs: range,
                } => p.glyphs(&cur, glyphs, *font, *size, *color, list.glyph_slice(range)),
            }
        }
        // Close any layer left open by an unbalanced list.
        while let Some(b) = st.layer.pop() {
            p.close_open_layer(b, root_clip);
        }
    }
    p
}

impl Prepared {
    /// A clip inside the current one; returns its id.
    fn push_clip(&mut self, cur: &Cur, rect: &Rect, radius: f32) -> u32 {
        let r = cur.rect(rect);
        let parent = self.clips[cur.clip.ix()].clone();
        let mut bounds = parent.bounds.intersect(&r.to_pixels());
        let start = to_u32(self.soft.len());
        // Keep the innermost MAX_SOFT_CLIPS soft clips: deeper nesting is clipped by the
        // integer bounds only, which bounds the memory a hostile list can use.
        let from = (parent.soft.end.ix())
            .saturating_sub(MAX_SOFT_CLIPS - 1)
            .max(parent.soft.start.ix());
        let existing: Vec<(Rect, [f32; 4])> = self.soft[from..parent.soft.end.ix()].to_vec();
        self.soft.extend(existing);
        let rad = (radius.max(0.0) * cur.scale).min(r.w.min(r.h) * 0.5);
        if rad > 0.0 || !is_aligned(&r) {
            self.soft.push((r, [rad; 4]));
        }
        if r.is_empty() {
            bounds = PixelRect::default();
        }
        self.clips.push(Clip {
            bounds,
            soft: start..to_u32(self.soft.len()),
        });
        to_u32(self.clips.len() - 1)
    }

    fn begin_layer(&mut self, alpha: u8, clip: u32) -> usize {
        let idx = self.prims.len();
        self.prims.push(Prim {
            kind: Kind::LayerBegin {
                alpha,
                end: usize::MAX,
            },
            bounds: PixelRect::default(),
            clip,
        });
        idx
    }

    /// Union of what the prims after `b` touched.
    fn touched_since(&self, b: usize) -> PixelRect {
        let mut ub = PixelRect::default();
        for q in &self.prims[b + 1..] {
            ub = union_px(ub, q.bounds);
        }
        ub
    }

    fn end_layer(&mut self, b: usize, alpha: u8) {
        let end = self.prims.len();
        // Layer bounds: union of what the children touched, clipped.
        let cb = self.clips[self.prims[b].clip.ix()].bounds;
        self.prims[b].bounds = self.touched_since(b).intersect(&cb);
        if let Kind::LayerBegin { end: e, .. } = &mut self.prims[b].kind {
            *e = end;
        }
        let bounds = self.prims[b].bounds;
        self.prims.push(Prim {
            kind: Kind::LayerEnd { alpha },
            bounds,
            clip: self.prims[b].clip,
        });
    }

    fn close_open_layer(&mut self, b: usize, root_clip: u32) {
        let end = self.prims.len();
        let ub = self.touched_since(b);
        self.prims[b].bounds = ub;
        if let Kind::LayerBegin { end: e, alpha } = &mut self.prims[b].kind {
            *e = end;
            let a = *alpha;
            self.prims.push(Prim {
                kind: Kind::LayerEnd { alpha: a },
                bounds: ub,
                clip: root_clip,
            });
        }
    }

    fn rrect(&mut self, cur: &Cur, rect: &Rect, radii: &Radii, fill: &Fill) {
        let r = cur.rect(rect);
        let rad = scale_radii(radii, cur.scale, r.w, r.h);
        let bounds = r.to_pixels().intersect(&cur.clip_bounds);
        if bounds.is_empty() {
            return;
        }
        let fill = match *fill {
            Fill::Solid(c) => PFill::Solid(cur.color(c)),
            Fill::Linear { from, to, p0, p1 } => {
                let a = (r.x + p0.0 * r.w, r.y + p0.1 * r.h);
                let b = (r.x + p1.0 * r.w, r.y + p1.1 * r.h);
                let d = (b.0 - a.0, b.1 - a.1);
                let dd = d.0 * d.0 + d.1 * d.1;
                PFill::Grad {
                    c0: cur.color(from),
                    c1: cur.color(to),
                    p0: a,
                    d,
                    inv: if dd > 0.0 { 1.0 / dd } else { 0.0 },
                }
            }
        };
        self.prims.push(Prim {
            kind: Kind::Rrect {
                rect: r,
                radii: rad,
                fill,
            },
            bounds,
            clip: cur.clip,
        });
    }

    fn border(&mut self, cur: &Cur, rect: &Rect, radii: &Radii, width: f32, color: Color) {
        let r = cur.rect(rect);
        let bw = (width.max(0.0) * cur.scale).min(r.w.min(r.h) * 0.5);
        if bw <= 0.0 {
            return;
        }
        let bounds = r.to_pixels().intersect(&cur.clip_bounds);
        if bounds.is_empty() {
            return;
        }
        let ro = scale_radii(radii, cur.scale, r.w, r.h);
        let inner = Rect::new(r.x + bw, r.y + bw, r.w - 2.0 * bw, r.h - 2.0 * bw);
        let ri = ro.map(|v| (v - bw).max(0.0));
        self.prims.push(Prim {
            kind: Kind::Border {
                outer: r,
                ro,
                inner,
                ri,
                color: cur.color(color),
            },
            bounds,
            clip: cur.clip,
        });
    }

    fn shadow(&mut self, cur: &Cur, rect: &Rect, radii: &Radii, sh: &crate::paint::list::Shadow) {
        let s = cur.scale;
        let base = cur.rect(rect);
        let sp = sh.spread * s;
        let cast = Rect::new(
            base.x + sh.dx * s - sp,
            base.y + sh.dy * s - sp,
            base.w + 2.0 * sp,
            base.h + 2.0 * sp,
        );
        let rad = Radii(radii.0.map(|v| (v * s + sp).max(0.0)))
            .fitted(cast.w, cast.h)
            .0;
        let blur_px = (sh.blur.max(0.0) * s).min(256.0);
        let margin = ceil_i32(blur_px * 1.5) + 2;
        let area = cast.to_pixels();
        let ext = PixelRect {
            x0: area.x0 - margin,
            y0: area.y0 - margin,
            x1: area.x1 + margin,
            y1: area.y1 + margin,
        };
        let vis = ext.intersect(&cur.clip_bounds);
        if vis.is_empty() || cast.is_empty() {
            return;
        }
        let (mw, mh) = (ext.width(), ext.height());
        if i64::from(mw) * i64::from(mh) > 16_000_000 {
            return;
        }
        let mut mask = vec![0u8; (mw * mh).ix()];
        for y in 0..mh {
            for x in 0..mw {
                let c = rrect_cov(
                    (ext.x0 + x) as f32 + 0.5,
                    (ext.y0 + y) as f32 + 0.5,
                    &cast,
                    &rad,
                );
                mask[(y * mw + x).ix()] = to_u8(cov_u8(c));
            }
        }
        box_blur3(&mut mask, mw.ix(), mh.ix(), blur_px);
        let off = self.masks.len();
        self.masks.extend_from_slice(&mask);
        self.prims.push(Prim {
            kind: Kind::Mask {
                ox: ext.x0,
                oy: ext.y0,
                w: mw,
                h: mh,
                off,
                color: cur.color(sh.color),
            },
            bounds: vis,
            clip: cur.clip,
        });
    }

    fn glyphs(
        &mut self,
        cur: &Cur,
        src: &mut dyn GlyphSource,
        font: crate::text::fontdb::FontId,
        size: f32,
        color: Color,
        run: &[crate::paint::list::PosGlyph],
    ) {
        let color = cur.color(color);
        let dev_size = size * cur.scale;
        for g in run {
            let x = (g.x + cur.ox) * cur.scale;
            let y = (g.y + cur.oy) * cur.scale;
            let (xi, sub) = split_subpixel(x);
            let yi = floor_i32(y + 0.5);
            let Some(gr) = src.glyph(font, g.id, dev_size, sub) else {
                continue;
            };
            let (gw, gh) = (i32::from(gr.width), i32::from(gr.height));
            let gx = xi + i32::from(gr.left);
            let gy = yi - i32::from(gr.top);
            let bounds = PixelRect {
                x0: gx,
                y0: gy,
                x1: gx + gw,
                y1: gy + gh,
            }
            .intersect(&cur.clip_bounds);
            if bounds.is_empty() {
                continue;
            }
            let off = self.masks.len();
            for row in 0..gh.ix() {
                self.masks
                    .extend_from_slice(&gr.data[row * gr.stride..row * gr.stride + gw.ix()]);
            }
            self.prims.push(Prim {
                kind: Kind::Mask {
                    ox: gx,
                    oy: gy,
                    w: gw,
                    h: gh,
                    off,
                    color,
                },
                bounds,
                clip: cur.clip,
            });
        }
    }
}

fn union_px(a: PixelRect, b: PixelRect) -> PixelRect {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    PixelRect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

/// Three box blurs approximating a Gaussian of standard deviation `blur / 2` (the CSS rule).
fn box_blur3(mask: &mut [u8], w: usize, h: usize, blur: f32) {
    if blur <= 0.0 || w == 0 || h == 0 {
        return;
    }
    let sigma = blur * 0.5;
    // Ideal box width for n = 3 passes: sqrt(12 sigma^2 / 3 + 1).
    let wi = trunc_usize((4.0 * sigma * sigma + 1.0).sqrt().floor());
    let size = if wi.is_multiple_of(2) { wi + 1 } else { wi }.max(1);
    let r = size / 2;
    let mut tmp = vec![0u8; mask.len()];
    for _ in 0..3 {
        // Horizontal.
        for y in 0..h {
            let row = &mask[y * w..(y + 1) * w];
            let out = &mut tmp[y * w..(y + 1) * w];
            blur_line(row, out, r, 1, w);
        }
        // Vertical, column by column through a strided view.
        for x in 0..w {
            let mut col = vec![0u8; h];
            for y in 0..h {
                col[y] = tmp[y * w + x];
            }
            let mut out = vec![0u8; h];
            blur_line(&col, &mut out, r, 1, h);
            for y in 0..h {
                mask[y * w + x] = out[y];
            }
        }
    }
}

fn blur_line(src: &[u8], dst: &mut [u8], r: usize, _stride: usize, n: usize) {
    let size = 2 * r + 1;
    let mut sum: u32 = 0;
    // Window [i - r, i + r], zeros outside.
    for (k, v) in src.iter().enumerate().take(r.min(n)) {
        let _ = k;
        sum += u32::from(*v);
    }
    for i in 0..n {
        if i + r < n {
            sum += u32::from(src[i + r]);
        }
        if i > r {
            sum -= u32::from(src[i - r - 1]);
        }
        dst[i] = to_u8((sum + (to_u32(size)) / 2) / to_u32(size));
    }
}

// ---------------------------------------------------------------------------------------------
// Tile composition
// ---------------------------------------------------------------------------------------------

struct TileBuf {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    px: Vec<u8>,
}

impl TileBuf {
    fn blank(x0: i32, y0: i32, w: i32, h: i32) -> TileBuf {
        TileBuf {
            x0,
            y0,
            w,
            h,
            px: vec![0; (w * h * 4).ix()],
        }
    }
}

/// Source-over of one premultiplied source pixel with coverage `cov` (0..=255) on `dst`.
#[inline]
fn over(dst: &mut [u8], src: [u8; 4], cov: u32) {
    if cov == 0 {
        return;
    }
    let s = if cov == 255 {
        [
            u32::from(src[0]),
            u32::from(src[1]),
            u32::from(src[2]),
            u32::from(src[3]),
        ]
    } else {
        [
            div255(u32::from(src[0]) * cov),
            div255(u32::from(src[1]) * cov),
            div255(u32::from(src[2]) * cov),
            div255(u32::from(src[3]) * cov),
        ]
    };
    let inv = 255 - s[3];
    for i in 0..4 {
        dst[i] = to_u8((s[i] + div255(u32::from(dst[i]) * inv)).min(255));
    }
}

fn clip_cov(p: &Prepared, clip: u32, px: f32, py: f32) -> u32 {
    let c = &p.clips[clip.ix()];
    let mut cov = 255u32;
    for (r, rad) in &p.soft[c.soft.start.ix()..c.soft.end.ix()] {
        let k = cov_u8(rrect_cov(px, py, r, rad));
        cov = div255(cov * k);
        if cov == 0 {
            break;
        }
    }
    cov
}

fn draw_prim(buf: &mut TileBuf, p: &Prepared, prim: &Prim) {
    let area = PixelRect {
        x0: buf.x0,
        y0: buf.y0,
        x1: buf.x0 + buf.w,
        y1: buf.y0 + buf.h,
    }
    .intersect(&prim.bounds)
    .intersect(&p.clips[prim.clip.ix()].bounds);
    if area.is_empty() {
        return;
    }
    let soft = !p.clips[prim.clip.ix()].soft.is_empty()
        && p.clips[prim.clip.ix()].soft.start != p.clips[prim.clip.ix()].soft.end;
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (src, mut cov) = match &prim.kind {
                Kind::Rrect { rect, radii, fill } => {
                    let c = cov_u8(rrect_cov(fx, fy, rect, radii));
                    let col = match *fill {
                        PFill::Solid(c) => c,
                        PFill::Grad { c0, c1, p0, d, inv } => {
                            let t = (((fx - p0.0) * d.0 + (fy - p0.1) * d.1) * inv).clamp(0.0, 1.0);
                            mix(c0, c1, to_u8(cov_u8(t)))
                        }
                    };
                    (col.premul(), c)
                }
                Kind::Border {
                    outer,
                    ro,
                    inner,
                    ri,
                    color,
                } => {
                    let co = rrect_cov(fx, fy, outer, ro);
                    let ci = if inner.is_empty() {
                        0.0
                    } else {
                        rrect_cov(fx, fy, inner, ri)
                    };
                    (color.premul(), cov_u8(co * (1.0 - ci)))
                }
                Kind::Mask {
                    ox,
                    oy,
                    w,
                    h,
                    off,
                    color,
                } => {
                    let (mx, my) = (x - ox, y - oy);
                    if mx < 0 || my < 0 || mx >= *w || my >= *h {
                        continue;
                    }
                    (color.premul(), u32::from(p.masks[off + (my * w + mx).ix()]))
                }
                Kind::LayerBegin { .. } | Kind::LayerEnd { .. } => return,
            };
            if soft {
                cov = div255(cov * clip_cov(p, prim.clip, fx, fy));
            }
            let i = (((y - buf.y0) * buf.w + (x - buf.x0)) * 4).ix();
            over(&mut buf.px[i..i + 4], src, cov);
        }
    }
}

fn compose_layer(dst: &mut TileBuf, layer: &TileBuf, alpha: u8) {
    let a = u32::from(alpha);
    for i in (0..dst.px.len()).step_by(4) {
        let s = [
            layer.px[i],
            layer.px[i + 1],
            layer.px[i + 2],
            layer.px[i + 3],
        ];
        if s[3] == 0 {
            continue;
        }
        over(&mut dst.px[i..i + 4], s, a);
    }
}

/// Renders `list` into a new `width x height` image.
pub fn render(
    list: &DisplayList,
    glyphs: &mut dyn GlyphSource,
    width: u32,
    height: u32,
    opts: &RasterOptions,
) -> Image {
    let mut img = Image::new(width, height, opts.clear);
    if width == 0 || height == 0 {
        return img;
    }
    let (w, h) = (to_i32(width), to_i32(height));
    let scale = if opts.scale.is_finite() && opts.scale > 0.0 {
        opts.scale
    } else {
        1.0
    };
    let prepared = prepare(list, glyphs, w, h, scale);
    let tile = opts.tile.clamp(1, 1024);
    let tiles_x = (w + tile - 1) / tile;
    let tiles_y = (h + tile - 1) / tile;
    // Bin primitives to tiles.
    let mut bins: Vec<Vec<u32>> = vec![Vec::new(); (tiles_x * tiles_y).ix()];
    for (i, prim) in prepared.prims.iter().enumerate() {
        let b = prim.bounds.intersect(&PixelRect {
            x0: 0,
            y0: 0,
            x1: w,
            y1: h,
        });
        if b.is_empty() {
            continue;
        }
        for ty in (b.y0 / tile)..=((b.y1 - 1) / tile) {
            for tx in (b.x0 / tile)..=((b.x1 - 1) / tile) {
                bins[(ty * tiles_x + tx).ix()].push(to_u32(i));
            }
        }
    }
    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let list = &bins[(ty * tiles_x + tx).ix()];
            if list.is_empty() {
                continue;
            }
            let (x0, y0) = (tx * tile, ty * tile);
            let (tw, th) = (tile.min(w - x0), tile.min(h - y0));
            // Load the current pixels.
            let mut base = TileBuf::blank(x0, y0, tw, th);
            for row in 0..th {
                let s = (((y0 + row) * w + x0) * 4).ix();
                let d = (row * tw * 4).ix();
                base.px[d..d + (tw * 4).ix()].copy_from_slice(&img.data[s..s + (tw * 4).ix()]);
            }
            let mut stack: Vec<TileBuf> = vec![base];
            let mut k = 0;
            while k < list.len() {
                let prim = &prepared.prims[list[k].ix()];
                match &prim.kind {
                    Kind::LayerBegin { .. } => stack.push(TileBuf::blank(x0, y0, tw, th)),
                    Kind::LayerEnd { alpha } => {
                        if stack.len() > 1
                            && let Some(layer) = stack.pop()
                            && let Some(top) = stack.last_mut()
                        {
                            compose_layer(top, &layer, *alpha);
                        }
                    }
                    _ => {
                        if let Some(top) = stack.last_mut() {
                            draw_prim(top, &prepared, prim);
                        }
                    }
                }
                k += 1;
            }
            while stack.len() > 1 {
                if let Some(layer) = stack.pop()
                    && let Some(top) = stack.last_mut()
                {
                    compose_layer(top, &layer, 255);
                }
            }
            let base = &stack[0];
            for row in 0..th {
                let d = (((y0 + row) * w + x0) * 4).ix();
                let s = (row * tw * 4).ix();
                img.data[d..d + (tw * 4).ix()].copy_from_slice(&base.px[s..s + (tw * 4).ix()]);
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::glyphs::NoGlyphs;
    use crate::paint::list::Shadow;

    fn draw(list: &DisplayList, w: u32, h: u32) -> Image {
        render(
            list,
            &mut NoGlyphs,
            w,
            h,
            &RasterOptions {
                clear: Color::rgb(255, 255, 255),
                ..RasterOptions::default()
            },
        )
    }

    #[test]
    fn solid_rect_is_exact() {
        let mut l = DisplayList::new();
        l.solid(Rect::new(2.0, 2.0, 4.0, 4.0), Color::rgb(255, 0, 0));
        let img = draw(&l, 8, 8);
        assert_eq!(img.pixel(2, 2), [255, 0, 0, 255]);
        assert_eq!(img.pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(img.pixel(1, 1), [255, 255, 255, 255]);
        assert_eq!(img.pixel(6, 6), [255, 255, 255, 255]);
    }

    #[test]
    fn half_pixel_edge_blends_half() {
        let mut l = DisplayList::new();
        l.solid(Rect::new(0.5, 0.0, 2.0, 1.0), Color::rgb(0, 0, 0));
        let img = draw(&l, 4, 1);
        assert_eq!(img.pixel(0, 0), [127, 127, 127, 255]);
        assert_eq!(img.pixel(1, 0), [0, 0, 0, 255]);
        assert_eq!(img.pixel(2, 0), [127, 127, 127, 255]);
    }

    #[test]
    fn rounded_corner_cuts_the_corner_pixel() {
        let mut l = DisplayList::new();
        l.rect(
            Rect::new(0.0, 0.0, 16.0, 16.0),
            Radii::uniform(8.0),
            Fill::Solid(Color::rgb(0, 0, 0)),
        );
        let img = draw(&l, 16, 16);
        assert_eq!(img.pixel(0, 0), [255, 255, 255, 255]);
        assert_eq!(img.pixel(8, 8), [0, 0, 0, 255]);
        assert_eq!(img.pixel(8, 4), [0, 0, 0, 255]);
        assert_eq!(img.pixel(1, 8), [0, 0, 0, 255]);
    }

    #[test]
    fn tile_size_does_not_change_output() {
        let mut l = DisplayList::new();
        l.rect(
            Rect::new(3.3, 4.7, 90.2, 40.9),
            Radii::uniform(9.0),
            Fill::vertical(Color::rgb(10, 200, 30), Color::rgb(200, 30, 90)),
        );
        l.border(
            Rect::new(3.3, 4.7, 90.2, 40.9),
            Radii::uniform(9.0),
            2.0,
            Color::rgba(0, 0, 0, 160),
        );
        l.shadow(
            Rect::new(10.0, 60.0, 60.0, 30.0),
            Radii::uniform(6.0),
            Shadow::new(8.0, 0.0, 0.0, 3.0, Color::rgba(0, 0, 0, 120)),
        );
        l.push_clip(Rect::new(20.0, 10.5, 50.0, 30.0), 6.0);
        l.solid(
            Rect::new(0.0, 0.0, 128.0, 128.0),
            Color::rgba(0, 0, 255, 90),
        );
        l.pop_clip();
        let a = render(
            &l,
            &mut NoGlyphs,
            128,
            100,
            &RasterOptions {
                tile: 64,
                ..RasterOptions::default()
            },
        );
        let b = render(
            &l,
            &mut NoGlyphs,
            128,
            100,
            &RasterOptions {
                tile: 7,
                ..RasterOptions::default()
            },
        );
        let c = render(
            &l,
            &mut NoGlyphs,
            128,
            100,
            &RasterOptions {
                tile: 1024,
                ..RasterOptions::default()
            },
        );
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn opacity_pushdown_and_layer_agree_without_overlap() {
        let mk = |overlap| {
            let mut l = DisplayList::new();
            l.push_opacity(128, overlap);
            l.solid(Rect::new(0.0, 0.0, 4.0, 4.0), Color::rgb(0, 0, 0));
            l.solid(Rect::new(4.0, 0.0, 4.0, 4.0), Color::rgb(255, 0, 0));
            l.pop_opacity();
            draw(&l, 8, 4)
        };
        let a = mk(false);
        let b = mk(true);
        for i in 0..a.data.len() {
            assert!(
                (i32::from(a.data[i]) - i32::from(b.data[i])).abs() <= 1,
                "byte {i}"
            );
        }
    }

    #[test]
    fn overlapping_children_differ_between_pushdown_and_layer() {
        let mk = |overlap| {
            let mut l = DisplayList::new();
            l.push_opacity(128, overlap);
            l.solid(Rect::new(0.0, 0.0, 4.0, 4.0), Color::rgb(0, 0, 0));
            l.solid(Rect::new(0.0, 0.0, 4.0, 4.0), Color::rgb(255, 0, 0));
            l.pop_opacity();
            draw(&l, 4, 4)
        };
        // Push-down shows the black child through the red one; a layer does not.
        assert_ne!(mk(false).pixel(1, 1), mk(true).pixel(1, 1));
    }

    #[test]
    fn shadow_is_soft_and_symmetric() {
        let mut l = DisplayList::new();
        l.shadow(
            Rect::new(24.0, 24.0, 16.0, 16.0),
            Radii::uniform(4.0),
            Shadow::new(12.0, 0.0, 0.0, 0.0, Color::rgb(0, 0, 0)),
        );
        let img = draw(&l, 64, 64);
        let c = img.pixel(32, 32)[0];
        let edge = img.pixel(24, 32)[0];
        let far = img.pixel(2, 32)[0];
        assert!(c < edge && edge < far, "{c} {edge} {far}");
        assert_eq!(img.pixel(24, 32), img.pixel(39, 32));
        assert_eq!(img.pixel(32, 24), img.pixel(32, 39));
    }
}
