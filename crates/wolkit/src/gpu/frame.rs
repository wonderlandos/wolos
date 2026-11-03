// SPDX-License-Identifier: BSD-2-Clause
//! The GPU renderer: a display list in, pixels on a target out (plan section 2.6).
//!
//! Per frame: the instance builder (`paint::batch`) turns the list into 64-byte instances and a
//! handful of draws (chunks that did not change reuse their instances); the renderer writes them
//! into the persistent ring of the frame's slot, uploads only the dirty rectangles of the glyph
//! and shadow atlases, and records one render pass over the damaged rectangle. Group opacity is
//! already pushed into the instances' colours; only overlapping groups get an offscreen layer.
//!
//! Device loss: a lost device drops every Vulkan object ([`Live`]) and the call returns
//! `WOLKIT_ERR_DEVICE_LOST`; the caller draws that frame on the CPU rasteriser. The next frame
//! makes a new device. After three losses in a minute the renderer stops trying for the life of
//! the process. Everything on the CPU side (instances, shadow prototypes, glyph pages) survives a
//! loss and is uploaded again in full.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ash::vk;

use super::device::{DeviceOptions, Gpu, barrier, vk_err};
use super::live::{Live, SLOTS};
use super::pipelines::MAX_LAYERS;
use super::shadow::ShadowAtlas;
use super::{ABSOLUTE, K_LAYER, copy};
use crate::cpu::raster::Image;
use crate::error::{Error, Result, Status};
use crate::paint::batch::{Draw, Instance, InstanceBuilder, Pass, kind};
use crate::paint::damage::Damage;
use crate::paint::geom::{Color, PixelRect, Rect};
use crate::paint::glyphs::{GlyphSource, NoGlyphs};
use crate::paint::list::DisplayList;
use crate::scene::ids::Ix;
use crate::text::fontdb::FontDb;
use crate::text::glyphcache::{GlyphCache, Glyphs};

/// Losses within [`LOSS_WINDOW`] after which the renderer gives up.
pub const MAX_LOSSES: usize = 3;
pub const LOSS_WINDOW: Duration = Duration::from_secs(60);

/// Colour formats a target can have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Headless and readback (the CPU rasteriser's byte order).
    Rgba8,
    /// What Wayland compositors offer first for swapchains.
    Bgra8,
}

impl Format {
    pub fn vk(self) -> vk::Format {
        match self {
            Format::Rgba8 => vk::Format::R8G8B8A8_UNORM,
            Format::Bgra8 => vk::Format::B8G8R8A8_UNORM,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RendererOptions {
    pub device: DeviceOptions,
    /// Pipeline cache file (the engine passes `$XDG_CACHE_HOME/wolkit/pipeline-cache`).
    pub cache_file: Option<PathBuf>,
    pub format: Format,
}

impl Default for RendererOptions {
    fn default() -> Self {
        RendererOptions {
            device: DeviceOptions::default(),
            cache_file: None,
            format: Format::Rgba8,
        }
    }
}

/// One frame's input.
pub struct Frame<'a> {
    pub list: &'a DisplayList,
    /// Fonts and the glyph cache, when the list has text.
    pub text: Option<(&'a FontDb, &'a mut GlyphCache)>,
    /// Device pixels.
    pub width: u32,
    pub height: u32,
    /// Device pixels per logical pixel.
    pub scale: f32,
    pub clear: Color,
    /// What changed since the last frame, in logical pixels; `None` redraws everything.
    pub damage: Option<&'a Damage>,
}

/// What the last frame did (frame ring `draws`, `damage_px`; budgets 2, 9 and 10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// `vkCmdDraw` calls (instance batches plus layer composites).
    pub draws: u32,
    pub instances: u32,
    pub layers: u32,
    /// Overlapping groups nested deeper than the GPU keeps layers for (drawn without a layer).
    pub layers_folded: u32,
    pub upload_bytes: u64,
    /// Pixels inside the scissor this frame (0 when nothing changed).
    pub damage_px: u64,
    /// The whole target was drawn.
    pub full: bool,
    /// Wall time from the call to the submit, nanoseconds (includes `wait_ns`).
    pub record_ns: u64,
    /// Part of `record_ns` spent waiting for the frame slot's fence (the GPU three frames behind;
    /// on a software device this is the driver's rasterisation).
    pub wait_ns: u64,
    pub chunks_rebuilt: u32,
    pub chunks_reused: u32,
}

/// The rectangle a frame redraws, in device pixels, clipped to the target.
pub fn scissor_for(damage: Option<&Damage>, scale: f32, w: u32, h: u32, fresh: bool) -> PixelRect {
    let full = PixelRect {
        x0: 0,
        y0: 0,
        x1: i32::try_from(w).unwrap_or(i32::MAX),
        y1: i32::try_from(h).unwrap_or(i32::MAX),
    };
    match damage {
        Some(d) if !fresh => {
            if d.is_empty() {
                return PixelRect::default();
            }
            let b = d.bounds();
            Rect::new(b.x * scale, b.y * scale, b.w * scale, b.h * scale)
                .to_pixels()
                .intersect(&full)
        }
        _ => full,
    }
}

pub struct Renderer {
    opts: RendererOptions,
    live: Option<Live>,
    losses: Vec<Instant>,
    builder: InstanceBuilder,
    shadows: ShadowAtlas,
    instances: Vec<Instance>,
    bytes: Vec<u8>,
    frame: u64,
    simulate_loss: bool,
    pub stats: FrameStats,
    /// Devices made over the renderer's life (1 plus recoveries).
    pub devices_made: u32,
}

impl Renderer {
    /// Opens a device now (so a missing GPU is known before the first frame).
    pub fn new(opts: RendererOptions) -> Result<Renderer> {
        let mut r = Renderer {
            opts,
            live: None,
            losses: Vec::new(),
            builder: InstanceBuilder::new(),
            shadows: ShadowAtlas::new(super::SHADOW_ATLAS),
            instances: Vec::new(),
            bytes: Vec::new(),
            frame: 0,
            simulate_loss: false,
            stats: FrameStats::default(),
            devices_made: 0,
        };
        r.ensure_live()?;
        Ok(r)
    }

    /// The colour format of the target.
    pub fn format(&self) -> Format {
        self.opts.format
    }

    /// The device facts (name, vendor, kind), when a device is open.
    pub fn gpu(&self) -> Option<&Arc<Gpu>> {
        self.live.as_ref().map(|l| &l.gpu)
    }

    /// The pipeline cache came from disk.
    pub fn pipeline_cache_hit(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.pipes.cache_hit)
    }

    /// Writes the pipeline cache file (the engine calls this at idle and on exit). A blob whose
    /// header does not describe this device is not written (some drivers return a truncated one;
    /// it could never be used and would be refused on the next start anyway). Returns whether
    /// a file was written.
    pub fn save_pipeline_cache(&self) -> Result<bool> {
        let (Some(l), Some(path)) = (self.live.as_ref(), self.opts.cache_file.as_deref()) else {
            return Ok(false);
        };
        let data = l.pipes.cache_data(&l.gpu);
        if !super::pipelines::cache_header_matches(&data, &l.gpu.props) {
            return Ok(false);
        }
        super::pipelines::write_cache(path, &data)
            .map(|()| true)
            .map_err(|_| Error::new(Status::Internal, copy::DRIVER))
    }

    /// Makes the next submit act as if the device was lost (tests of the recovery path).
    pub fn simulate_device_loss(&mut self) {
        self.simulate_loss = true;
    }

    /// The renderer gave up on the GPU for good (three losses in a minute).
    pub fn gave_up(&self) -> bool {
        let now = Instant::now();
        self.losses
            .iter()
            .filter(|t| now.duration_since(**t) < LOSS_WINDOW)
            .count()
            >= MAX_LOSSES
    }

    /// The next list is not a successor of the last one (another window, or a list whose chunk
    /// ids and versions start over): every chunk is rebuilt and the whole target redrawn.
    pub fn invalidate(&mut self) {
        self.builder = InstanceBuilder::new();
        if let Some(t) = self.live.as_mut().and_then(|l| l.target.as_mut()) {
            t.fresh = true;
        }
    }

    /// Drops GPU textures and the CPU caches (Memory Saver, ADR 0026).
    pub fn trim(&mut self) {
        self.builder = InstanceBuilder::new();
        self.shadows.trim();
        self.instances = Vec::new();
        self.bytes = Vec::new();
        if let Some(l) = self.live.as_mut() {
            l.shadow_gen = 0;
        }
    }

    fn ensure_live(&mut self) -> Result<()> {
        if self.live.is_some() {
            return Ok(());
        }
        if self.gave_up() {
            return Err(Error::unavailable(copy::GAVE_UP));
        }
        let gpu = Arc::new(Gpu::new(&self.opts.device)?);
        let live = Live::new(gpu, self.opts.format.vk(), self.opts.cache_file.as_deref())?;
        self.devices_made += 1;
        self.live = Some(live);
        Ok(())
    }

    fn lose(&mut self) {
        self.live = None;
        self.losses.push(Instant::now());
        if self.losses.len() > 8 {
            self.losses.remove(0);
        }
    }

    /// Runs `f`; a lost device is dropped so the next frame recovers.
    fn guarded<T>(&mut self, f: impl FnOnce(&mut Renderer) -> Result<T>) -> Result<T> {
        let r = f(self);
        if let Err(e) = &r
            && e.status == Status::DeviceLost
        {
            self.lose();
        }
        r
    }

    /// Draws a frame and reads it back (headless rendering and the goldens).
    pub fn render_image(&mut self, frame: Frame<'_>) -> Result<Image> {
        self.guarded(|r| {
            let (w, h) = (frame.width, frame.height);
            let slot = r.record(frame)?;
            r.finish_readback(slot, w, h)
        })
    }

    /// Draws a frame into the offscreen target and submits it without waiting (an offscreen
    /// window, and the benches: the time to here is the engine side of budget 1).
    pub fn render_submit(&mut self, frame: Frame<'_>) -> Result<()> {
        self.guarded(|r| {
            let slot = r.record(frame)?;
            if r.take_simulated_loss() {
                return Err(vk_err(vk::Result::ERROR_DEVICE_LOST));
            }
            match r.live.as_mut() {
                Some(l) => l.submit(slot, &[], &[], false),
                None => Err(Error::unavailable(copy::NO_DEVICE)),
            }
        })
    }

    /// Waits for every submitted frame (benches and orderly shutdown).
    pub fn wait_idle(&self) -> Result<()> {
        self.live.as_ref().map_or(Ok(()), |l| l.gpu.idle())
    }

    /// Draws a frame into the target and submits it without waiting; the caller presents.
    pub(crate) fn render_target(&mut self, frame: Frame<'_>) -> Result<(usize, PixelRect)> {
        self.guarded(|r| {
            let scale = frame.scale;
            let damage = frame.damage.cloned();
            let (w, h) = (frame.width, frame.height);
            let fresh = r
                .live
                .as_ref()
                .and_then(|l| l.target.as_ref())
                .is_none_or(|t| t.fresh);
            let slot = r.record(frame)?;
            Ok((slot, scissor_for(damage.as_ref(), scale, w, h, fresh)))
        })
    }

    pub(crate) fn live_mut(&mut self) -> Option<&mut Live> {
        self.live.as_mut()
    }

    pub(crate) fn take_simulated_loss(&mut self) -> bool {
        std::mem::take(&mut self.simulate_loss)
    }

    pub(crate) fn lose_now(&mut self) {
        self.lose();
    }

    /// Builds instances, uploads, and records the frame's passes into a slot; returns it.
    fn record(&mut self, mut frame: Frame<'_>) -> Result<usize> {
        let t0 = Instant::now();
        self.ensure_live()?;
        let slot = self.frame.ix() % SLOTS;
        self.frame += 1;
        let atlas_gen = frame
            .text
            .as_ref()
            .map_or(0, |(_, c)| c.atlas.stats.evictions);
        {
            let mut none = NoGlyphs;
            let mut with;
            let src: &mut dyn GlyphSource = match frame.text.as_mut() {
                Some((db, cache)) => {
                    with = Glyphs { db, cache };
                    &mut with
                }
                None => &mut none,
            };
            self.builder.build(frame.list, src, frame.scale, atlas_gen);
        }
        let Some(live) = self.live.as_mut() else {
            return Err(Error::unavailable(copy::NO_DEVICE));
        };
        let tw = Instant::now();
        live.wait(slot)?;
        let wait_ns = u64::try_from(tw.elapsed().as_nanos()).unwrap_or(u64::MAX);
        live.ensure_target(frame.width, frame.height)?;
        let fresh = live.target.as_ref().is_none_or(|t| t.fresh);
        let scissor = scissor_for(frame.damage, frame.scale, frame.width, frame.height, fresh);
        let depth = max_depth(&self.builder.draws);
        for d in 0..depth.min(MAX_LAYERS) {
            live.layer(d)?;
        }
        let pages = frame.text.as_ref().map_or(0, |(_, c)| c.atlas.page_count());
        let page_size = frame.text.as_ref().map_or(1, |(_, c)| c.atlas.page_size());
        let remade = live.ensure_glyphs(page_size, u32::try_from(pages).unwrap_or(1))?;
        // Instances: the builder's, with shadows pointed at their prototypes, then composites.
        self.instances.clear();
        self.instances.extend_from_slice(&self.builder.instances);
        for d in &self.builder.draws {
            if d.pass == Pass::Draw {
                place_shadows(&mut self.shadows, &mut self.instances, d);
            }
        }
        composite_instances(&self.builder.draws, scissor, &mut self.instances);
        self.bytes.clear();
        for i in &self.instances {
            self.bytes.extend_from_slice(&i.to_bytes());
        }
        let clips_n = u64::try_from(self.builder.clips.len()).unwrap_or(0);
        let inst_n = u64::try_from(self.instances.len()).unwrap_or(0);
        let staging_need = upload_need(&frame, &self.shadows);
        live.reserve(slot, inst_n, clips_n.max(1), staging_need)?;
        let mut clip_bytes = Vec::with_capacity(self.builder.clips.len() * 48);
        for c in &self.builder.clips {
            for v in c
                .rect
                .iter()
                .chain(c.rounded.iter())
                .chain([c.radius, 0.0, 0.0, 0.0].iter())
            {
                clip_bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        if let Some(s) = live.slots.get_mut(slot)
            && (!s.ring.write(0, &self.bytes) || !s.clips.write(0, &clip_bytes))
        {
            return Err(Error::new(Status::Internal, copy::DRIVER));
        }
        live.update_sets(slot);
        let cmd = live.slots.get(slot).map(|s| s.cmd).unwrap_or_default();
        live.begin(cmd)?;
        let uploaded = upload(live, slot, cmd, &mut frame, remade, &mut self.shadows)?;
        let mut stats = FrameStats {
            upload_bytes: uploaded,
            damage_px: if scissor.is_empty() {
                0
            } else {
                scissor.area()
            },
            full: scissor.area() == u64::from(frame.width) * u64::from(frame.height),
            instances: u32::try_from(self.builder.instances.len()).unwrap_or(u32::MAX),
            chunks_rebuilt: self.builder.stats.chunks_rebuilt,
            chunks_reused: self.builder.stats.chunks_reused,
            wait_ns,
            ..FrameStats::default()
        };
        if !scissor.is_empty() {
            let base = u32::try_from(self.builder.instances.len()).unwrap_or(0);
            passes(
                live,
                slot,
                cmd,
                &self.builder.draws,
                base,
                scissor,
                fresh,
                frame.clear,
                &mut stats,
            )?;
            if let Some(t) = live.target.as_mut() {
                t.fresh = false;
            }
        }
        stats.record_ns = u64::try_from(t0.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.stats = stats;
        Ok(slot)
    }

    fn finish_readback(&mut self, slot: usize, w: u32, h: u32) -> Result<Image> {
        let loss = self.take_simulated_loss();
        let Some(live) = self.live.as_mut() else {
            return Err(Error::unavailable(copy::NO_DEVICE));
        };
        let bytes = u64::from(w) * u64::from(h) * 4;
        if live.readback.as_ref().is_none_or(|b| b.size < bytes) {
            if let Some(mut b) = live.readback.take() {
                live.gpu.idle()?;
                b.destroy(&live.gpu);
            }
            live.readback = Some(super::device::Buffer::mapped(
                &live.gpu,
                bytes,
                vk::BufferUsageFlags::TRANSFER_DST,
            )?);
        }
        let cmd = live.slots.get(slot).map(|s| s.cmd).unwrap_or_default();
        let gpu = Arc::clone(&live.gpu);
        let (Some(t), Some(rb)) = (live.target.as_mut(), live.readback.as_ref()) else {
            return Err(Error::unavailable(copy::NO_DEVICE));
        };
        barrier(
            &gpu,
            cmd,
            &mut t.image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        );
        let region = vk::BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D::default(),
            image_extent: vk::Extent3D {
                width: w,
                height: h,
                depth: 1,
            },
        };
        // SAFETY: `cmd` is recording; the target is in TRANSFER_SRC and the buffer holds w*h*4.
        unsafe {
            gpu.device.cmd_copy_image_to_buffer(
                cmd,
                t.image.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                rb.buffer,
                &[region],
            );
        }
        barrier(
            &gpu,
            cmd,
            &mut t.image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        );
        if loss {
            return Err(vk_err(vk::Result::ERROR_DEVICE_LOST));
        }
        live.submit_wait(slot)?;
        let mut data = vec![0u8; bytes.ix()];
        if !live.readback.as_ref().is_some_and(|b| b.read(0, &mut data)) {
            return Err(Error::new(Status::Internal, copy::DRIVER));
        }
        Ok(Image {
            width: w,
            height: h,
            data,
        })
    }
}

/// Deepest nesting of overlapping-opacity layers in a frame.
fn max_depth(draws: &[Draw]) -> usize {
    let (mut d, mut m) = (0usize, 0usize);
    for x in draws {
        match x.pass {
            Pass::LayerBegin => {
                d += 1;
                m = m.max(d);
            }
            Pass::LayerEnd { .. } => d = d.saturating_sub(1),
            Pass::Draw => {}
        }
    }
    m
}

/// Rewrites the shadow instances of one draw: the quad becomes the mask's pixel rectangle in
/// window space (`ABSOLUTE`), `uv` the prototype in the shadow atlas, `extra` its slicing.
fn place_shadows(atlas: &mut ShadowAtlas, inst: &mut [Instance], d: &Draw) {
    let (a, b) = (d.first.ix(), (d.first + d.count).ix());
    let Some(range) = inst.get_mut(a..b) else {
        return;
    };
    for i in range.iter_mut().filter(|i| i.kind() == kind::SHADOW) {
        let cast = Rect::new(
            i.rect[0] + d.offset.0,
            i.rect[1] + d.offset.1,
            i.rect[2],
            i.rect[3],
        );
        let radii = i.radii.map(|q| f32::from(q) / 16.0);
        match atlas.get(&cast, &radii, i.extra[0]) {
            Some((p, ext)) => {
                i.rect = [
                    ext.x0 as f32,
                    ext.y0 as f32,
                    ext.width() as f32,
                    ext.height() as f32,
                ];
                i.extra = [
                    p.slicing.split_x as f32,
                    p.slicing.split_y as f32,
                    p.slicing.stretch_x as f32,
                    p.slicing.stretch_y as f32,
                ];
                i.uv = [p.x, p.y, p.w, p.h];
                i.params = kind::SHADOW | ABSOLUTE;
            }
            None => i.rect = [0.0; 4],
        }
    }
}

/// Appends one composite instance per layer end.
fn composite_instances(draws: &[Draw], scissor: PixelRect, out: &mut Vec<Instance>) {
    for d in draws {
        if let Pass::LayerEnd { alpha } = d.pass {
            out.push(Instance {
                rect: [
                    scissor.x0 as f32,
                    scissor.y0 as f32,
                    scissor.width() as f32,
                    scissor.height() as f32,
                ],
                color0: u32::from(alpha) << 24,
                params: K_LAYER | ABSOLUTE,
                ..Instance::default()
            });
        }
    }
}

/// Staging bytes a frame can need at most: every glyph page and the shadow atlas whole (dirty
/// rectangles are never bigger). The buffer grows once to this and is then reused.
fn upload_need(frame: &Frame<'_>, shadows: &ShadowAtlas) -> u64 {
    let mut n = 0u64;
    if let Some((_, c)) = frame.text.as_ref() {
        let page = u64::from(c.atlas.page_size()) * u64::from(c.atlas.page_size());
        n += u64::try_from(c.atlas.page_count()).unwrap_or(0) * page;
    }
    let s = u64::from(shadows.size());
    n + s * s
}

/// Uploads glyph pages (dirty rectangles, or whole pages after the texture was made) and the
/// shadow atlas (dirty rectangle, or whole after a reset or a new device).
fn upload(
    live: &mut Live,
    slot: usize,
    cmd: vk::CommandBuffer,
    frame: &mut Frame<'_>,
    remade: bool,
    shadows: &mut ShadowAtlas,
) -> Result<u64> {
    let mut at = 0u64;
    let mut total = 0u64;
    if let Some((_, cache)) = frame.text.as_mut() {
        let size = cache.atlas.page_size();
        let si = i32::try_from(size).unwrap_or(0);
        for page in 0..cache.atlas.page_count() {
            let dirty = cache.atlas.take_dirty(page);
            let rect = if remade {
                Some(PixelRect {
                    x0: 0,
                    y0: 0,
                    x1: si,
                    y1: si,
                })
            } else {
                dirty
            };
            let (Some(rect), Some(px)) = (rect, cache.atlas.page_pixels(page)) else {
                continue;
            };
            let n = live.upload_r8(
                slot,
                cmd,
                at,
                px,
                size,
                rect,
                Some(u32::try_from(page).unwrap_or(0)),
            )?;
            at += n;
            total += n;
        }
    }
    let gpu = Arc::clone(&live.gpu);
    if remade {
        barrier(
            &gpu,
            cmd,
            &mut live.glyphs,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
    }
    let si = i32::try_from(shadows.size()).unwrap_or(0);
    let whole = live.shadow_gen != shadows.generation;
    let dirty = shadows.take_dirty();
    let rect = if whole {
        Some(PixelRect {
            x0: 0,
            y0: 0,
            x1: si,
            y1: si,
        })
    } else {
        dirty
    };
    if let Some(rect) = rect {
        let n = live.upload_r8(slot, cmd, at, &shadows.pixels, shadows.size(), rect, None)?;
        total += n;
        live.shadow_gen = shadows.generation;
    }
    Ok(total)
}

struct Pc {
    offset: (f32, f32),
    target: (f32, f32),
}

fn push(live: &Live, cmd: vk::CommandBuffer, pc: &Pc) {
    let mut b = [0u8; 16];
    for (k, v) in [pc.offset.0, pc.offset.1, pc.target.0, pc.target.1]
        .iter()
        .enumerate()
    {
        b[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    // SAFETY: `cmd` is recording with this pipeline layout; 16 bytes match its range.
    unsafe {
        live.gpu.device.cmd_push_constants(
            cmd,
            live.pipes.layout,
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            0,
            &b,
        );
    }
}

/// Begins a render pass on `fb` over `area` and binds the pipeline, ring and descriptor set.
fn begin_pass(
    live: &Live,
    slot: usize,
    cmd: vk::CommandBuffer,
    fb: vk::Framebuffer,
    clear: Option<[f32; 4]>,
    area: vk::Rect2D,
    set: usize,
) {
    let cv = [vk::ClearValue {
        color: vk::ClearColorValue {
            float32: clear.unwrap_or([0.0; 4]),
        },
    }];
    let rp = if clear.is_some() {
        live.pipes.pass_clear
    } else {
        live.pipes.pass_load
    };
    let (Some(s), Some(t)) = (live.slots.get(slot), live.target.as_ref()) else {
        return;
    };
    let info = vk::RenderPassBeginInfo::default()
        .render_pass(rp)
        .framebuffer(fb)
        .render_area(area)
        .clear_values(&cv);
    let vp = [vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: t.image.width as f32,
        height: t.image.height as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    }];
    let d = &live.gpu.device;
    // SAFETY: `cmd` is recording outside a render pass; every handle is live and compatible
    // (the framebuffer was made for `pass_load`, which `pass_clear` is compatible with).
    unsafe {
        d.cmd_begin_render_pass(cmd, &info, vk::SubpassContents::INLINE);
        d.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, live.pipes.pipeline);
        d.cmd_set_viewport(cmd, 0, &vp);
        d.cmd_set_scissor(cmd, 0, &[area]);
        d.cmd_bind_vertex_buffers(cmd, 0, &[s.ring.buffer], &[0]);
        d.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            live.pipes.layout,
            0,
            &[s.sets[set.min(MAX_LAYERS)]],
            &[],
        );
    }
}

fn premul_f(c: Color) -> [f32; 4] {
    let p = c.premul();
    p.map(|v| f32::from(v) / 255.0)
}

/// Records one frame's render passes: the main pass over the scissor, an offscreen pass per
/// overlapping group (up to [`MAX_LAYERS`] deep) and the composites that bring layers back.
struct Recorder<'a> {
    live: &'a mut Live,
    gpu: Arc<Gpu>,
    slot: usize,
    cmd: vk::CommandBuffer,
    area: vk::Rect2D,
    target: (f32, f32),
    target_fb: vk::Framebuffer,
    /// Open layers: true for a real layer, false for one folded into its parent.
    stack: Vec<bool>,
    depth: usize,
}

impl Recorder<'_> {
    fn parent_fb(&self) -> vk::Framebuffer {
        if self.depth == 0 {
            return self.target_fb;
        }
        self.live
            .target
            .as_ref()
            .and_then(|t| t.layers.get(self.depth - 1))
            .map(|l| l.1)
            .unwrap_or_default()
    }

    fn begin_main(&mut self, fresh: bool, clear: Color) {
        let gpu = Arc::clone(&self.gpu);
        if let Some(t) = self.live.target.as_mut() {
            if fresh {
                t.image.layout = vk::ImageLayout::UNDEFINED;
            } else {
                barrier(
                    &gpu,
                    self.cmd,
                    &mut t.image,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                );
            }
        }
        begin_pass(
            self.live,
            self.slot,
            self.cmd,
            self.target_fb,
            fresh.then(|| premul_f(clear)),
            self.area,
            0,
        );
        if !fresh {
            let att = [vk::ClearAttachment {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                color_attachment: 0,
                clear_value: vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: premul_f(clear),
                    },
                },
            }];
            let rect = [vk::ClearRect {
                rect: self.area,
                base_array_layer: 0,
                layer_count: 1,
            }];
            // SAFETY: inside the render pass just begun; the rectangle is its render area.
            unsafe { gpu.device.cmd_clear_attachments(self.cmd, &att, &rect) };
        }
        if let Some(t) = self.live.target.as_mut() {
            t.image.layout = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
        }
    }

    fn draw(&mut self, d: &Draw, stats: &mut FrameStats) {
        push(
            self.live,
            self.cmd,
            &Pc {
                offset: d.offset,
                target: self.target,
            },
        );
        // SAFETY: inside a render pass with the pipeline, ring and set bound.
        unsafe { self.gpu.device.cmd_draw(self.cmd, 4, d.count, 0, d.first) };
        stats.draws += 1;
    }

    fn layer_begin(&mut self, stats: &mut FrameStats) {
        if self.depth >= MAX_LAYERS {
            self.stack.push(false);
            stats.layers_folded += 1;
            return;
        }
        self.stack.push(true);
        // SAFETY: a render pass is open.
        unsafe { self.gpu.device.cmd_end_render_pass(self.cmd) };
        let fb = self
            .live
            .target
            .as_ref()
            .and_then(|t| t.layers.get(self.depth))
            .map(|l| l.1)
            .unwrap_or_default();
        begin_pass(
            self.live,
            self.slot,
            self.cmd,
            fb,
            Some([0.0; 4]),
            self.area,
            0,
        );
        if let Some(l) = self
            .live
            .target
            .as_mut()
            .and_then(|t| t.layers.get_mut(self.depth))
        {
            l.0.layout = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
        }
        self.depth += 1;
        stats.layers += 1;
    }

    /// Closes the innermost layer; with `composite` it is drawn into its parent with that
    /// instance (a folded or unbalanced layer is not).
    fn layer_end(&mut self, composite: Option<u32>, stats: &mut FrameStats) {
        if self.stack.pop() != Some(true) {
            return;
        }
        let d = &self.gpu.device;
        self.depth -= 1;
        // SAFETY: the layer's render pass is open.
        unsafe { d.cmd_end_render_pass(self.cmd) };
        let gpu = Arc::clone(&self.gpu);
        if let Some(l) = self
            .live
            .target
            .as_mut()
            .and_then(|t| t.layers.get_mut(self.depth))
        {
            barrier(
                &gpu,
                self.cmd,
                &mut l.0,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            );
        }
        let parent = self.parent_fb();
        let Some(first) = composite else {
            begin_pass(self.live, self.slot, self.cmd, parent, None, self.area, 0);
            return;
        };
        begin_pass(
            self.live,
            self.slot,
            self.cmd,
            parent,
            None,
            self.area,
            self.depth + 1,
        );
        push(
            self.live,
            self.cmd,
            &Pc {
                offset: (0.0, 0.0),
                target: self.target,
            },
        );
        let set0 = self
            .live
            .slots
            .get(self.slot)
            .map(|s| s.sets[0])
            .unwrap_or_default();
        // SAFETY: inside the parent's render pass; set `depth + 1` samples only this layer, and
        // set 0 (back for the next instances) names no render target.
        unsafe {
            d.cmd_draw(self.cmd, 4, 1, 0, first);
            d.cmd_bind_descriptor_sets(
                self.cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.live.pipes.layout,
                0,
                &[set0],
                &[],
            );
        }
        stats.draws += 1;
    }
}

/// Records the main pass, layer passes and composites over `scissor`.
#[allow(clippy::too_many_arguments)] // the frame's full recording context
fn passes(
    live: &mut Live,
    slot: usize,
    cmd: vk::CommandBuffer,
    draws: &[Draw],
    composite_base: u32,
    scissor: PixelRect,
    fresh: bool,
    clear: Color,
    stats: &mut FrameStats,
) -> Result<()> {
    let Some((tw, th, target_fb)) = live
        .target
        .as_ref()
        .map(|t| (t.image.width, t.image.height, t.fb))
    else {
        return Ok(());
    };
    let area = vk::Rect2D {
        offset: vk::Offset2D {
            x: scissor.x0,
            y: scissor.y0,
        },
        extent: vk::Extent2D {
            width: u32::try_from(scissor.width()).unwrap_or(0),
            height: u32::try_from(scissor.height()).unwrap_or(0),
        },
    };
    let gpu = Arc::clone(&live.gpu);
    let mut r = Recorder {
        live,
        gpu,
        slot,
        cmd,
        area,
        target: (tw as f32, th as f32),
        target_fb,
        stack: Vec::new(),
        depth: 0,
    };
    r.begin_main(fresh, clear);
    let mut composite = composite_base;
    for d in draws {
        match d.pass {
            Pass::Draw if d.count > 0 => r.draw(d, stats),
            Pass::Draw => {}
            Pass::LayerBegin => r.layer_begin(stats),
            Pass::LayerEnd { .. } => {
                r.layer_end(Some(composite), stats);
                composite += 1;
            }
        }
    }
    // An unbalanced list: close open layers without compositing them.
    while !r.stack.is_empty() {
        r.layer_end(None, stats);
    }
    // SAFETY: the main render pass is open.
    unsafe { r.gpu.device.cmd_end_render_pass(cmd) };
    Ok(())
}
