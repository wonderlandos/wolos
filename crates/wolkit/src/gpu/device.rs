// SPDX-License-Identifier: BSD-2-Clause
//! The Vulkan instance and device: loading, device choice ([`super::select`]), memory, and the
//! buffer and image wrappers the renderer is built from.
//!
//! `libvulkan.so.1` is loaded at run time (ash `loaded`), so a machine without Vulkan still runs
//! every app on the CPU rasteriser. The instance asks for the Wayland surface extensions when the
//! loader has them; the device asks for the swapchain and incremental-present extensions when the
//! driver has them. Every `unsafe` block in this file is a Vulkan call on handles this module
//! created and owns; the `// SAFETY:` line names the extra condition, if any.

use std::ffi::{CStr, c_char};

use ash::vk;

use super::copy;
use super::select::{Blocklist, DeviceFacts, DeviceKind, Preference, choose};
use crate::error::{Error, Result, Status};

/// Maps a Vulkan error to an engine error. A lost device gets its own status so the engine
/// knows to fall back and recover.
pub fn vk_err(r: vk::Result) -> Error {
    match r {
        vk::Result::ERROR_DEVICE_LOST => Error::new(Status::DeviceLost, copy::DEVICE_LOST),
        vk::Result::ERROR_OUT_OF_DEVICE_MEMORY | vk::Result::ERROR_OUT_OF_HOST_MEMORY => {
            Error::new(Status::Limit, copy::OUT_OF_MEMORY)
        }
        vk::Result::ERROR_SURFACE_LOST_KHR | vk::Result::ERROR_NATIVE_WINDOW_IN_USE_KHR => {
            Error::unavailable(copy::NO_SURFACE)
        }
        _ => Error::new(Status::Unavailable, copy::DRIVER),
    }
}

/// Instance-level facts the surface API needs before any device exists.
pub const SURFACE_INSTANCE_EXTENSIONS: [&CStr; 2] =
    [vk::KHR_SURFACE_NAME, vk::KHR_WAYLAND_SURFACE_NAME];

/// The loaded instance and the chosen device.
pub struct Gpu {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
    pub physical: vk::PhysicalDevice,
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub family: u32,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    pub props: vk::PhysicalDeviceProperties,
    pub facts: DeviceFacts,
    /// The instance has VK_KHR_surface and VK_KHR_wayland_surface.
    pub wayland: bool,
    /// The device has VK_KHR_swapchain.
    pub swapchain: bool,
    /// The device has VK_KHR_incremental_present.
    pub incremental_present: bool,
}

/// The device half of a [`Gpu`], before it is joined with its instance.
struct Opened {
    physical: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
    props: vk::PhysicalDeviceProperties,
    facts: DeviceFacts,
    swapchain: bool,
    incremental_present: bool,
}

/// What [`Gpu::new`] is told by its caller (the engine reads the environment, not this file).
#[derive(Clone, Debug, Default)]
pub struct DeviceOptions {
    pub preference: Preference,
    /// The blocklist file's text, if one is installed.
    pub blocklist: Option<String>,
}

fn has(list: &[vk::ExtensionProperties], name: &CStr) -> bool {
    list.iter()
        .any(|e| e.extension_name_as_c_str().is_ok_and(|n| n == name))
}

fn kind_of(t: vk::PhysicalDeviceType) -> DeviceKind {
    match t {
        vk::PhysicalDeviceType::INTEGRATED_GPU => DeviceKind::Integrated,
        vk::PhysicalDeviceType::DISCRETE_GPU => DeviceKind::Discrete,
        vk::PhysicalDeviceType::VIRTUAL_GPU => DeviceKind::Virtual,
        vk::PhysicalDeviceType::CPU => DeviceKind::Software,
        _ => DeviceKind::Other,
    }
}

fn node(has: vk::Bool32, major: i64, minor: i64) -> Option<(u32, u32)> {
    if has != vk::TRUE {
        return None;
    }
    Some((u32::try_from(major).ok()?, u32::try_from(minor).ok()?))
}

/// What the uber-pipeline needs from a device: R8 sampled images, RGBA8 and BGRA8 colour
/// attachments with blending, and a graphics queue.
fn suitable(instance: &ash::Instance, pd: vk::PhysicalDevice) -> Option<u32> {
    // SAFETY: `pd` came from this instance's enumeration.
    let (families, r8, rgba) = unsafe {
        (
            instance.get_physical_device_queue_family_properties(pd),
            instance.get_physical_device_format_properties(pd, vk::Format::R8_UNORM),
            instance.get_physical_device_format_properties(pd, vk::Format::R8G8B8A8_UNORM),
        )
    };
    let blend =
        vk::FormatFeatureFlags::COLOR_ATTACHMENT_BLEND | vk::FormatFeatureFlags::SAMPLED_IMAGE;
    if !r8
        .optimal_tiling_features
        .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
        || !rgba.optimal_tiling_features.contains(blend)
    {
        return None;
    }
    families
        .iter()
        .position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        .and_then(|i| u32::try_from(i).ok())
}

impl Gpu {
    /// Loads Vulkan, picks a device by the policy in [`super::select`] and opens it.
    pub fn new(opts: &DeviceOptions) -> Result<Gpu> {
        // SAFETY: loading the system loader runs its initialisers; it is the documented entry.
        let entry =
            unsafe { ash::Entry::load() }.map_err(|_| Error::unavailable(copy::NO_VULKAN))?;
        // SAFETY: plain query on the loaded entry.
        let inst_exts =
            unsafe { entry.enumerate_instance_extension_properties(None) }.map_err(vk_err)?;
        let wayland = SURFACE_INSTANCE_EXTENSIONS
            .iter()
            .all(|n| has(&inst_exts, n));
        let mut names: Vec<*const c_char> = Vec::new();
        if wayland {
            names.extend(SURFACE_INSTANCE_EXTENSIONS.iter().map(|n| n.as_ptr()));
        }
        let app = vk::ApplicationInfo::default()
            .application_name(c"libwolkit")
            .engine_name(c"libwolkit")
            .api_version(vk::make_api_version(0, 1, 1, 0));
        let info = vk::InstanceCreateInfo::default()
            .application_info(&app)
            .enabled_extension_names(&names);
        // SAFETY: `info` and the names it points at outlive the call.
        let instance = unsafe { entry.create_instance(&info, None) }.map_err(vk_err)?;
        match Gpu::open(&instance, wayland, opts) {
            Ok(o) => Ok(Gpu {
                entry,
                instance,
                physical: o.physical,
                device: o.device,
                queue: o.queue,
                family: o.family,
                memory: o.memory,
                props: o.props,
                facts: o.facts,
                wayland,
                swapchain: o.swapchain,
                incremental_present: o.incremental_present,
            }),
            Err(e) => {
                // SAFETY: nothing was created from the instance that is still alive.
                unsafe { instance.destroy_instance(None) };
                Err(e)
            }
        }
    }

    fn facts(
        instance: &ash::Instance,
        pd: vk::PhysicalDevice,
        index: usize,
    ) -> (DeviceFacts, Vec<vk::ExtensionProperties>, Option<u32>) {
        // SAFETY: `pd` came from this instance's enumeration; the chained struct lives on the stack.
        let (props, exts) = unsafe {
            (
                instance.get_physical_device_properties(pd),
                instance
                    .enumerate_device_extension_properties(pd)
                    .unwrap_or_default(),
            )
        };
        let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
        if has(&exts, vk::EXT_PHYSICAL_DEVICE_DRM_NAME) {
            let mut p2 = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
            // SAFETY: core 1.1 query; `p2` chains only `drm`, which outlives the call.
            unsafe { instance.get_physical_device_properties2(pd, &mut p2) };
        }
        let family = suitable(instance, pd);
        let name = props
            .device_name_as_c_str()
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_default();
        let facts = DeviceFacts {
            index,
            name,
            vendor: props.vendor_id,
            device: props.device_id,
            driver_version: props.driver_version,
            kind: kind_of(props.device_type),
            render_node: node(drm.has_render, drm.render_major, drm.render_minor),
            primary_node: node(drm.has_primary, drm.primary_major, drm.primary_minor),
            suitable: family.is_some(),
        };
        (facts, exts, family)
    }

    fn open(instance: &ash::Instance, wayland: bool, opts: &DeviceOptions) -> Result<Opened> {
        // SAFETY: plain query on a live instance.
        let pds = unsafe { instance.enumerate_physical_devices() }.map_err(vk_err)?;
        let all: Vec<_> = pds
            .iter()
            .enumerate()
            .map(|(i, pd)| Gpu::facts(instance, *pd, i))
            .collect();
        let facts: Vec<DeviceFacts> = all.iter().map(|a| a.0.clone()).collect();
        let blocklist = Blocklist::with_extra(opts.blocklist.as_deref());
        let Some(chosen) = choose(&facts, &opts.preference, &blocklist) else {
            let blocked = facts.iter().any(|f| f.suitable && blocklist.blocks(f));
            let msg = if blocked {
                copy::BLOCKED
            } else {
                copy::NO_DEVICE
            };
            return Err(Error::unavailable(msg));
        };
        let i = chosen.index;
        let (facts, exts, family) = all[i].clone();
        let (Some(family), Some(&physical)) = (family, pds.get(i)) else {
            return Err(Error::unavailable(copy::NO_DEVICE));
        };
        let swapchain = wayland && has(&exts, vk::KHR_SWAPCHAIN_NAME);
        let incremental_present = swapchain && has(&exts, vk::KHR_INCREMENTAL_PRESENT_NAME);
        let mut dev_exts: Vec<*const c_char> = Vec::new();
        if swapchain {
            dev_exts.push(vk::KHR_SWAPCHAIN_NAME.as_ptr());
        }
        if incremental_present {
            dev_exts.push(vk::KHR_INCREMENTAL_PRESENT_NAME.as_ptr());
        }
        let prio = [1.0f32];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&prio)];
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&dev_exts);
        // SAFETY: `physical` belongs to `instance`; `info` and its arrays outlive the call.
        let created = unsafe {
            instance.create_device(physical, &info, None).map(|d| {
                let q = d.get_device_queue(family, 0);
                let m = instance.get_physical_device_memory_properties(physical);
                let p = instance.get_physical_device_properties(physical);
                (d, q, m, p)
            })
        };
        let (device, queue, memory, props) = created.map_err(vk_err)?;
        Ok(Opened {
            physical,
            device,
            queue,
            family,
            memory,
            props,
            facts,
            swapchain,
            incremental_present,
        })
    }

    /// A memory type index with `bits` allowed and `flags` set.
    pub fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Option<u32> {
        let n = self.memory.memory_type_count.min(32);
        (0..n).find(|&i| {
            bits & (1 << i) != 0
                && self
                    .memory
                    .memory_types
                    .get(usize::try_from(i).unwrap_or(usize::MAX))
                    .is_some_and(|t| t.property_flags.contains(flags))
        })
    }

    /// The largest square image side the device can draw into.
    pub fn max_extent(&self) -> u32 {
        self.props.limits.max_image_dimension2_d
    }

    /// Waits until the device is idle; a lost device is reported.
    pub fn idle(&self) -> Result<()> {
        // SAFETY: plain call on a live device.
        unsafe { self.device.device_wait_idle() }.map_err(vk_err)
    }
}

impl Drop for Gpu {
    // Releases the device, then the instance (everything created from them is dropped first by
    // the owners in `frame.rs`, which hold an `Arc<Gpu>`).
    fn drop(&mut self) {
        // SAFETY: the last `Arc<Gpu>` is gone, so no object created from the device is alive.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

/// A buffer with its memory, mapped for its whole life when host-visible.
pub struct Buffer {
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub size: u64,
    pub mapped: Option<std::ptr::NonNull<u8>>,
}

// SAFETY: the mapping is only written by the thread that owns the renderer (one thread at a
// time per engine, wolkit.h); moving the owner between threads is fine.
unsafe impl Send for Buffer {}

impl Buffer {
    /// A host-visible, coherent buffer mapped for writing.
    pub fn mapped(gpu: &Gpu, size: u64, usage: vk::BufferUsageFlags) -> Result<Buffer> {
        let size = size.max(256);
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let d = &gpu.device;
        // SAFETY: calls on a live device; every object made here is destroyed on the error paths
        // and in `destroy`.
        unsafe {
            let buffer = d.create_buffer(&info, None).map_err(vk_err)?;
            let req = d.get_buffer_memory_requirements(buffer);
            let flags =
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let Some(ty) = gpu.memory_type(req.memory_type_bits, flags) else {
                d.destroy_buffer(buffer, None);
                return Err(Error::new(Status::Limit, copy::OUT_OF_MEMORY));
            };
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(ty);
            let memory = match d.allocate_memory(&alloc, None) {
                Ok(m) => m,
                Err(e) => {
                    d.destroy_buffer(buffer, None);
                    return Err(vk_err(e));
                }
            };
            let mapped = d.bind_buffer_memory(buffer, memory, 0).and_then(|()| {
                d.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            });
            match mapped {
                Ok(p) => Ok(Buffer {
                    buffer,
                    memory,
                    size,
                    mapped: std::ptr::NonNull::new(p.cast::<u8>()),
                }),
                Err(e) => {
                    d.destroy_buffer(buffer, None);
                    d.free_memory(memory, None);
                    Err(vk_err(e))
                }
            }
        }
    }

    /// Copies `bytes` into the mapping at `offset`; false when it does not fit.
    pub fn write(&mut self, offset: u64, bytes: &[u8]) -> bool {
        let end = offset.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        let (Some(base), true) = (self.mapped, end <= self.size) else {
            return false;
        };
        let Ok(off) = usize::try_from(offset) else {
            return false;
        };
        // SAFETY: `[offset, end)` lies inside the mapped allocation (checked above), the mapping
        // is live until `destroy`, and the GPU does not read this range now (the frame fence of
        // its slot was waited for by the caller).
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), base.as_ptr().add(off), bytes.len());
        }
        true
    }

    /// Reads `out.len()` bytes from the mapping at `offset` (readback).
    pub fn read(&self, offset: u64, out: &mut [u8]) -> bool {
        let end = offset.saturating_add(u64::try_from(out.len()).unwrap_or(u64::MAX));
        let (Some(base), true) = (self.mapped, end <= self.size) else {
            return false;
        };
        let Ok(off) = usize::try_from(offset) else {
            return false;
        };
        // SAFETY: in bounds (checked), mapped, and the GPU finished writing (fence waited).
        unsafe {
            std::ptr::copy_nonoverlapping(base.as_ptr().add(off), out.as_mut_ptr(), out.len());
        }
        true
    }

    pub fn destroy(&mut self, gpu: &Gpu) {
        // SAFETY: the owner waited for the GPU to stop using the buffer.
        unsafe {
            gpu.device.destroy_buffer(self.buffer, None);
            gpu.device.free_memory(self.memory, None);
        }
        self.buffer = vk::Buffer::null();
        self.memory = vk::DeviceMemory::null();
        self.mapped = None;
    }
}

/// A device-local image with one view.
pub struct Image {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub format: vk::Format,
    /// The layout the image is in after the last recorded command.
    pub layout: vk::ImageLayout,
}

impl Image {
    /// Null handles (destroying them is a no-op), for fields filled in later.
    pub fn placeholder() -> Image {
        Image {
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            width: 0,
            height: 0,
            layers: 1,
            format: vk::Format::UNDEFINED,
            layout: vk::ImageLayout::UNDEFINED,
        }
    }

    /// A `w x h` image of `layers` layers; `array` makes the view a 2D array (a sampler2DArray).
    pub fn new(
        gpu: &Gpu,
        (w, h, layers): (u32, u32, u32),
        array: bool,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
    ) -> Result<Image> {
        let max = gpu.max_extent();
        if w == 0 || h == 0 || w > max || h > max {
            return Err(Error::limit(copy::TOO_LARGE));
        }
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: w,
                height: h,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(layers.max(1))
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let d = &gpu.device;
        // SAFETY: calls on a live device; partial objects are destroyed on every error path.
        unsafe {
            let image = d.create_image(&info, None).map_err(vk_err)?;
            let req = d.get_image_memory_requirements(image);
            let Some(ty) = gpu
                .memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .or_else(|| {
                    gpu.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::empty())
                })
            else {
                d.destroy_image(image, None);
                return Err(Error::new(Status::Limit, copy::OUT_OF_MEMORY));
            };
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(ty);
            let memory = match d.allocate_memory(&alloc, None) {
                Ok(m) => m,
                Err(e) => {
                    d.destroy_image(image, None);
                    return Err(vk_err(e));
                }
            };
            if let Err(e) = d.bind_image_memory(image, memory, 0) {
                d.destroy_image(image, None);
                d.free_memory(memory, None);
                return Err(vk_err(e));
            }
            let view_type = if array {
                vk::ImageViewType::TYPE_2D_ARRAY
            } else {
                vk::ImageViewType::TYPE_2D
            };
            let vinfo = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(view_type)
                .format(format)
                .subresource_range(color_range(layers.max(1)));
            let view = match d.create_image_view(&vinfo, None) {
                Ok(v) => v,
                Err(e) => {
                    d.destroy_image(image, None);
                    d.free_memory(memory, None);
                    return Err(vk_err(e));
                }
            };
            Ok(Image {
                image,
                memory,
                view,
                width: w,
                height: h,
                layers: layers.max(1),
                format,
                layout: vk::ImageLayout::UNDEFINED,
            })
        }
    }

    pub fn destroy(&mut self, gpu: &Gpu) {
        // SAFETY: the owner waited for the GPU to stop using the image.
        unsafe {
            gpu.device.destroy_image_view(self.view, None);
            gpu.device.destroy_image(self.image, None);
            gpu.device.free_memory(self.memory, None);
        }
        self.image = vk::Image::null();
        self.view = vk::ImageView::null();
        self.memory = vk::DeviceMemory::null();
    }
}

pub fn color_range(layers: u32) -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: layers,
    }
}

/// Records a layout transition with the usual access masks for each layout.
pub fn barrier(gpu: &Gpu, cmd: vk::CommandBuffer, img: &mut Image, to: vk::ImageLayout) {
    let access = |l: vk::ImageLayout| match l {
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => (
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::COLOR_ATTACHMENT_READ,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        ),
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL => (
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        ),
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL => (
            vk::AccessFlags::TRANSFER_READ,
            vk::PipelineStageFlags::TRANSFER,
        ),
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => (
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TRANSFER,
        ),
        vk::ImageLayout::PRESENT_SRC_KHR => (
            vk::AccessFlags::empty(),
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
        ),
        _ => (
            vk::AccessFlags::empty(),
            vk::PipelineStageFlags::TOP_OF_PIPE,
        ),
    };
    let (sa, ss) = access(img.layout);
    let (da, ds) = access(to);
    let b = vk::ImageMemoryBarrier::default()
        .src_access_mask(sa)
        .dst_access_mask(da)
        .old_layout(img.layout)
        .new_layout(to)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(img.image)
        .subresource_range(color_range(img.layers));
    // SAFETY: `cmd` is recording and `img` is live.
    unsafe {
        gpu.device
            .cmd_pipeline_barrier(cmd, ss, ds, vk::DependencyFlags::empty(), &[], &[], &[b]);
    }
    img.layout = to;
}
