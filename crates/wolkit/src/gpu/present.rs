// SPDX-License-Identifier: BSD-2-Clause
//! Presenting through the Vulkan Wayland surface extension (plan section 2.6, v1).
//!
//! The renderer draws into its own persistent target, so a frame only redraws its damage. The
//! swapchain image then receives the damage of every frame since that image was last shown
//! (buffer age, tracked here per image index), copied from the target, and the present carries
//! only this frame's rectangle as an incremental-present region, which the compositor turns into
//! `wl_surface.damage_buffer`. A frame with no damage is not presented at all (budget 10: idle
//! damage is 0). Out-of-date and suboptimal swapchains are rebuilt on the next frame; a lost
//! device rebuilds the surface on the new device.

use std::ffi::c_void;
use std::sync::Arc;

use ash::vk;

use super::copy;
use super::device::{Gpu, color_range, vk_err};
use super::frame::{Frame, Renderer};
use super::live::SLOTS;
use crate::error::{Error, Result, Status};
use crate::paint::geom::PixelRect;

/// Frames of damage remembered for buffer age (older images are copied whole).
pub const AGE_DEPTH: usize = 4;

/// What [`Presenter::present`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presented {
    /// Nothing changed; no image was acquired or presented.
    Skipped,
    /// Presented; the bytes copied into the swapchain image.
    Shown { copied_px: u64, age: u32 },
}

struct Chain {
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    /// Frame number each image was last written in (0: never, its content is undefined).
    written: Vec<u64>,
    extent: vk::Extent2D,
}

pub struct Presenter {
    display: *mut c_void,
    wl_surface: *mut c_void,
    gpu: Option<Arc<Gpu>>,
    surface_fn: Option<ash::khr::surface::Instance>,
    swap_fn: Option<ash::khr::swapchain::Device>,
    surface: vk::SurfaceKHR,
    chain: Option<Chain>,
    acquire: Vec<vk::Semaphore>,
    /// Scissors of the last frames, newest last.
    history: Vec<PixelRect>,
    frame: u64,
    size: (u32, u32),
    stale: bool,
    pub vsync: bool,
}

// SAFETY: the raw Wayland pointers are only passed to Vulkan on the thread that owns the engine
// (one thread at a time per engine, wolkit.h); the presenter never dereferences them.
unsafe impl Send for Presenter {}

fn union(a: PixelRect, b: PixelRect) -> PixelRect {
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

/// What to copy into an image of age `age` (frames since it was written; 0 = never): the union
/// of this frame's damage and the `age - 1` frames before it, or everything.
pub fn copy_region(history: &[PixelRect], now: PixelRect, age: u64, full: PixelRect) -> PixelRect {
    let older = usize::try_from(age.saturating_sub(1)).unwrap_or(usize::MAX);
    if age == 0 || older > history.len() {
        return full;
    }
    let mut r = now;
    for h in history.iter().rev().take(older) {
        r = union(r, *h);
    }
    r.intersect(&full)
}

impl Presenter {
    /// A presenter for a Wayland surface. Nothing Vulkan is made until the first frame.
    ///
    /// # Safety
    /// `display` is a live `wl_display *` and `wl_surface` a live `wl_surface *` of it, and both
    /// outlive the presenter.
    pub unsafe fn new(
        display: *mut c_void,
        wl_surface: *mut c_void,
        width: u32,
        height: u32,
    ) -> Presenter {
        Presenter {
            display,
            wl_surface,
            gpu: None,
            surface_fn: None,
            swap_fn: None,
            surface: vk::SurfaceKHR::null(),
            chain: None,
            acquire: Vec::new(),
            history: Vec::new(),
            frame: 0,
            size: (width, height),
            stale: true,
            vsync: true,
        }
    }

    /// The window was resized (device pixels): the swapchain is rebuilt before the next frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.size != (width, height) {
            self.size = (width, height);
            self.stale = true;
        }
    }

    fn teardown(&mut self) {
        let Some(gpu) = self.gpu.take() else {
            return;
        };
        let _ = gpu.idle();
        // SAFETY: the device is idle; every handle was made on it or on its instance.
        unsafe {
            if let (Some(sf), Some(c)) = (self.swap_fn.as_ref(), self.chain.take()) {
                sf.destroy_swapchain(c.swapchain, None);
            }
            for s in self.acquire.drain(..) {
                gpu.device.destroy_semaphore(s, None);
            }
            if let Some(f) = self.surface_fn.as_ref() {
                f.destroy_surface(self.surface, None);
            }
        }
        self.surface = vk::SurfaceKHR::null();
        self.surface_fn = None;
        self.swap_fn = None;
        self.history.clear();
    }

    /// Makes the surface on `gpu` (first use, or a new device after a loss).
    fn attach(&mut self, gpu: &Arc<Gpu>) -> Result<()> {
        if self.gpu.as_ref().is_some_and(|g| Arc::ptr_eq(g, gpu)) {
            return Ok(());
        }
        self.teardown();
        if !gpu.swapchain {
            return Err(Error::unavailable(copy::NO_SURFACE));
        }
        let wl = ash::khr::wayland_surface::Instance::new(&gpu.entry, &gpu.instance);
        let info = vk::WaylandSurfaceCreateInfoKHR::default()
            .display(self.display)
            .surface(self.wl_surface);
        // SAFETY: `new`'s contract makes both Wayland pointers live; the instance has the
        // Wayland surface extension (`gpu.swapchain` implies it).
        let surface = unsafe { wl.create_wayland_surface(&info, None) }.map_err(vk_err)?;
        let surface_fn = ash::khr::surface::Instance::new(&gpu.entry, &gpu.instance);
        // SAFETY: query on live handles.
        let ok = unsafe {
            surface_fn.get_physical_device_surface_support(gpu.physical, gpu.family, surface)
        };
        let mut sems = Vec::new();
        for _ in 0..SLOTS {
            // SAFETY: call on a live device.
            match unsafe {
                gpu.device
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
            } {
                Ok(s) => sems.push(s),
                Err(e) => {
                    // SAFETY: the semaphores and the surface were made just above.
                    unsafe {
                        for s in sems {
                            gpu.device.destroy_semaphore(s, None);
                        }
                        surface_fn.destroy_surface(surface, None);
                    }
                    return Err(vk_err(e));
                }
            }
        }
        self.surface = surface;
        self.acquire = sems;
        self.swap_fn = Some(ash::khr::swapchain::Device::new(&gpu.instance, &gpu.device));
        self.surface_fn = Some(surface_fn);
        self.gpu = Some(Arc::clone(gpu));
        self.stale = true;
        if !ok.unwrap_or(false) {
            self.teardown();
            return Err(Error::unavailable(copy::NO_SURFACE));
        }
        Ok(())
    }

    fn rebuild(&mut self, format: vk::Format) -> Result<()> {
        let (Some(gpu), Some(sf), Some(swf)) = (
            self.gpu.clone(),
            self.surface_fn.as_ref(),
            self.swap_fn.as_ref(),
        ) else {
            return Err(Error::unavailable(copy::NO_SURFACE));
        };
        gpu.idle()?;
        // SAFETY: queries on live handles.
        let (caps, formats, modes) = unsafe {
            (
                sf.get_physical_device_surface_capabilities(gpu.physical, self.surface)
                    .map_err(vk_err)?,
                sf.get_physical_device_surface_formats(gpu.physical, self.surface)
                    .map_err(vk_err)?,
                sf.get_physical_device_surface_present_modes(gpu.physical, self.surface)
                    .unwrap_or_default(),
            )
        };
        let Some(fmt) = formats.iter().find(|f| f.format == format) else {
            return Err(Error::unavailable(copy::NO_SURFACE));
        };
        let extent = if caps.current_extent.width == u32::MAX {
            vk::Extent2D {
                width: self.size.0.clamp(
                    caps.min_image_extent.width.max(1),
                    caps.max_image_extent.width.max(1),
                ),
                height: self.size.1.clamp(
                    caps.min_image_extent.height.max(1),
                    caps.max_image_extent.height.max(1),
                ),
            }
        } else {
            caps.current_extent
        };
        let alpha = if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED)
        {
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED
        } else {
            vk::CompositeAlphaFlagsKHR::OPAQUE
        };
        let mode = if !self.vsync && modes.contains(&vk::PresentModeKHR::MAILBOX) {
            vk::PresentModeKHR::MAILBOX
        } else {
            vk::PresentModeKHR::FIFO
        };
        let count = if caps.max_image_count == 0 {
            caps.min_image_count.max(3)
        } else {
            caps.min_image_count.max(3).min(caps.max_image_count)
        };
        let old = self
            .chain
            .as_ref()
            .map_or(vk::SwapchainKHR::null(), |c| c.swapchain);
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(count)
            .image_format(fmt.format)
            .image_color_space(fmt.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(alpha)
            .present_mode(mode)
            .clipped(true)
            .old_swapchain(old);
        // SAFETY: the device is idle, so the old swapchain's images are not in use.
        let (swapchain, images) = unsafe {
            let s = swf.create_swapchain(&info, None).map_err(vk_err)?;
            if old != vk::SwapchainKHR::null() {
                swf.destroy_swapchain(old, None);
            }
            (s, swf.get_swapchain_images(s).map_err(vk_err)?)
        };
        self.chain = Some(Chain {
            swapchain,
            written: vec![0; images.len()],
            images,
            extent,
        });
        self.history.clear();
        self.stale = false;
        Ok(())
    }

    /// Draws `frame` with `renderer` and presents it.
    pub fn present(&mut self, renderer: &mut Renderer, frame: Frame<'_>) -> Result<Presented> {
        let format = renderer.format().vk();
        let (slot, scissor) = renderer.render_target(frame)?;
        let loss = renderer.take_simulated_loss();
        let r = self.finish(renderer, slot, scissor, format, loss);
        if let Err(e) = &r
            && e.status == Status::DeviceLost
        {
            self.teardown();
            renderer.lose_now();
        }
        r
    }

    fn finish(
        &mut self,
        renderer: &mut Renderer,
        slot: usize,
        scissor: PixelRect,
        format: vk::Format,
        loss: bool,
    ) -> Result<Presented> {
        let Some(gpu) = renderer.gpu().cloned() else {
            return Err(Error::unavailable(copy::NO_DEVICE));
        };
        self.attach(&gpu)?;
        let Some(live) = renderer.live_mut() else {
            return Err(Error::unavailable(copy::NO_DEVICE));
        };
        if scissor.is_empty() {
            live.submit(slot, &[], &[], false)?;
            return Ok(Presented::Skipped);
        }
        if self.stale || self.chain.is_none() {
            self.rebuild(format)?;
        }
        let (Some(swf), Some(chain)) = (self.swap_fn.as_ref(), self.chain.as_mut()) else {
            return Err(Error::unavailable(copy::NO_SURFACE));
        };
        let sem = self.acquire.get(slot).copied().unwrap_or_default();
        // SAFETY: live swapchain; the semaphore is unsignalled (its last wait was by the submit
        // of this slot, whose fence the renderer waited for before recording).
        let acquired = unsafe {
            swf.acquire_next_image(chain.swapchain, 1_000_000_000, sem, vk::Fence::null())
        };
        let index = match acquired {
            Ok((i, suboptimal)) => {
                self.stale |= suboptimal;
                i
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.stale = true;
                live.submit(slot, &[], &[], false)?;
                return Ok(Presented::Skipped);
            }
            Err(e) => return Err(vk_err(e)),
        };
        let iu = usize::try_from(index).unwrap_or(0);
        self.frame += 1;
        let full = PixelRect {
            x0: 0,
            y0: 0,
            x1: i32::try_from(chain.extent.width).unwrap_or(0),
            y1: i32::try_from(chain.extent.height).unwrap_or(0),
        };
        let written = chain.written.get(iu).copied().unwrap_or(0);
        let age = if written == 0 {
            0
        } else {
            self.frame - written
        };
        let region = copy_region(&self.history, scissor, age, full);
        let image = chain.images.get(iu).copied().unwrap_or_default();
        let cmd = live.slots.get(slot).map(|s| s.cmd).unwrap_or_default();
        let old = if written == 0 {
            vk::ImageLayout::UNDEFINED
        } else {
            vk::ImageLayout::PRESENT_SRC_KHR
        };
        copy_to_swap(live, cmd, image, old, region);
        if loss {
            return Err(vk_err(vk::Result::ERROR_DEVICE_LOST));
        }
        live.submit(slot, &[sem], &[vk::PipelineStageFlags::TRANSFER], true)?;
        if let Some(w) = chain.written.get_mut(iu) {
            *w = self.frame;
        }
        self.history.push(scissor);
        if self.history.len() > AGE_DEPTH {
            self.history.remove(0);
        }
        let done = live.slots.get(slot).map(|s| s.done).unwrap_or_default();
        let rects = [vk::RectLayerKHR {
            offset: vk::Offset2D {
                x: scissor.x0,
                y: scissor.y0,
            },
            extent: vk::Extent2D {
                width: u32::try_from(scissor.width()).unwrap_or(0),
                height: u32::try_from(scissor.height()).unwrap_or(0),
            },
            layer: 0,
        }];
        let regions = [vk::PresentRegionKHR::default().rectangles(&rects)];
        let mut incr = vk::PresentRegionsKHR::default().regions(&regions);
        let waits = [done];
        let chains = [chain.swapchain];
        let indices = [index];
        let mut info = vk::PresentInfoKHR::default()
            .wait_semaphores(&waits)
            .swapchains(&chains)
            .image_indices(&indices);
        if gpu.incremental_present {
            info = info.push_next(&mut incr);
        }
        // SAFETY: the queue supports present on this surface (checked in `attach`); the image was
        // acquired and the submit that writes it signals `done`.
        match unsafe { swf.queue_present(gpu.queue, &info) } {
            Ok(suboptimal) => self.stale |= suboptimal,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.stale = true,
            Err(e) => return Err(vk_err(e)),
        }
        Ok(Presented::Shown {
            copied_px: region.area(),
            age: u32::try_from(age).unwrap_or(u32::MAX),
        })
    }
}

/// Records the copy of `region` from the target into a swapchain image and leaves the image
/// ready to present.
fn copy_to_swap(
    live: &mut super::live::Live,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    old: vk::ImageLayout,
    region: PixelRect,
) {
    let gpu = Arc::clone(&live.gpu);
    let Some(t) = live.target.as_mut() else {
        return;
    };
    super::device::barrier(
        &gpu,
        cmd,
        &mut t.image,
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
    );
    let to_dst = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(old)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(color_range(1));
    let to_present = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::empty())
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(color_range(1));
    let layers = vk::ImageSubresourceLayers {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
    };
    let off = vk::Offset3D {
        x: region.x0,
        y: region.y0,
        z: 0,
    };
    let copy = vk::ImageCopy {
        src_subresource: layers,
        src_offset: off,
        dst_subresource: layers,
        dst_offset: off,
        extent: vk::Extent3D {
            width: u32::try_from(region.width()).unwrap_or(0),
            height: u32::try_from(region.height()).unwrap_or(0),
            depth: 1,
        },
    };
    let d = &gpu.device;
    // SAFETY: `cmd` is recording outside a render pass; the target and the acquired image are
    // live, the region lies inside both (the target has the swapchain's size).
    unsafe {
        d.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_dst],
        );
        d.cmd_copy_image(
            cmd,
            t.image.image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[copy],
        );
        d.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_present],
        );
    }
    super::device::barrier(
        &gpu,
        cmd,
        &mut t.image,
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    );
}

impl Drop for Presenter {
    // Releases the swapchain, the acquire semaphores and the Vulkan surface (the Wayland
    // objects belong to the native window).
    fn drop(&mut self) {
        self.teardown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: i32, y0: i32, x1: i32, y1: i32) -> PixelRect {
        PixelRect { x0, y0, x1, y1 }
    }

    #[test]
    fn buffer_age_copies_the_damage_of_every_missed_frame() {
        let full = r(0, 0, 100, 100);
        let hist = [r(0, 0, 10, 10), r(50, 50, 60, 60), r(90, 0, 100, 5)];
        let now = r(20, 20, 30, 30);
        assert_eq!(
            copy_region(&hist, now, 1, full),
            now,
            "age 1: only this frame"
        );
        assert_eq!(copy_region(&hist, now, 2, full), r(20, 0, 100, 30));
        assert_eq!(copy_region(&hist, now, 3, full), r(20, 0, 100, 60));
        assert_eq!(copy_region(&hist, now, 0, full), full, "never written");
        assert_eq!(
            copy_region(&hist, now, 9, full),
            full,
            "older than the history"
        );
    }
}
