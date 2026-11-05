// SPDX-License-Identifier: BSD-2-Clause
//! The wgpu renderer: the same instances, clip table and atlases as the Vulkan renderer
//! (`wolkit::paint::batch`, `wolkit::gpu::prepare`), drawn by `shaders/ui.wgsl`.
//!
//! Natively wgpu runs on Vulkan (lavapipe in CI, so the trait tests need no GPU); compiled for
//! `wasm32-unknown-unknown` the same code runs on the browser's WebGPU, drawing into a canvas
//! surface ([`WgpuRenderer::render_to`]) or reading a frame back ([`WgpuRenderer::encode_image`]).
//!
//! Per frame: build instances (chunk cache, glyph and picture placement), place shadows and append
//! layer composites, upload the instance ring, the clip table, the dirty rectangles of the glyph
//! pages and of the shadow atlas and the pending picture levels, then record one render pass per
//! run of draws between layer boundaries. A uniform slot per draw stands in for the push
//! constants WebGPU does not have. No damage tracking yet: every frame redraws the whole target.

use std::collections::HashMap;

use wgpu::util::DeviceExt as _;
use wolkit::gpu::SHADOW_ATLAS;
use wolkit::gpu::images::{ImageAtlas, PAGE as IMAGE_PAGE};
use wolkit::gpu::prepare::{clip_bytes, frame_instances, max_depth};
use wolkit::gpu::shadow::ShadowAtlas;
use wolkit::paint::batch::{Draw, Instance, InstanceBuilder, Pass};
use wolkit::paint::geom::{Color, PixelRect};
use wolkit::paint::glyphs::{GlyphSource, NoGlyphs};
use wolkit::text::glyphcache::Glyphs;

use crate::{FrameIn, Image};
#[cfg(not(target_arch = "wasm32"))]
use crate::{RenderError, Renderer};

/// The WGSL uber-shader.
pub const SHADER: &str = include_str!("shaders/ui.wgsl");
/// Offscreen layers kept for overlapping groups (deeper ones are folded), as on Vulkan.
pub const MAX_LAYERS: usize = 4;
/// Bytes between uniform slots (WebGPU's minimum uniform offset alignment).
const PUSH_STRIDE: u64 = 256;
/// The same stride as a byte count and as a dynamic offset (no `as` casts between them).
const PUSH_STRIDE_BYTES: usize = 256;
const PUSH_STRIDE_OFFSET: u32 = 256;
const _: () =
    assert!(PUSH_STRIDE_BYTES as u64 == PUSH_STRIDE && PUSH_STRIDE_OFFSET as u64 == PUSH_STRIDE);
/// The format of layers and of the readback target.
const OFFSCREEN: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[cfg(not(target_arch = "wasm32"))]
fn err(e: impl std::fmt::Display) -> RenderError {
    RenderError(e.to_string())
}

/// Instance attributes, matching `paint::batch::Instance` (64 bytes) and `ui.wgsl` `Inst`.
const ATTRS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
    0 => Float32x4,
    1 => Float32x4,
    2 => Uint32,
    3 => Uint32,
    4 => Uint16x4,
    5 => Uint16x4,
    6 => Uint32,
    7 => Uint32,
];

struct Tex {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    w: u32,
    h: u32,
    layers: u32,
}

fn tex(
    device: &wgpu::Device,
    label: &str,
    (w, h, layers): (u32, u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    array: bool,
) -> Tex {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: layers.max(1),
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(if array {
            wgpu::TextureViewDimension::D2Array
        } else {
            wgpu::TextureViewDimension::D2
        }),
        ..Default::default()
    });
    Tex {
        tex,
        view,
        w: w.max(1),
        h: h.max(1),
        layers: layers.max(1),
    }
}

/// A run of draws into one target (the window or a layer).
struct Segment {
    /// `None` is the window target, `Some(d)` layer `d`.
    target: Option<usize>,
    clear: Option<wgpu::Color>,
    ops: Vec<SegOp>,
}

enum SegOp {
    /// Instances `first..first + count` with uniform slot `push`, bind group 0.
    Draw { first: u32, count: u32, push: u32 },
    /// One composite instance sampling layer `layer`.
    Composite { inst: u32, layer: usize, push: u32 },
}

/// What one frame did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WgpuStats {
    pub draws: u32,
    pub instances: u32,
    pub layers: u32,
    pub layers_folded: u32,
    pub upload_bytes: u64,
    pub passes: u32,
}

pub struct WgpuRenderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    adapter_name: String,
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    bgl0: wgpu::BindGroupLayout,
    bgl1: wgpu::BindGroupLayout,
    pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    builder: InstanceBuilder,
    shadows: ShadowAtlas,
    shadow_gen: u64,
    images: ImageAtlas,
    instances: Vec<Instance>,
    bytes: Vec<u8>,
    clips: Vec<u8>,
    inst_buf: Option<(wgpu::Buffer, u64)>,
    clip_buf: Option<(wgpu::Buffer, u64)>,
    push_buf: Option<(wgpu::Buffer, u64)>,
    glyphs: Option<Tex>,
    /// Glyph pages that hold uploaded texels in the current texture (a new texture is zeros, so
    /// pages never uploaded need only their dirty rectangles).
    glyph_pages_live: usize,
    shadow_tex: Tex,
    image_tex: Option<Tex>,
    dummy: Tex,
    layer_tex: Vec<Tex>,
    readback: Option<Tex>,
    pub stats: WgpuStats,
}

impl WgpuRenderer {
    /// A renderer on an adapter and device the caller already has (the web page, which gets them
    /// asynchronously from the browser).
    pub fn from_device(adapter: &wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let info = adapter.get_info();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wolkit ui.wgsl"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex_entry = |binding, array| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: if array {
                    wgpu::TextureViewDimension::D2Array
                } else {
                    wgpu::TextureViewDimension::D2
                },
                multisampled: false,
            },
            count: None,
        };
        let bgl0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wolkit set 0"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(1, true),
                tex_entry(2, false),
                tex_entry(3, false),
                tex_entry(4, true),
            ],
        });
        let bgl1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wolkit push"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wolkit layout"),
            bind_group_layouts: &[Some(&bgl0), Some(&bgl1)],
            immediate_size: 0,
        });
        let r8 = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let shadow_tex = tex(
            &device,
            "wolkit shadows",
            (SHADOW_ATLAS, SHADOW_ATLAS, 1),
            wgpu::TextureFormat::R8Unorm,
            r8,
            false,
        );
        let dummy = tex(&device, "wolkit no layer", (1, 1, 1), OFFSCREEN, r8, false);
        WgpuRenderer {
            adapter_name: format!("{} ({:?})", info.name, info.backend),
            module,
            layout,
            bgl0,
            bgl1,
            pipelines: HashMap::new(),
            builder: InstanceBuilder::new(),
            shadows: ShadowAtlas::new(SHADOW_ATLAS),
            shadow_gen: 0,
            images: ImageAtlas::new(),
            instances: Vec::new(),
            bytes: Vec::new(),
            clips: Vec::new(),
            inst_buf: None,
            clip_buf: None,
            push_buf: None,
            glyphs: None,
            glyph_pages_live: 0,
            shadow_tex,
            image_tex: None,
            dummy,
            layer_tex: Vec::new(),
            readback: None,
            stats: WgpuStats::default(),
            device,
            queue,
        }
    }

    /// A headless renderer on a native adapter (Vulkan; software first unless `prefer_hardware`).
    /// Blocks; not for the web.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new_native(prefer_hardware: bool) -> Result<WgpuRenderer, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let hardware = prefer_hardware;
        let adapter = pollster_block(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: !hardware,
            compatible_surface: None,
            ..Default::default()
        }))
        .or_else(|_| {
            pollster_block(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        })
        .map_err(err)?;
        let (device, queue) =
            pollster_block(adapter.request_device(&device_descriptor(&adapter))).map_err(err)?;
        Ok(WgpuRenderer::from_device(&adapter, device, queue))
    }

    /// Forgets every cached chunk, so the next frame builds all instances again (a different
    /// display list whose chunk ids and versions happen to repeat, as between test scenes).
    pub fn invalidate(&mut self) {
        self.builder = InstanceBuilder::new();
    }

    /// The adapter's name and backend.
    pub fn adapter(&self) -> &str {
        &self.adapter_name
    }

    fn pipeline(&mut self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        let (device, module, layout) = (&self.device, &self.module, &self.layout);
        self.pipelines.entry(format).or_insert_with(|| {
            let blend = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("wolkit uber-pipeline"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &ATTRS,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: blend,
                            alpha: blend,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// A buffer of at least `need` bytes with `usage`, grown by doubling.
    fn ensure_buf(
        device: &wgpu::Device,
        slot: &mut Option<(wgpu::Buffer, u64)>,
        need: u64,
        usage: wgpu::BufferUsages,
        label: &str,
    ) {
        if slot.as_ref().is_some_and(|(_, n)| *n >= need) {
            return;
        }
        let size = need.max(256).next_power_of_two();
        *slot = Some((
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            size,
        ));
    }

    /// Builds the frame's instances and uploads everything it needs. Returns the draws.
    fn prepare(&mut self, frame: &mut FrameIn<'_>) -> Vec<Draw> {
        let glyph_gen = frame
            .text
            .as_ref()
            .map_or(0, |(_, c)| c.atlas.stats.evictions);
        for _ in 0..2 {
            let atlas_gen = glyph_gen.wrapping_add(self.images.generation << 40);
            let mut none = NoGlyphs;
            let mut with;
            let src: &mut dyn GlyphSource = match frame.text.as_mut() {
                Some((db, cache)) => {
                    with = Glyphs { db, cache };
                    &mut with
                }
                None => &mut none,
            };
            self.builder.build_with_images(
                frame.list,
                src,
                &mut self.images,
                frame.scale,
                atlas_gen,
            );
            if !self.images.take_overflow() {
                break;
            }
            self.images.reset();
        }
        let full = PixelRect {
            x0: 0,
            y0: 0,
            x1: i32::try_from(frame.width).unwrap_or(i32::MAX),
            y1: i32::try_from(frame.height).unwrap_or(i32::MAX),
        };
        frame_instances(
            &self.builder.instances,
            &self.builder.draws,
            &mut self.shadows,
            full,
            &mut self.instances,
        );
        self.bytes.clear();
        for i in &self.instances {
            self.bytes.extend_from_slice(&i.to_bytes());
        }
        clip_bytes(&self.builder.clips, &mut self.clips);
        let mut uploaded = 0u64;
        Self::ensure_buf(
            &self.device,
            &mut self.inst_buf,
            self.bytes.len() as u64,
            wgpu::BufferUsages::VERTEX,
            "wolkit instances",
        );
        Self::ensure_buf(
            &self.device,
            &mut self.clip_buf,
            self.clips.len() as u64,
            wgpu::BufferUsages::STORAGE,
            "wolkit clips",
        );
        if let (Some((ib, _)), Some((cb, _))) = (&self.inst_buf, &self.clip_buf) {
            if !self.bytes.is_empty() {
                self.queue.write_buffer(ib, 0, &self.bytes);
            }
            self.queue.write_buffer(cb, 0, &self.clips);
        }
        uploaded += (self.bytes.len() + self.clips.len()) as u64;
        uploaded += self.upload_glyphs(frame);
        uploaded += self.upload_shadows();
        uploaded += self.upload_images();
        self.stats.upload_bytes = uploaded;
        self.stats.instances = u32::try_from(self.builder.instances.len()).unwrap_or(u32::MAX);
        self.builder.draws.clone()
    }

    fn upload_glyphs(&mut self, frame: &mut FrameIn<'_>) -> u64 {
        let Some((_, cache)) = frame.text.as_mut() else {
            if self.glyphs.is_none() {
                self.glyphs = Some(tex(
                    &self.device,
                    "wolkit glyphs",
                    (1, 1, 1),
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    true,
                ));
            }
            return 0;
        };
        let size = cache.atlas.page_size();
        let pages = u32::try_from(cache.atlas.page_count()).unwrap_or(1).max(1);
        let remade = self
            .glyphs
            .as_ref()
            .is_none_or(|t| t.w != size || t.layers < pages);
        let live = if remade {
            self.glyphs = Some(tex(
                &self.device,
                "wolkit glyphs",
                (size, size, pages),
                wgpu::TextureFormat::R8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                true,
            ));
            std::mem::take(&mut self.glyph_pages_live)
        } else {
            0
        };
        let Some(t) = self.glyphs.as_ref() else {
            return 0;
        };
        let si = i32::try_from(size).unwrap_or(0);
        let mut n = 0u64;
        for page in 0..cache.atlas.page_count() {
            let dirty = cache.atlas.take_dirty(page);
            // Pages the old texture held go up whole; the rest only where they were drawn.
            let rect = if remade && page < live {
                Some(PixelRect {
                    x0: 0,
                    y0: 0,
                    x1: si,
                    y1: si,
                })
            } else {
                dirty
            };
            let (Some(r), Some(px)) = (rect, cache.atlas.page_pixels(page)) else {
                continue;
            };
            self.glyph_pages_live = self.glyph_pages_live.max(page + 1);
            n += write_r8(
                &self.queue,
                &t.tex,
                px,
                size,
                r,
                u32::try_from(page).unwrap_or(0),
            );
        }
        n
    }

    fn upload_shadows(&mut self) -> u64 {
        let si = i32::try_from(self.shadows.size()).unwrap_or(0);
        // A new generation (the atlas was cleared) goes up whole, except the first: the texture
        // starts as zeros, so the dirty rectangle is all it lacks.
        let whole = self.shadow_gen != 0 && self.shadow_gen != self.shadows.generation;
        let dirty = self.shadows.take_dirty();
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
        let Some(r) = rect else {
            return 0;
        };
        self.shadow_gen = self.shadows.generation;
        write_r8(
            &self.queue,
            &self.shadow_tex.tex,
            &self.shadows.pixels,
            self.shadows.size(),
            r,
            0,
        )
    }

    fn upload_images(&mut self) -> u64 {
        let pages = self.images.page_count().max(1);
        // A 1x1 stand-in until the first picture, then full pages.
        let side = if self.images.page_count() == 0 {
            1
        } else {
            IMAGE_PAGE
        };
        if self
            .image_tex
            .as_ref()
            .is_none_or(|t| t.layers < pages || t.w < side)
        {
            self.image_tex = Some(tex(
                &self.device,
                "wolkit pictures",
                (side, side, pages),
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                true,
            ));
            if self.images.page_count() > 0 {
                self.images.reupload_all();
            }
        }
        let Some(t) = self.image_tex.as_ref() else {
            return 0;
        };
        let mut n = 0u64;
        for u in std::mem::take(&mut self.images.pending) {
            let Some(l) = u.px.levels.get(u.level) else {
                continue;
            };
            if l.w == 0 || l.h == 0 || u.x + l.w > t.w || u.y + l.h > t.h {
                continue;
            }
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &t.tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: u.x,
                        y: u.y,
                        z: u.page,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &l.px,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(l.w * 4),
                    rows_per_image: Some(l.h),
                },
                wgpu::Extent3d {
                    width: l.w,
                    height: l.h,
                    depth_or_array_layers: 1,
                },
            );
            n += l.px.len() as u64;
        }
        n
    }

    fn ensure_layers(&mut self, depth: usize, w: u32, h: u32) {
        if self.layer_tex.first().is_some_and(|t| t.w != w || t.h != h) {
            self.layer_tex.clear();
        }
        while self.layer_tex.len() < depth.min(MAX_LAYERS) {
            self.layer_tex.push(tex(
                &self.device,
                "wolkit layer",
                (w, h, 1),
                OFFSCREEN,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                false,
            ));
        }
    }

    /// Turns the draws into passes, mirroring the Vulkan recorder: a layer begin ends the pass
    /// and starts one on the layer (up to [`MAX_LAYERS`] deep, deeper ones fold into their
    /// parent), a layer end goes back to the parent and composites the layer with one instance.
    fn segments(
        &mut self,
        draws: &[Draw],
        clear: Color,
        pushes: &mut Vec<[f32; 2]>,
    ) -> Vec<Segment> {
        let premul = clear.premul().map(|v| f64::from(v) / 255.0);
        let mut segs = vec![Segment {
            target: None,
            clear: Some(wgpu::Color {
                r: premul[0],
                g: premul[1],
                b: premul[2],
                a: premul[3],
            }),
            ops: Vec::new(),
        }];
        let mut stack: Vec<bool> = Vec::new();
        let mut depth = 0usize;
        let mut composite = u32::try_from(self.builder.instances.len()).unwrap_or(0);
        let mut push = |o: (f32, f32)| {
            pushes.push([o.0, o.1]);
            u32::try_from(pushes.len() - 1).unwrap_or(0)
        };
        let target_of = |depth: usize| if depth == 0 { None } else { Some(depth - 1) };
        for d in draws {
            match d.pass {
                Pass::Draw if d.count > 0 => {
                    let p = push(d.offset);
                    if let Some(s) = segs.last_mut() {
                        s.ops.push(SegOp::Draw {
                            first: d.first,
                            count: d.count,
                            push: p,
                        });
                    }
                    self.stats.draws += 1;
                }
                Pass::Draw => {}
                Pass::LayerBegin => {
                    if depth >= MAX_LAYERS {
                        stack.push(false);
                        self.stats.layers_folded += 1;
                        continue;
                    }
                    stack.push(true);
                    segs.push(Segment {
                        target: Some(depth),
                        clear: Some(wgpu::Color::TRANSPARENT),
                        ops: Vec::new(),
                    });
                    depth += 1;
                    self.stats.layers += 1;
                }
                Pass::LayerEnd { .. } => {
                    let inst = composite;
                    composite += 1;
                    if stack.pop() != Some(true) {
                        continue;
                    }
                    depth -= 1;
                    let p = push((0.0, 0.0));
                    segs.push(Segment {
                        target: target_of(depth),
                        clear: None,
                        ops: vec![SegOp::Composite {
                            inst,
                            layer: depth,
                            push: p,
                        }],
                    });
                    self.stats.draws += 1;
                }
            }
        }
        // An unbalanced list: open layers are left without compositing them, as on Vulkan.
        segs
    }

    /// Draws `frame` into `view` (format `format`, the frame's size). The web page passes the
    /// canvas's current texture; [`Renderer::render_image`] an offscreen one.
    pub fn render_to(
        &mut self,
        mut frame: FrameIn<'_>,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) -> wgpu::CommandEncoder {
        self.stats = WgpuStats::default();
        let draws = self.prepare(&mut frame);
        self.ensure_layers(max_depth(&draws), frame.width, frame.height);
        let mut pushes = Vec::new();
        let segs = self.segments(&draws, frame.clear, &mut pushes);
        // Uniform slots: offset then target size, 16 bytes at a 256-byte stride.
        let mut ub = vec![0u8; pushes.len().max(1) * PUSH_STRIDE_BYTES];
        for (k, p) in pushes.iter().enumerate() {
            let o = k * PUSH_STRIDE_BYTES;
            for (j, v) in [p[0], p[1], frame.width as f32, frame.height as f32]
                .iter()
                .enumerate()
            {
                ub[o + j * 4..o + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        Self::ensure_buf(
            &self.device,
            &mut self.push_buf,
            ub.len() as u64,
            wgpu::BufferUsages::UNIFORM,
            "wolkit push",
        );
        let pipeline = self.pipeline(format).clone();
        let (Some((pb, _)), Some((cb, _)), Some((ib, _)), Some(glyphs), Some(images)) = (
            self.push_buf.as_ref(),
            self.clip_buf.as_ref(),
            self.inst_buf.as_ref(),
            self.glyphs.as_ref(),
            self.image_tex.as_ref(),
        ) else {
            return self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        };
        self.queue.write_buffer(pb, 0, &ub);
        let set0 = |layer: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("wolkit set 0"),
                layout: &self.bgl0,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: cb.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&glyphs.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.shadow_tex.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(layer),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&images.view),
                    },
                ],
            })
        };
        let base = set0(&self.dummy.view);
        let per_layer: Vec<wgpu::BindGroup> =
            self.layer_tex.iter().map(|t| set0(&t.view)).collect();
        let push_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wolkit push"),
            layout: &self.bgl1,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: pb,
                    offset: 0,
                    size: wgpu::BufferSize::new(16),
                }),
            }],
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wolkit frame"),
            });
        for s in &segs {
            let target = match s.target {
                None => view,
                Some(d) => match self.layer_tex.get(d) {
                    Some(t) => &t.view,
                    None => continue,
                },
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wolkit pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: s.clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.stats.passes += 1;
            pass.set_pipeline(&pipeline);
            pass.set_vertex_buffer(0, ib.slice(..));
            for op in &s.ops {
                match *op {
                    SegOp::Draw { first, count, push } => {
                        pass.set_bind_group(0, &base, &[]);
                        pass.set_bind_group(1, &push_bg, &[push * PUSH_STRIDE_OFFSET]);
                        pass.draw(0..4, first..first + count);
                    }
                    SegOp::Composite { inst, layer, push } => {
                        let Some(bg) = per_layer.get(layer) else {
                            continue;
                        };
                        pass.set_bind_group(0, bg, &[]);
                        pass.set_bind_group(1, &push_bg, &[push * PUSH_STRIDE_OFFSET]);
                        pass.draw(0..4, inst..inst + 1);
                    }
                }
            }
        }
        enc
    }

    /// Draws `frame` offscreen and records the copy to a readback buffer. The caller submits the
    /// encoder, maps the buffer, and calls [`unpad`] on the bytes (`bytes_per_row` padded to 256).
    pub fn encode_image(
        &mut self,
        frame: FrameIn<'_>,
    ) -> (wgpu::CommandEncoder, wgpu::Buffer, u32) {
        let (w, h) = (frame.width.max(1), frame.height.max(1));
        if self.readback.as_ref().is_none_or(|t| t.w != w || t.h != h) {
            self.readback = Some(tex(
                &self.device,
                "wolkit offscreen",
                (w, h, 1),
                OFFSCREEN,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                false,
            ));
        }
        let view = self.readback.as_ref().map(|t| t.view.clone());
        let Some(view) = view else {
            unreachable!("made above")
        };
        let enc = self.render_to(frame, &view, OFFSCREEN);
        self.readback_copy(enc, w, h)
    }

    /// The exact path (WOLKIT-1TO1): a frame the CPU reference drew, written into `texture`
    /// byte for byte (`queue.write_texture`, no sampling, no blending), swizzled when the target
    /// is BGRA. What the canvas shows is then the CPU frame, so the web page and the shell's Kit
    /// windows (which paint the same frame) are the same pixels on every GPU. `texture` needs
    /// `COPY_DST` and an 8-bit unorm format (RGBA or BGRA).
    pub fn upload_frame(&self, img: &Image, texture: &wgpu::Texture) {
        let bgra = matches!(
            texture.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let swizzled;
        let data: &[u8] = if bgra {
            swizzled = img
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[2], p[1], p[0], p[3]])
                .collect::<Vec<u8>>();
            &swizzled
        } else {
            &img.data
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(img.width * 4),
                rows_per_image: Some(img.height),
            },
            wgpu::Extent3d {
                width: img.width,
                height: img.height,
                depth_or_array_layers: 1,
            },
        );
    }

    /// [`Self::upload_frame`] into the offscreen target and the copy to a readback buffer, as
    /// [`Self::encode_image`] does for a GPU-drawn frame (tests read the exact path back).
    pub fn encode_upload(&mut self, img: &Image) -> (wgpu::CommandEncoder, wgpu::Buffer, u32) {
        let (w, h) = (img.width.max(1), img.height.max(1));
        if self.readback.as_ref().is_none_or(|t| t.w != w || t.h != h) {
            self.readback = Some(tex(
                &self.device,
                "wolkit offscreen",
                (w, h, 1),
                OFFSCREEN,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                false,
            ));
        }
        if let Some(t) = self.readback.as_ref() {
            self.upload_frame(img, &t.tex);
        }
        let enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wolkit upload"),
            });
        self.readback_copy(enc, w, h)
    }

    fn readback_copy(
        &mut self,
        mut enc: wgpu::CommandEncoder,
        w: u32,
        h: u32,
    ) -> (wgpu::CommandEncoder, wgpu::Buffer, u32) {
        let row = (w * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wolkit readback"),
            size: u64::from(row) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        if let Some(t) = self.readback.as_ref() {
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &t.tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(h),
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        (enc, buf, row)
    }

    /// A buffer the size of `data`, filled (tests).
    pub fn buffer_init(&self, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: data,
                usage,
            })
    }
}

/// Rows of `row` padded bytes to `w * 4` bytes each.
pub fn unpad(padded: &[u8], w: u32, h: u32, row: u32) -> Vec<u8> {
    let ix = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
    let line = ix(w) * 4;
    let mut out = Vec::with_capacity(line * ix(h));
    for y in 0..ix(h) {
        let o = y * ix(row);
        out.extend_from_slice(&padded[o..o + line]);
    }
    out
}

/// Uploads rectangle `r` of an R8 page (`size` wide) to `layer` of `t`.
fn write_r8(
    queue: &wgpu::Queue,
    t: &wgpu::Texture,
    px: &[u8],
    size: u32,
    r: PixelRect,
    layer: u32,
) -> u64 {
    let x0 = u32::try_from(r.x0.max(0)).unwrap_or(0);
    let y0 = u32::try_from(r.y0.max(0)).unwrap_or(0);
    let x1 = u32::try_from(r.x1.max(0)).unwrap_or(0).min(size);
    let y1 = u32::try_from(r.y1.max(0)).unwrap_or(0).min(size);
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    let off = usize::try_from(y0 * size + x0).unwrap_or(usize::MAX);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: t,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: x0,
                y: y0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &px[off..],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: x1 - x0,
            height: y1 - y0,
            depth_or_array_layers: 1,
        },
    );
    u64::from((x1 - x0) * (y1 - y0))
}

/// The device the renderer asks for: WebGPU's defaults, within what the adapter has.
pub fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some("wolkit"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pollster_block<F: std::future::Future>(f: F) -> F::Output {
    // A tiny executor: wgpu's native futures are ready once polled after the blocking calls.
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl WgpuRenderer {
    /// The exact path natively: `img` written into the GPU target and read back (what the web
    /// page's canvas holds when it shows a CPU frame).
    pub fn upload_image(&mut self, img: &Image) -> Result<Image, RenderError> {
        let (w, h) = (img.width.max(1), img.height.max(1));
        let (enc, buf, row) = self.encode_upload(img);
        self.read_back(enc, &buf, w, h, row)
    }

    fn read_back(
        &mut self,
        enc: wgpu::CommandEncoder,
        buf: &wgpu::Buffer,
        w: u32,
        h: u32,
        row: u32,
    ) -> Result<Image, RenderError> {
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(err)?;
        rx.recv().map_err(err)?.map_err(err)?;
        let data = {
            let view = slice.get_mapped_range().map_err(err)?;
            unpad(&view, w, h, row)
        };
        buf.unmap();
        Ok(Image {
            width: w,
            height: h,
            data,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Renderer for WgpuRenderer {
    fn name(&self) -> String {
        format!("wgpu {}", self.adapter_name)
    }

    fn render_image(&mut self, frame: FrameIn<'_>) -> Result<Image, RenderError> {
        // The trait draws one frame on its own, like the Vulkan renderer's `render_image` after
        // `invalidate`; the live page uses `render_to`, which keeps the chunk cache.
        self.invalidate();
        let (w, h) = (frame.width.max(1), frame.height.max(1));
        let (enc, buf, row) = self.encode_image(frame);
        self.read_back(enc, &buf, w, h, row)
    }
}
