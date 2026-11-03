// SPDX-License-Identifier: BSD-2-Clause
//! Everything that lives on one Vulkan device: pipelines, the frame slots with the persistent
//! instance ring, textures, the render target and its layers. Dropped as a whole when the device
//! is lost; [`super::frame::Renderer`] makes a new one on recovery.
//!
//! Frames in flight: [`SLOTS`] slots, each with a command buffer, a fence, its third of the ring
//! (instances and the clip table, written straight into mapped memory, no staging copy), an
//! upload staging buffer and descriptor sets. A slot is reused only after its fence signalled.

use std::sync::Arc;

use ash::vk;

use super::device::{Buffer, Gpu, Image, barrier, vk_err};
use super::pipelines::{MAX_LAYERS, Pipelines};
use crate::error::Result;
use crate::paint::geom::PixelRect;
use crate::scene::ids::Ix;

/// Frames in flight.
pub const SLOTS: usize = 3;
/// Initial ring room per slot, in instances (grows by doubling).
pub const RING_INSTANCES: u64 = 4096;
/// The picture pages' format: the CPU rasteriser's premultiplied byte order.
pub const IMAGE_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
/// Descriptor sets per slot (see [`Slot::sets`]).
pub const SETS: usize = MAX_LAYERS + 1;

pub struct Slot {
    pub cmd: vk::CommandBuffer,
    pub fence: vk::Fence,
    /// Signalled by the submit when presenting (the swapchain waits on it).
    pub done: vk::Semaphore,
    pub ring: Buffer,
    pub clips: Buffer,
    pub staging: Buffer,
    /// Set 0 draws (binding 3 is the dummy); set `d + 1` composites layer `d` (binding 3 is that
    /// layer). So no bound set ever names an image while it is a render target.
    pub sets: [vk::DescriptorSet; SETS],
    /// The resource epoch the sets were written for.
    pub epoch: u64,
    pub busy: bool,
}

/// The render target and its offscreen layers (same size and format).
pub struct Target {
    pub image: Image,
    pub fb: vk::Framebuffer,
    pub layers: Vec<(Image, vk::Framebuffer)>,
    /// Nothing valid has been drawn yet (the first frame clears everything).
    pub fresh: bool,
}

pub struct Live {
    pub gpu: Arc<Gpu>,
    pub pipes: Pipelines,
    pub pool: vk::CommandPool,
    pub desc_pool: vk::DescriptorPool,
    pub slots: Vec<Slot>,
    /// The glyph pages (an R8 2D array; one 1x1 page until the first glyph).
    pub glyphs: Image,
    /// The picture pages (an RGBA8 2D array, premultiplied; one 1x1 page until the first).
    pub images: Image,
    pub shadows: Image,
    /// The shadow atlas generation the texture holds.
    pub shadow_gen: u64,
    /// A 1x1 image bound where no layer exists yet.
    pub dummy: Image,
    pub target: Option<Target>,
    pub readback: Option<Buffer>,
    /// Bumped whenever a buffer or image a descriptor points at is replaced.
    pub epoch: u64,
}

fn slot(
    gpu: &Gpu,
    pool: vk::CommandPool,
    dpool: vk::DescriptorPool,
    layout: vk::DescriptorSetLayout,
) -> Result<Slot> {
    let d = &gpu.device;
    let usage = vk::BufferUsageFlags::VERTEX_BUFFER;
    let ring = Buffer::mapped(gpu, RING_INSTANCES * 64, usage)?;
    let clips = Buffer::mapped(gpu, 4096 * 48, vk::BufferUsageFlags::STORAGE_BUFFER)?;
    let staging = Buffer::mapped(gpu, 1 << 20, vk::BufferUsageFlags::TRANSFER_SRC)?;
    let layouts = [layout; SETS];
    // SAFETY: calls on a live device with pools it owns; the infos outlive the calls.
    let (cmd, fence, done, sets) = unsafe {
        let cmd = d
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .map_err(vk_err)?;
        let fence = d
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(vk_err)?;
        let done = d
            .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
            .map_err(vk_err)?;
        let sets = d
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(dpool)
                    .set_layouts(&layouts),
            )
            .map_err(vk_err)?;
        (cmd, fence, done, sets)
    };
    let mut s = [vk::DescriptorSet::null(); SETS];
    for (o, v) in s.iter_mut().zip(sets) {
        *o = v;
    }
    Ok(Slot {
        cmd: cmd.first().copied().unwrap_or_default(),
        fence,
        done,
        ring,
        clips,
        staging,
        sets: s,
        epoch: 0,
        busy: false,
    })
}

impl Live {
    pub fn new(
        gpu: Arc<Gpu>,
        format: vk::Format,
        cache_file: Option<&std::path::Path>,
    ) -> Result<Live> {
        let pipes = Pipelines::new(&gpu, format, cache_file)?;
        let d = &gpu.device;
        let n = u32::try_from(SLOTS * SETS).unwrap_or(16);
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: n,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                descriptor_count: 4 * n,
            },
        ];
        // SAFETY: calls on a live device; the infos outlive them.
        let (pool, desc_pool) = unsafe {
            let pool = d.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(gpu.family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            );
            let dp = d.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(n)
                    .pool_sizes(&sizes),
                None,
            );
            (pool.map_err(vk_err), dp.map_err(vk_err))
        };
        let usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
        let shadows = Image::new(
            &gpu,
            (super::SHADOW_ATLAS, super::SHADOW_ATLAS, 1),
            false,
            vk::Format::R8_UNORM,
            usage,
        );
        let dummy = Image::new(
            &gpu,
            (1, 1, 1),
            false,
            format,
            usage | vk::ImageUsageFlags::COLOR_ATTACHMENT,
        );
        let glyphs = Image::new(&gpu, (1, 1, 1), true, vk::Format::R8_UNORM, usage);
        let images = Image::new(&gpu, (1, 1, 1), true, IMAGE_FORMAT, usage);
        let mut live = Live {
            pipes,
            pool: pool.as_ref().copied().unwrap_or_default(),
            desc_pool: desc_pool.as_ref().copied().unwrap_or_default(),
            slots: Vec::new(),
            glyphs: Image::placeholder(),
            images: Image::placeholder(),
            shadows: Image::placeholder(),
            shadow_gen: 0,
            dummy: Image::placeholder(),
            target: None,
            readback: None,
            epoch: 1,
            gpu,
        };
        // Everything made so far is owned by `live` now, so an error below drops it cleanly.
        pool?;
        desc_pool?;
        live.shadows = shadows?;
        live.dummy = dummy?;
        live.glyphs = glyphs?;
        live.images = images?;
        for _ in 0..SLOTS {
            let s = slot(&live.gpu, live.pool, live.desc_pool, live.pipes.set_layout)?;
            live.slots.push(s);
        }
        live.init_images()?;
        Ok(live)
    }

    /// Puts the shadow atlas and the dummy into the layouts the shader reads.
    fn init_images(&mut self) -> Result<()> {
        let cmd = self.slots.first().map(|s| s.cmd).unwrap_or_default();
        self.begin(cmd)?;
        barrier(
            &self.gpu,
            cmd,
            &mut self.shadows,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
        barrier(
            &self.gpu,
            cmd,
            &mut self.dummy,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
        barrier(
            &self.gpu,
            cmd,
            &mut self.glyphs,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
        barrier(
            &self.gpu,
            cmd,
            &mut self.images,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
        self.submit_wait(0)
    }

    pub fn begin(&self, cmd: vk::CommandBuffer) -> Result<()> {
        // SAFETY: `cmd` belongs to this device's pool and is not pending (its fence was waited).
        unsafe {
            self.gpu
                .device
                .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())
                .and_then(|()| {
                    self.gpu.device.begin_command_buffer(
                        cmd,
                        &vk::CommandBufferBeginInfo::default()
                            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                    )
                })
        }
        .map_err(vk_err)
    }

    /// Ends and submits slot `i`'s command buffer and waits for it.
    pub fn submit_wait(&mut self, i: usize) -> Result<()> {
        self.submit(i, &[], &[], false)?;
        self.wait(i)
    }

    /// Ends and submits slot `i`; with `signal_done` the slot's semaphore is signalled too.
    pub fn submit(
        &mut self,
        i: usize,
        wait: &[vk::Semaphore],
        stages: &[vk::PipelineStageFlags],
        signal_done: bool,
    ) -> Result<()> {
        let Some(s) = self.slots.get_mut(i) else {
            return Ok(());
        };
        let cmds = [s.cmd];
        let sig = [s.done];
        let mut info = vk::SubmitInfo::default()
            .command_buffers(&cmds)
            .wait_semaphores(wait)
            .wait_dst_stage_mask(stages);
        if signal_done {
            info = info.signal_semaphores(&sig);
        }
        let d = &self.gpu.device;
        // SAFETY: the command buffer is recording on this device; the fence is unsignalled (reset
        // after its last wait) and the arrays outlive the call.
        unsafe {
            d.end_command_buffer(s.cmd)
                .and_then(|()| d.queue_submit(self.gpu.queue, &[info], s.fence))
        }
        .map_err(vk_err)?;
        s.busy = true;
        Ok(())
    }

    /// Waits for slot `i`'s last submit and resets its fence.
    pub fn wait(&mut self, i: usize) -> Result<()> {
        let Some(s) = self.slots.get_mut(i) else {
            return Ok(());
        };
        if !s.busy {
            return Ok(());
        }
        let f = [s.fence];
        // SAFETY: the fence belongs to this device and was submitted.
        unsafe {
            self.gpu
                .device
                .wait_for_fences(&f, true, 5_000_000_000)
                .and_then(|()| self.gpu.device.reset_fences(&f))
        }
        .map_err(vk_err)?;
        s.busy = false;
        Ok(())
    }

    /// Makes sure slot `i`'s ring holds `instances` and its clip buffer `clips` rows.
    pub fn reserve(&mut self, i: usize, instances: u64, clips: u64, staging: u64) -> Result<()> {
        let gpu = Arc::clone(&self.gpu);
        let Some(s) = self.slots.get_mut(i) else {
            return Ok(());
        };
        let grow = |b: &mut Buffer, need: u64, usage: vk::BufferUsageFlags| -> Result<bool> {
            if b.size >= need {
                return Ok(false);
            }
            let size = need.next_power_of_two();
            let nb = Buffer::mapped(&gpu, size, usage)?;
            b.destroy(&gpu);
            *b = nb;
            Ok(true)
        };
        let a = grow(
            &mut s.ring,
            instances * 64,
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        let b = grow(
            &mut s.clips,
            clips * 48,
            vk::BufferUsageFlags::STORAGE_BUFFER,
        )?;
        let _ = grow(&mut s.staging, staging, vk::BufferUsageFlags::TRANSFER_SRC)?;
        if a || b {
            s.epoch = 0;
        }
        Ok(())
    }

    /// Creates or resizes the target (and drops the layers, made again on demand).
    pub fn ensure_target(&mut self, w: u32, h: u32) -> Result<()> {
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.image.width == w && t.image.height == h)
        {
            return Ok(());
        }
        self.gpu.idle()?;
        self.drop_target();
        let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
            | vk::ImageUsageFlags::TRANSFER_SRC
            | vk::ImageUsageFlags::TRANSFER_DST;
        let mut image = Image::new(&self.gpu, (w, h, 1), false, self.pipes.format, usage)?;
        let fb = match self.framebuffer(&image) {
            Ok(f) => f,
            Err(e) => {
                image.destroy(&self.gpu);
                return Err(e);
            }
        };
        self.target = Some(Target {
            image,
            fb,
            layers: Vec::new(),
            fresh: true,
        });
        Ok(())
    }

    /// The layer image at `depth`, made on first use.
    pub fn layer(&mut self, depth: usize) -> Result<()> {
        let Some((w, h)) = self
            .target
            .as_ref()
            .map(|t| (t.image.width, t.image.height))
        else {
            return Ok(());
        };
        while self
            .target
            .as_ref()
            .is_some_and(|t| t.layers.len() <= depth)
        {
            let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED;
            let mut img = Image::new(&self.gpu, (w, h, 1), false, self.pipes.format, usage)?;
            let fb = match self.framebuffer(&img) {
                Ok(f) => f,
                Err(e) => {
                    img.destroy(&self.gpu);
                    return Err(e);
                }
            };
            if let Some(t) = self.target.as_mut() {
                t.layers.push((img, fb));
            }
            self.epoch += 1;
        }
        Ok(())
    }

    fn framebuffer(&self, img: &Image) -> Result<vk::Framebuffer> {
        let views = [img.view];
        let info = vk::FramebufferCreateInfo::default()
            .render_pass(self.pipes.pass_load)
            .attachments(&views)
            .width(img.width)
            .height(img.height)
            .layers(1);
        // SAFETY: call on a live device; the view belongs to it.
        unsafe { self.gpu.device.create_framebuffer(&info, None) }.map_err(vk_err)
    }

    fn drop_target(&mut self) {
        if let Some(mut t) = self.target.take() {
            // SAFETY: the caller waited for the device to go idle.
            unsafe { self.gpu.device.destroy_framebuffer(t.fb, None) };
            t.image.destroy(&self.gpu);
            for (mut img, fb) in t.layers.drain(..) {
                // SAFETY: same as above.
                unsafe { self.gpu.device.destroy_framebuffer(fb, None) };
                img.destroy(&self.gpu);
            }
            self.epoch += 1;
        }
        if let Some(mut b) = self.readback.take() {
            b.destroy(&self.gpu);
        }
    }

    /// Makes the glyph texture array hold `pages` pages of `size` squared; true when it was
    /// (re)made, so every page must be uploaded whole.
    pub fn ensure_glyphs(&mut self, size: u32, pages: u32) -> Result<bool> {
        let pages = pages.max(1);
        if self.glyphs.width == size && self.glyphs.layers >= pages {
            return Ok(false);
        }
        self.gpu.idle()?;
        let usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
        let img = Image::new(
            &self.gpu,
            (size, size, pages),
            true,
            vk::Format::R8_UNORM,
            usage,
        )?;
        self.glyphs.destroy(&self.gpu);
        self.glyphs = img;
        self.epoch += 1;
        Ok(true)
    }

    /// Makes the picture texture array hold `pages` pages of `size` squared; true when it was
    /// (re)made, so every placed level must be uploaded again.
    pub fn ensure_images(&mut self, size: u32, pages: u32) -> Result<bool> {
        if pages == 0 || (self.images.width == size && self.images.layers >= pages) {
            return Ok(false);
        }
        self.gpu.idle()?;
        let usage = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
        let img = Image::new(&self.gpu, (size, size, pages), true, IMAGE_FORMAT, usage)?;
        self.images.destroy(&self.gpu);
        self.images = img;
        self.epoch += 1;
        Ok(true)
    }

    /// Copies picture levels into the texture array through slot `i`'s staging buffer from
    /// `at` (rounded up to 4); returns the bytes used. One barrier pair for the whole batch.
    pub fn upload_images(
        &mut self,
        i: usize,
        cmd: vk::CommandBuffer,
        at: u64,
        ups: &[super::images::Upload],
    ) -> Result<u64> {
        let Some(s) = self.slots.get_mut(i) else {
            return Ok(0);
        };
        let mut off = at.next_multiple_of(4);
        let mut regions = Vec::with_capacity(ups.len());
        for u in ups {
            let Some(l) = u.px.levels.get(u.level) else {
                continue;
            };
            if u.page >= self.images.layers || !s.staging.write(off, &l.px) {
                continue;
            }
            regions.push(vk::BufferImageCopy {
                buffer_offset: off,
                buffer_row_length: l.w,
                buffer_image_height: l.h,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: u.page,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D {
                    x: i32::try_from(u.x).unwrap_or(0),
                    y: i32::try_from(u.y).unwrap_or(0),
                    z: 0,
                },
                image_extent: vk::Extent3D {
                    width: l.w,
                    height: l.h,
                    depth: 1,
                },
            });
            off += u64::try_from(l.px.len()).unwrap_or(0);
        }
        if regions.is_empty() {
            return Ok(0);
        }
        let staging = s.staging.buffer;
        let gpu = Arc::clone(&self.gpu);
        barrier(
            &gpu,
            cmd,
            &mut self.images,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        );
        // SAFETY: `cmd` is recording; the staging buffer holds every region (each write above
        // succeeded) and every region lies inside one layer of the live texture array.
        unsafe {
            gpu.device.cmd_copy_buffer_to_image(
                cmd,
                staging,
                self.images.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &regions,
            );
        }
        barrier(
            &gpu,
            cmd,
            &mut self.images,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
        Ok(off - at)
    }

    /// Writes slot `i`'s descriptor sets if a resource changed since.
    pub fn update_sets(&mut self, i: usize) {
        let epoch = self.epoch;
        let Some(s) = self.slots.get(i) else {
            return;
        };
        if s.epoch == epoch {
            return;
        }
        let img = |v: vk::ImageView| {
            [vk::DescriptorImageInfo {
                sampler: self.pipes.sampler,
                image_view: v,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            }]
        };
        let buf = [vk::DescriptorBufferInfo {
            buffer: s.clips.buffer,
            offset: 0,
            range: vk::WHOLE_SIZE,
        }];
        let gi = img(self.glyphs.view);
        let si = img(self.shadows.view);
        let ii = img(self.images.view);
        for (k, set) in s.sets.iter().enumerate() {
            let layer_view = k
                .checked_sub(1)
                .and_then(|depth| self.target.as_ref().and_then(|t| t.layers.get(depth)))
                .map_or(self.dummy.view, |l| l.0.view);
            let li = img(layer_view);
            let w = |b: u32, ty: vk::DescriptorType| {
                vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(b)
                    .descriptor_type(ty)
            };
            let writes = [
                w(0, vk::DescriptorType::STORAGE_BUFFER).buffer_info(&buf),
                w(1, vk::DescriptorType::COMBINED_IMAGE_SAMPLER).image_info(&gi),
                w(2, vk::DescriptorType::COMBINED_IMAGE_SAMPLER).image_info(&si),
                w(3, vk::DescriptorType::COMBINED_IMAGE_SAMPLER).image_info(&li),
                w(4, vk::DescriptorType::COMBINED_IMAGE_SAMPLER).image_info(&ii),
            ];
            // SAFETY: the slot is idle (its fence was waited), every handle is live.
            unsafe { self.gpu.device.update_descriptor_sets(&writes, &[]) };
        }
        if let Some(s) = self.slots.get_mut(i) {
            s.epoch = epoch;
        }
    }

    /// Copies `rect` of an R8 `size`-wide source into `dst` (layer `layer`) through slot `i`'s
    /// staging buffer at `at`; returns the bytes used.
    #[allow(clippy::too_many_arguments)] // one upload's full description; a struct would only rename it
    pub fn upload_r8(
        &mut self,
        i: usize,
        cmd: vk::CommandBuffer,
        at: u64,
        src: &[u8],
        stride: u32,
        rect: PixelRect,
        glyph_layer: Option<u32>,
    ) -> Result<u64> {
        let (Ok(x0), Ok(y0), Ok(w), Ok(h)) = (
            u32::try_from(rect.x0),
            u32::try_from(rect.y0),
            u32::try_from(rect.width()),
            u32::try_from(rect.height()),
        ) else {
            return Ok(0);
        };
        if w == 0 || h == 0 {
            return Ok(0);
        }
        let Some(s) = self.slots.get_mut(i) else {
            return Ok(0);
        };
        for row in 0..h {
            let o = (y0 + row).ix() * stride.ix() + x0.ix();
            let Some(line) = src.get(o..o + w.ix()) else {
                return Ok(0);
            };
            if !s.staging.write(at + u64::from(row) * u64::from(w), line) {
                return Ok(0);
            }
        }
        let staging = s.staging.buffer;
        let gpu = Arc::clone(&self.gpu);
        let dst = match glyph_layer {
            Some(_) => &mut self.glyphs,
            None => &mut self.shadows,
        };
        barrier(&gpu, cmd, dst, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
        let region = vk::BufferImageCopy {
            buffer_offset: at,
            buffer_row_length: w,
            buffer_image_height: h,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: glyph_layer.unwrap_or(0),
                layer_count: 1,
            },
            image_offset: vk::Offset3D {
                x: rect.x0,
                y: rect.y0,
                z: 0,
            },
            image_extent: vk::Extent3D {
                width: w,
                height: h,
                depth: 1,
            },
        };
        // SAFETY: `cmd` is recording; buffer and image are live and the region is in bounds.
        unsafe {
            gpu.device.cmd_copy_buffer_to_image(
                cmd,
                staging,
                dst.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }
        barrier(&gpu, cmd, dst, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        Ok(u64::from(w) * u64::from(h))
    }
}

impl Drop for Live {
    // Releases every Vulkan object made on this device: slots (command buffers go with the
    // pool), textures, the target and its layers, pipelines, then the pools. The `Gpu` itself
    // goes when its last `Arc` does.
    fn drop(&mut self) {
        let _ = self.gpu.idle();
        self.drop_target();
        let gpu = Arc::clone(&self.gpu);
        for s in &mut self.slots {
            s.ring.destroy(&gpu);
            s.clips.destroy(&gpu);
            s.staging.destroy(&gpu);
            // SAFETY: the device is idle; the fence and semaphore belong to it.
            unsafe {
                gpu.device.destroy_fence(s.fence, None);
                gpu.device.destroy_semaphore(s.done, None);
            }
        }
        self.glyphs.destroy(&gpu);
        self.images.destroy(&gpu);
        self.shadows.destroy(&gpu);
        self.dummy.destroy(&gpu);
        self.pipes.destroy(&gpu);
        // SAFETY: the device is idle; nothing allocated from the pools is in use.
        unsafe {
            gpu.device.destroy_descriptor_pool(self.desc_pool, None);
            gpu.device.destroy_command_pool(self.pool, None);
        }
    }
}
