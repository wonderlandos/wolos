// SPDX-License-Identifier: BSD-2-Clause
//! The instance builder: display list to GPU instances and draw batches.
//!
//! One uber-pipeline draws every primitive as an instanced quad. An [`Instance`] is 64 bytes. A
//! batch (one draw call) breaks only on a blend change, a chunk transform change or an offscreen
//! layer, never on a primitive kind or a clip: clips are rows of a persistent table the shader
//! indexes, and atlas pages are a texture array. A typical window is a handful of draws.
//!
//! Chunks are cached by `(id, version, scale, atlas generation)`: a chunk whose content did not
//! change keeps its instances, and scrolling (a changed chunk *offset*) rebuilds nothing, since the
//! offset is the draw's push constant. Glyphs are the exception that keeps them exact: a glyph's
//! pixel row and quarter-pixel phase are chosen after the offset (as the CPU rasteriser does), so
//! the sub-pixel part of the device offset is folded into glyph positions and the shader moves
//! glyph quads by the whole-pixel part only. Whole-pixel scrolling still rebuilds nothing; a new
//! sub-pixel part rebuilds that chunk. The builder owns its buffers and reuses them, so a frame
//! with unchanged content allocates nothing.
//!
//! Coordinates in instances are device pixels (the display list's logical pixels times scale).

use crate::num::{floor_i32, round_u32, trunc_u32};
use crate::scene::ids::{Ix, to_u8, to_u16, to_u32};
use std::collections::HashMap;
use std::mem::size_of;

use crate::paint::geom::{Color, Rect, div255};
use crate::paint::glyphs::{GlyphSource, Place};
use crate::paint::list::{DisplayList, Fill, Item, Radii};
use crate::text::glyph::split_subpixel;

/// Primitive kinds, in the low 4 bits of `Instance::params`.
pub mod kind {
    pub const RECT: u32 = 0;
    pub const GRADIENT: u32 = 1;
    pub const BORDER: u32 = 2;
    pub const SHADOW: u32 = 3;
    pub const GLYPH: u32 = 4;
    /// After the GPU's own `K_LAYER` (5).
    pub const IMAGE: u32 = 6;
}

/// Where a picture level sits in the GPU image atlas: page and texel origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImagePlace {
    pub page: u32,
    pub x: u16,
    pub y: u16,
}

/// Places picture levels in the image atlas for the instance builder.
pub trait ImageSource {
    /// The atlas place of `level` of `px`, uploading it if needed; `None` skips the picture.
    fn place(
        &mut self,
        px: &std::sync::Arc<crate::image::Pixels>,
        level: usize,
    ) -> Option<ImagePlace>;
}

/// No atlas: pictures are skipped (lists without them, and tests of the other primitives).
pub struct NoImages;

impl ImageSource for NoImages {
    fn place(&mut self, _: &std::sync::Arc<crate::image::Pixels>, _: usize) -> Option<ImagePlace> {
        None
    }
}

/// One instanced quad, 64 bytes, `repr(C)` so it can be copied to a mapped GPU buffer as is.
///
/// * `rect`: x, y, w, h in device pixels (shadow: the casting box; the shader grows the quad).
/// * `extra`: border width / shadow blur in `extra[0]`; otherwise zero.
/// * `color0`, `color1`: straight RGBA8 (`Color::pack`); `color1` is the gradient end.
/// * `radii`: corner radii in 1/16 pixel (tl, tr, br, bl).
/// * `uv`: glyph atlas texels `(x, y, w, h)`; gradient end points as 16-bit fractions
///   `(x0, y0, x1, y1)` of the rectangle; image: the level's atlas origin and size.
/// * image: `rect` is the part drawn, `extra` the picture's origin and texels per pixel
///   `(dest x, dest y, kx, ky)`, `color0` alpha the opacity.
/// * `clip`: row of the clip table (0 = no clip).
/// * `params`: bits 0..4 kind, 4..10 atlas page.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Instance {
    pub rect: [f32; 4],
    pub extra: [f32; 4],
    pub color0: u32,
    pub color1: u32,
    pub radii: [u16; 4],
    pub uv: [u16; 4],
    pub clip: u32,
    pub params: u32,
}

const _: () = assert!(size_of::<Instance>() == 64);

impl Instance {
    pub fn kind(&self) -> u32 {
        self.params & 0xf
    }

    pub fn page(&self) -> u32 {
        (self.params >> 4) & 0x3f
    }

    /// The instance as 64 little-endian bytes (for upload on any host).
    pub fn to_bytes(&self) -> [u8; 64] {
        let mut b = [0u8; 64];
        let mut o = 0;
        let mut put = |bytes: &[u8]| {
            b[o..o + bytes.len()].copy_from_slice(bytes);
            o += bytes.len();
        };
        for v in self.rect.iter().chain(self.extra.iter()) {
            put(&v.to_le_bytes());
        }
        put(&self.color0.to_le_bytes());
        put(&self.color1.to_le_bytes());
        for v in self.radii.iter().chain(self.uv.iter()) {
            put(&v.to_le_bytes());
        }
        put(&self.clip.to_le_bytes());
        put(&self.params.to_le_bytes());
        b
    }
}

/// A row of the clip table: an axis-aligned rectangle and optionally a rounded one, both in
/// chunk-local device pixels. Only the innermost rounded clip is represented on the GPU; the CPU
/// rasteriser honours every nested one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipEntry {
    pub rect: [f32; 4],
    pub rounded: [f32; 4],
    pub radius: f32,
    pub _pad: [f32; 3],
}

const NO_CLIP: ClipEntry = ClipEntry {
    rect: [-1.0e9, -1.0e9, 2.0e9, 2.0e9],
    rounded: [0.0; 4],
    radius: 0.0,
    _pad: [0.0; 3],
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// Premultiplied source-over.
    Normal,
}

/// What a draw does besides drawing instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Draw,
    /// Start rendering into an offscreen layer.
    LayerBegin,
    /// Composite the layer back with this alpha.
    LayerEnd {
        alpha: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub first: u32,
    pub count: u32,
    /// The chunk transform, in device pixels (push constant).
    pub offset: (f32, f32),
    pub blend: Blend,
    pub pass: Pass,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchStats {
    pub instances: u32,
    /// Draw calls that draw instances (layer markers excluded).
    pub draws: u32,
    pub layers: u32,
    pub chunks_rebuilt: u32,
    pub chunks_reused: u32,
    /// Glyphs too large for the atlas; the GPU path draws them as path fills.
    pub path_glyphs: u32,
}

#[derive(Clone)]
enum Op {
    Instances(std::ops::Range<u32>),
    LayerBegin,
    LayerEnd(u8),
}

struct Cached {
    version: u64,
    scale_bits: u32,
    /// The sub-pixel part of the chunk's device offset the glyphs were placed for.
    frac_bits: (u32, u32),
    atlas_gen: u64,
    instances: Vec<Instance>,
    ops: Vec<Op>,
    path_glyphs: u32,
    touched: u64,
}

/// Maximum rows in the clip table before it is reset (which rebuilds every chunk once).
const MAX_CLIPS: usize = 4096;

#[derive(Default)]
pub struct InstanceBuilder {
    cache: HashMap<u32, Cached>,
    clip_ids: HashMap<[u32; 9], u32>,
    pub clips: Vec<ClipEntry>,
    pub instances: Vec<Instance>,
    pub draws: Vec<Draw>,
    pub stats: BatchStats,
    frame: u64,
    order: Vec<usize>,
}

fn q16(v: f32) -> u16 {
    to_u16(trunc_u32((v * 16.0).round().clamp(0.0, 65535.0)))
}

fn frac16(v: f32) -> u16 {
    to_u16(round_u32(v.clamp(0.0, 1.0) * 65535.0))
}

fn clip_key(c: &ClipEntry) -> [u32; 9] {
    [
        c.rect[0].to_bits(),
        c.rect[1].to_bits(),
        c.rect[2].to_bits(),
        c.rect[3].to_bits(),
        c.rounded[0].to_bits(),
        c.rounded[1].to_bits(),
        c.rounded[2].to_bits(),
        c.rounded[3].to_bits(),
        c.radius.to_bits(),
    ]
}

impl InstanceBuilder {
    pub fn new() -> InstanceBuilder {
        let mut b = InstanceBuilder::default();
        b.reset_clips();
        b
    }

    fn reset_clips(&mut self) {
        self.clips.clear();
        self.clip_ids.clear();
        self.clips.push(NO_CLIP);
        self.clip_ids.insert(clip_key(&NO_CLIP), 0);
        self.cache.clear();
    }

    fn intern(&mut self, c: ClipEntry) -> u32 {
        if let Some(&id) = self.clip_ids.get(&clip_key(&c)) {
            return id;
        }
        let id = to_u32(self.clips.len());
        self.clips.push(c);
        self.clip_ids.insert(clip_key(&c), id);
        id
    }

    /// Builds the frame: `self.instances`, `self.draws`, `self.clips` and `self.stats`.
    ///
    /// `atlas_gen` is any counter that changes whenever glyph images may have moved (the atlas
    /// eviction count); a change rebuilds the chunks that hold glyphs.
    pub fn build(
        &mut self,
        list: &DisplayList,
        glyphs: &mut dyn GlyphSource,
        scale: f32,
        atlas_gen: u64,
    ) {
        self.build_with_images(list, glyphs, &mut NoImages, scale, atlas_gen);
    }

    /// [`InstanceBuilder::build`] with pictures placed through `images`. `atlas_gen` must also
    /// change whenever placed pictures may have moved.
    pub fn build_with_images(
        &mut self,
        list: &DisplayList,
        glyphs: &mut dyn GlyphSource,
        images: &mut dyn ImageSource,
        scale: f32,
        atlas_gen: u64,
    ) {
        if self.clips.is_empty() || self.clips.len() > MAX_CLIPS {
            self.reset_clips();
        }
        self.frame += 1;
        self.instances.clear();
        self.draws.clear();
        self.stats = BatchStats::default();
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let mut order = std::mem::take(&mut self.order);
        list.draw_order_into(&mut order);
        for &ci in &order {
            let chunk = &list.chunks[ci];
            let frac = offset_frac((chunk.offset.0 * scale, chunk.offset.1 * scale));
            let fresh = match self.cache.get_mut(&chunk.id) {
                Some(c)
                    if c.version == chunk.version
                        && c.scale_bits == scale.to_bits()
                        && c.frac_bits == (frac.0.to_bits(), frac.1.to_bits())
                        && c.atlas_gen == atlas_gen =>
                {
                    c.touched = self.frame;
                    false
                }
                _ => true,
            };
            if fresh {
                let built = self.build_chunk(list, ci, (glyphs, images), (scale, frac), atlas_gen);
                self.cache.insert(chunk.id, built);
                self.stats.chunks_rebuilt += 1;
            } else {
                self.stats.chunks_reused += 1;
            }
            let offset = (chunk.offset.0 * scale, chunk.offset.1 * scale);
            let cached = &self.cache[&chunk.id];
            self.stats.path_glyphs += cached.path_glyphs;
            for op in &cached.ops {
                match op {
                    Op::Instances(r) => {
                        let first = to_u32(self.instances.len());
                        self.instances
                            .extend_from_slice(&cached.instances[r.start.ix()..r.end.ix()]);
                        let count = r.end - r.start;
                        // Merge with the previous draw when nothing forces a break.
                        match self.draws.last_mut() {
                            Some(d)
                                if d.pass == Pass::Draw
                                    && d.offset == offset
                                    && d.first + d.count == first =>
                            {
                                d.count += count;
                            }
                            _ => self.draws.push(Draw {
                                first,
                                count,
                                offset,
                                blend: Blend::Normal,
                                pass: Pass::Draw,
                            }),
                        }
                    }
                    Op::LayerBegin => {
                        let at = to_u32(self.instances.len());
                        self.draws.push(Draw {
                            first: at,
                            count: 0,
                            offset,
                            blend: Blend::Normal,
                            pass: Pass::LayerBegin,
                        });
                        self.stats.layers += 1;
                    }
                    Op::LayerEnd(a) => {
                        let at = to_u32(self.instances.len());
                        self.draws.push(Draw {
                            first: at,
                            count: 0,
                            offset,
                            blend: Blend::Normal,
                            pass: Pass::LayerEnd { alpha: *a },
                        });
                    }
                }
            }
        }
        self.order = order;
        self.stats.instances = to_u32(self.instances.len());
        self.stats.draws = to_u32(self.draws.iter().filter(|d| d.pass == Pass::Draw).count());
        // Forget chunks that were not part of this frame.
        if self.cache.len() > list.chunks.len() {
            let f = self.frame;
            self.cache.retain(|_, c| c.touched == f);
        }
    }

    fn build_chunk(
        &mut self,
        list: &DisplayList,
        ci: usize,
        (glyphs, images): (&mut dyn GlyphSource, &mut dyn ImageSource),
        (scale, frac): (f32, (f32, f32)),
        atlas_gen: u64,
    ) -> Cached {
        let chunk = &list.chunks[ci];
        // A chunk clips its content to the whole pixels its bounds touch on screen (the CPU
        // rasteriser's `to_pixels`), so the base clip row is snapped to that grid: in chunk-local
        // space the grid is shifted by the sub-pixel part of the offset.
        let snap = |lo: f32, len: f32, f: f32| {
            let a = (lo * scale + f).floor() - f;
            let b = ((lo + len) * scale + f).ceil() - f;
            (a, (b - a).max(0.0))
        };
        let (bx, bw) = snap(chunk.bounds.x, chunk.bounds.w, frac.0);
        let (by, bh) = snap(chunk.bounds.y, chunk.bounds.h, frac.1);
        let chunk_bounds = Rect::new(bx, by, bw, bh);
        let base_clip = if chunk.bounds.w > 1.0e5 {
            0
        } else {
            self.intern(ClipEntry {
                rect: [
                    chunk_bounds.x,
                    chunk_bounds.y,
                    chunk_bounds.w,
                    chunk_bounds.h,
                ],
                rounded: [0.0; 4],
                radius: 0.0,
                _pad: [0.0; 3],
            })
        };
        let base_entry = if base_clip == 0 {
            NO_CLIP
        } else {
            self.clips[base_clip.ix()]
        };
        let mut b = ChunkBuild {
            out: Cached {
                version: chunk.version,
                scale_bits: scale.to_bits(),
                frac_bits: (frac.0.to_bits(), frac.1.to_bits()),
                atlas_gen,
                instances: Vec::new(),
                ops: Vec::new(),
                path_glyphs: 0,
                touched: self.frame,
            },
            scale,
            frac,
            clip_stack: vec![(base_clip, base_entry)],
            opacity: 255,
            opacity_stack: Vec::new(),
            run_start: 0,
        };
        for item in list.chunk_items(chunk) {
            match item {
                Item::PushClip { .. } | Item::PopClip => b.clip(self, item),
                Item::PushOpacity { .. } | Item::PopOpacity => b.layer(item),
                Item::Rect { .. } | Item::Border { .. } | Item::Shadow { .. } => b.shape(item),
                Item::Glyphs { .. } => b.glyphs(list, glyphs, item),
                Item::Image { .. } => b.image(images, item),
            }
        }
        b.flush();
        // An unbalanced list must not leave a layer open on the GPU.
        while let Some((_, layer, alpha)) = b.opacity_stack.pop() {
            if layer {
                b.out.ops.push(Op::LayerEnd(alpha));
            }
        }
        b.out
    }
}

/// The state of one chunk being turned into instances.
struct ChunkBuild {
    out: Cached,
    scale: f32,
    /// Sub-pixel part of the chunk's device offset, added to glyph positions before rounding.
    frac: (f32, f32),
    clip_stack: Vec<(u32, ClipEntry)>,
    opacity: u32,
    /// (opacity before, whether it opened a layer, alpha).
    opacity_stack: Vec<(u32, bool, u8)>,
    run_start: u32,
}

impl ChunkBuild {
    /// End the current run of instances as one op.
    fn flush(&mut self) {
        let n = to_u32(self.out.instances.len());
        if n > self.run_start {
            self.out.ops.push(Op::Instances(self.run_start..n));
        }
        self.run_start = n;
    }

    fn clip_id(&self) -> u32 {
        self.clip_stack.last().map_or(0, |c| c.0)
    }

    fn scaled(&self, r: &Rect) -> Rect {
        let s = self.scale;
        Rect::new(r.x * s, r.y * s, r.w * s, r.h * s)
    }

    fn clip(&mut self, b: &mut InstanceBuilder, item: &Item) {
        match item {
            Item::PushClip { rect, radius } => {
                let clip_entry = self.clip_stack.last().map_or(NO_CLIP, |c| c.1);
                let r = self.scaled(rect);
                let mut e = clip_entry;
                let cur = PixelRectF::from(e.rect);
                let inter = cur.intersect(&r);
                e.rect = [inter.x, inter.y, inter.w, inter.h];
                if *radius > 0.0 {
                    e.rounded = [r.x, r.y, r.w, r.h];
                    e.radius = (radius * self.scale).min(r.w.min(r.h) * 0.5);
                }
                let id = b.intern(e);
                self.clip_stack.push((id, e));
            }
            Item::PopClip if self.clip_stack.len() > 1 => {
                self.clip_stack.pop();
            }
            _ => {}
        }
    }

    fn layer(&mut self, item: &Item) {
        match item {
            Item::PushOpacity { alpha, overlap } => {
                if *overlap {
                    self.flush();
                    self.out.ops.push(Op::LayerBegin);
                    self.opacity_stack.push((self.opacity, true, *alpha));
                } else {
                    self.opacity_stack.push((self.opacity, false, *alpha));
                    self.opacity = div255(self.opacity * u32::from(*alpha));
                }
            }
            Item::PopOpacity => {
                if let Some((prev, layer, alpha)) = self.opacity_stack.pop() {
                    if layer {
                        self.flush();
                        self.out.ops.push(Op::LayerEnd(alpha));
                    }
                    self.opacity = prev;
                }
            }
            _ => {}
        }
    }

    /// Rectangles, borders and shadows: one instance each.
    fn shape(&mut self, item: &Item) {
        let (scale, clip_id) = (self.scale, self.clip_id());
        let alpha = to_u8(self.opacity);
        match item {
            Item::Rect { rect, radii, fill } => {
                let r = self.scaled(rect);
                if r.is_empty() {
                    return;
                }
                let mut inst = Instance {
                    rect: [r.x, r.y, r.w, r.h],
                    radii: radii_q(radii, scale, r.w, r.h),
                    clip: clip_id,
                    ..Instance::default()
                };
                match *fill {
                    Fill::Solid(c) => {
                        inst.color0 = c.with_opacity(alpha).pack();
                        inst.params = kind::RECT;
                    }
                    Fill::Linear { from, to, p0, p1 } => {
                        inst.color0 = from.with_opacity(alpha).pack();
                        inst.color1 = to.with_opacity(alpha).pack();
                        inst.uv = [frac16(p0.0), frac16(p0.1), frac16(p1.0), frac16(p1.1)];
                        inst.params = kind::GRADIENT;
                    }
                }
                self.out.instances.push(inst);
            }
            Item::Border {
                rect,
                radii,
                width,
                color,
            } => {
                let r = self.scaled(rect);
                if r.is_empty() || *width <= 0.0 {
                    return;
                }
                self.out.instances.push(Instance {
                    rect: [r.x, r.y, r.w, r.h],
                    extra: [width * scale, 0.0, 0.0, 0.0],
                    color0: color.with_opacity(alpha).pack(),
                    radii: radii_q(radii, scale, r.w, r.h),
                    clip: clip_id,
                    params: kind::BORDER,
                    ..Instance::default()
                });
            }
            Item::Shadow {
                rect,
                radii,
                blur,
                spread,
                dx,
                dy,
                color,
            } => {
                let sp = spread * scale;
                let r = Rect::new(
                    rect.x * scale + dx * scale - sp,
                    rect.y * scale + dy * scale - sp,
                    rect.w * scale + 2.0 * sp,
                    rect.h * scale + 2.0 * sp,
                );
                if r.is_empty() {
                    return;
                }
                let rr = Radii(radii.0.map(|v| (v * scale + sp).max(0.0))).fitted(r.w, r.h);
                self.out.instances.push(Instance {
                    rect: [r.x, r.y, r.w, r.h],
                    extra: [blur.max(0.0) * scale, 0.0, 0.0, 0.0],
                    color0: color.with_opacity(alpha).pack(),
                    radii: rr.0.map(q16),
                    clip: clip_id,
                    params: kind::SHADOW,
                    ..Instance::default()
                });
            }
            _ => {}
        }
    }

    /// A picture: one instance sampling its level from the image atlas.
    fn image(&mut self, images: &mut dyn ImageSource, item: &Item) {
        let Item::Image {
            rect,
            dest,
            image,
            alpha,
        } = item
        else {
            return;
        };
        let (r, d) = (self.scaled(rect), self.scaled(dest));
        if r.is_empty() || d.is_empty() {
            return;
        }
        let level = image.0.level_for(d.w, d.h);
        let Some(l) = image.0.levels.get(level) else {
            return;
        };
        let Some(at) = images.place(&image.0, level) else {
            return;
        };
        let a = div255(self.opacity * u32::from(*alpha));
        self.out.instances.push(Instance {
            rect: [r.x, r.y, r.w, r.h],
            extra: [d.x, d.y, l.w as f32 / d.w, l.h as f32 / d.h],
            color0: a << 24,
            uv: [at.x, at.y, to_u16(l.w), to_u16(l.h)],
            clip: self.clip_id(),
            params: kind::IMAGE | (at.page.min(63) << 4),
            ..Instance::default()
        });
    }

    /// A run of glyphs: atlas glyphs become instances, the rest are counted for the path pass.
    fn glyphs(&mut self, list: &DisplayList, glyphs: &mut dyn GlyphSource, item: &Item) {
        let Item::Glyphs {
            font,
            size,
            color,
            glyphs: range,
        } = item
        else {
            return;
        };
        let (scale, clip_id) = (self.scale, self.clip_id());
        let color: Color = color.with_opacity(to_u8(self.opacity));
        for g in list.glyph_slice(range) {
            let x = g.x * scale + self.frac.0;
            let y = g.y * scale + self.frac.1;
            let (xi, sub) = split_subpixel(x);
            let yi = floor_i32(y + 0.5);
            let Some(gr) = glyphs.glyph(*font, g.id, size * scale, sub) else {
                continue;
            };
            match gr.place {
                Place::Atlas { page, x: ux, y: uy } => {
                    self.out.instances.push(Instance {
                        rect: [
                            (xi + i32::from(gr.left)) as f32,
                            (yi - i32::from(gr.top)) as f32,
                            f32::from(gr.width),
                            f32::from(gr.height),
                        ],
                        color0: color.pack(),
                        uv: [ux, uy, gr.width, gr.height],
                        clip: clip_id,
                        params: kind::GLYPH | (u32::from(page.min(63)) << 4),
                        ..Instance::default()
                    });
                }
                Place::Transient => self.out.path_glyphs += 1,
            }
        }
    }
}

/// The sub-pixel part (`0 <= f < 1`) of a device offset; the shader moves glyphs by the rest.
pub fn offset_frac(o: (f32, f32)) -> (f32, f32) {
    let f = |v: f32| if v.is_finite() { v - v.floor() } else { 0.0 };
    (f(o.0), f(o.1))
}

fn radii_q(r: &Radii, scale: f32, w: f32, h: f32) -> [u16; 4] {
    Radii(r.0.map(|v| v * scale)).fitted(w, h).0.map(q16)
}

/// A float rectangle with the intersect used for clip rows.
#[derive(Clone, Copy)]
struct PixelRectF {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl From<[f32; 4]> for PixelRectF {
    fn from(a: [f32; 4]) -> Self {
        PixelRectF {
            x: a[0],
            y: a[1],
            w: a[2],
            h: a[3],
        }
    }
}

impl PixelRectF {
    fn intersect(&self, o: &Rect) -> PixelRectF {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        PixelRectF {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0.0),
            h: (y1 - y0).max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::glyphs::NoGlyphs;
    use crate::paint::list::Shadow;

    #[test]
    fn instance_is_64_bytes_and_serialises() {
        assert_eq!(size_of::<Instance>(), 64);
        let i = Instance {
            rect: [1.0, 2.0, 3.0, 4.0],
            clip: 9,
            params: kind::BORDER,
            ..Instance::default()
        };
        let b = i.to_bytes();
        assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), 1.0);
        assert_eq!(u32::from_le_bytes(b[56..60].try_into().unwrap()), 9);
        assert_eq!(
            u32::from_le_bytes(b[60..64].try_into().unwrap()),
            kind::BORDER
        );
    }

    fn sample(offset_y: f32, version: u64) -> DisplayList {
        let mut l = DisplayList::new();
        l.begin_chunk(1, 1, Rect::new(0.0, 0.0, 400.0, 300.0), (0.0, 0.0), 0);
        l.rect(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            Radii::ZERO,
            Fill::Solid(Color::rgb(250, 250, 252)),
        );
        l.push_clip(Rect::new(10.0, 10.0, 200.0, 200.0), 8.0);
        for i in 0..20 {
            l.rect(
                Rect::new(10.0, 10.0 + i as f32 * 10.0, 200.0, 8.0),
                Radii::uniform(2.0),
                Fill::vertical(Color::rgb(200, 210, 255), Color::rgb(180, 190, 240)),
            );
        }
        l.pop_clip();
        l.shadow(
            Rect::new(220.0, 20.0, 100.0, 60.0),
            Radii::uniform(8.0),
            Shadow::new(12.0, 0.0, 0.0, 4.0, Color::rgba(0, 0, 0, 90)),
        );
        l.border(
            Rect::new(220.0, 20.0, 100.0, 60.0),
            Radii::uniform(8.0),
            1.0,
            Color::rgb(120, 120, 140),
        );
        l.begin_chunk(
            2,
            version,
            Rect::new(0.0, 0.0, 400.0, 600.0),
            (0.0, offset_y),
            1,
        );
        l.rect(
            Rect::new(0.0, 0.0, 100.0, 40.0),
            Radii::uniform(4.0),
            Fill::Solid(Color::rgb(90, 120, 255)),
        );
        l.finish();
        l
    }

    #[test]
    fn typical_window_needs_few_draws() {
        let mut b = InstanceBuilder::new();
        b.build(&sample(0.0, 1), &mut NoGlyphs, 1.0, 0);
        assert!(b.stats.draws <= 10, "{:?}", b.stats);
        assert_eq!(
            b.stats.draws, 1,
            "chunks with the same transform share a draw: {:?}",
            b.stats
        );
        assert_eq!(b.stats.instances, 1 + 20 + 1 + 1 + 1);
        b.build(&sample(30.0, 1), &mut NoGlyphs, 1.0, 0);
        assert_eq!(
            b.stats.draws, 2,
            "a scrolled chunk draws with its own transform: {:?}",
            b.stats
        );
    }

    #[test]
    fn scrolling_rebuilds_nothing() {
        let mut b = InstanceBuilder::new();
        b.build(&sample(0.0, 1), &mut NoGlyphs, 1.0, 0);
        let first = b.instances.clone();
        b.build(&sample(37.0, 1), &mut NoGlyphs, 1.0, 0);
        assert_eq!(b.stats.chunks_rebuilt, 0);
        assert_eq!(b.stats.chunks_reused, 2);
        assert_eq!(b.instances, first);
        assert_eq!(b.draws.last().unwrap().offset, (0.0, 37.0));
    }

    #[test]
    fn glyphs_follow_the_sub_pixel_offset_and_whole_pixels_rebuild_nothing() {
        use crate::paint::glyphs::{GlyphRef, Place};
        use crate::paint::list::PosGlyph;
        use crate::text::fontdb::FontId;
        struct One([u8; 4], Vec<u8>);
        impl GlyphSource for One {
            fn glyph(&mut self, _: FontId, _: u32, _: f32, sub: u8) -> Option<GlyphRef<'_>> {
                self.1.push(sub);
                Some(GlyphRef {
                    width: 2,
                    height: 2,
                    left: 0,
                    top: 2,
                    stride: 2,
                    data: &self.0,
                    place: Place::Atlas {
                        page: 0,
                        x: 0,
                        y: 0,
                    },
                })
            }
        }
        let mk = |oy: f32, ox: f32| {
            let mut l = DisplayList::new();
            l.begin_chunk(7, 1, Rect::new(0.0, 0.0, 100.0, 100.0), (ox, oy), 0);
            l.glyph_run(
                FontId(0),
                12.0,
                Color::rgb(0, 0, 0),
                [PosGlyph {
                    id: 5,
                    x: 10.0,
                    y: 20.2,
                }],
            );
            l.finish();
            l
        };
        let mut src = One([255; 4], Vec::new());
        let mut b = InstanceBuilder::new();
        b.build(&mk(2.5, 0.0), &mut src, 1.0, 0);
        // CPU: baseline row floor(20.2 + 2.5 + 0.5) = 23; GPU: local row + floor(2.5).
        let local = b.instances[0].rect[1] + 2.0 + 2.0;
        assert_eq!(local, 23.0, "row chosen after the offset");
        b.build(&mk(3.5, 0.0), &mut src, 1.0, 0);
        assert_eq!(
            b.stats.chunks_rebuilt, 0,
            "same sub-pixel part: whole-pixel scroll is free"
        );
        b.build(&mk(3.75, 0.25), &mut src, 1.0, 0);
        assert_eq!(
            b.stats.chunks_rebuilt, 1,
            "a new sub-pixel part places glyphs again"
        );
        assert_eq!(
            src.1.last().copied(),
            Some(1),
            "quarter-pixel phase of x = 10.25"
        );
        assert_eq!(offset_frac((-0.25, 7.0)), (0.75, 0.0));
    }

    #[test]
    fn version_bump_rebuilds_only_that_chunk() {
        let mut b = InstanceBuilder::new();
        b.build(&sample(0.0, 1), &mut NoGlyphs, 1.0, 0);
        b.build(&sample(0.0, 2), &mut NoGlyphs, 1.0, 0);
        assert_eq!(b.stats.chunks_rebuilt, 1);
        assert_eq!(b.stats.chunks_reused, 1);
    }

    #[test]
    fn same_clip_is_one_row() {
        let mut b = InstanceBuilder::new();
        b.build(&sample(0.0, 1), &mut NoGlyphs, 1.0, 0);
        let n = b.clips.len();
        b.build(&sample(0.0, 2), &mut NoGlyphs, 1.0, 0);
        assert_eq!(b.clips.len(), n);
    }

    #[test]
    fn overlap_layers_break_batches() {
        let mut l = DisplayList::new();
        l.solid(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1, 1, 1));
        l.push_opacity(128, true);
        l.solid(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(2, 2, 2));
        l.solid(Rect::new(5.0, 5.0, 10.0, 10.0), Color::rgb(3, 3, 3));
        l.pop_opacity();
        l.solid(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(4, 4, 4));
        l.finish();
        let mut b = InstanceBuilder::new();
        b.build(&l, &mut NoGlyphs, 1.0, 0);
        assert_eq!(b.stats.layers, 1);
        assert_eq!(b.stats.draws, 3);
        assert!(matches!(b.draws[1].pass, Pass::LayerBegin));
        assert!(matches!(b.draws[3].pass, Pass::LayerEnd { alpha: 128 }));
    }

    #[test]
    fn pushed_down_opacity_folds_into_colour() {
        let mut l = DisplayList::new();
        l.push_opacity(128, false);
        l.solid(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(9, 9, 9));
        l.pop_opacity();
        l.finish();
        let mut b = InstanceBuilder::new();
        b.build(&l, &mut NoGlyphs, 1.0, 0);
        assert_eq!(Color::unpack(b.instances[0].color0).a, 128);
        assert_eq!(b.stats.draws, 1);
    }
}
