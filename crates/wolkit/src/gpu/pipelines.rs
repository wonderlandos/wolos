// SPDX-License-Identifier: BSD-2-Clause
//! The uber-pipeline (one graphics pipeline for every primitive), its layouts and render passes,
//! and the on-disk pipeline cache.
//!
//! The SPIR-V is compiled from `shaders/ui.{vert,frag}` by `shaders/build.sh` and checked in;
//! `tests/gpu_shaders.rs` fails when a source no longer matches `shaders/spv.lock`. The pipeline
//! cache lives in a file the engine names (under `$XDG_CACHE_HOME/wolkit/`); it is wrapped in a
//! checksum and its Vulkan header is checked against this device before the driver sees it, so a
//! cache from another GPU or a truncated file is ignored, never trusted.

use std::io::Write;
use std::path::{Path, PathBuf};

use ash::vk;

use super::device::{Gpu, vk_err};
use crate::error::{Error, Result};
use crate::text::cache::{FNV_OFFSET, hash_bytes};

pub const VERT_SPV: &[u8] = include_bytes!("../../shaders/ui.vert.spv");
pub const FRAG_SPV: &[u8] = include_bytes!("../../shaders/ui.frag.spv");

/// Bytes per instance (`paint::batch::Instance`).
pub const INSTANCE_STRIDE: u32 = 64;
/// Push constants: chunk offset (vec2) and target size (vec2).
pub const PUSH_BYTES: u32 = 16;
/// Nested offscreen layers the GPU path keeps (deeper groups fold into their parent).
pub const MAX_LAYERS: usize = 4;

const CACHE_MAGIC: &[u8; 4] = b"WKPC";
const CACHE_VERSION: u32 = 1;
/// Largest pipeline cache file read back.
pub const MAX_CACHE_BYTES: u64 = 32 << 20;

fn words(spv: &[u8]) -> Result<Vec<u32>> {
    ash::util::read_spv(&mut std::io::Cursor::new(spv))
        .map_err(|_| Error::new(crate::error::Status::Internal, super::copy::DRIVER))
}

/// Checks a cache blob's Vulkan header (`VkPipelineCacheHeaderVersionOne`) against `props`.
pub fn cache_header_matches(data: &[u8], props: &vk::PhysicalDeviceProperties) -> bool {
    let u = |o: usize| {
        data.get(o..o + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    };
    u(0).is_some_and(|len| len >= 32)
        && u(4) == Some(1)
        && u(8) == Some(props.vendor_id)
        && u(12) == Some(props.device_id)
        && data.get(16..32) == Some(&props.pipeline_cache_uuid[..])
}

/// Wraps a driver cache blob for disk.
pub fn wrap_cache(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 24);
    out.extend_from_slice(CACHE_MAGIC);
    out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    out.extend_from_slice(&hash_bytes(FNV_OFFSET, data).to_le_bytes());
    out.extend_from_slice(&u64::try_from(data.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// The driver blob inside a file written by [`wrap_cache`], if the file is intact.
pub fn unwrap_cache(file: &[u8]) -> Option<&[u8]> {
    if file.get(..4)? != CACHE_MAGIC || file.get(4..8)? != CACHE_VERSION.to_le_bytes() {
        return None;
    }
    let sum = u64::from_le_bytes(file.get(8..16)?.try_into().ok()?);
    let len = usize::try_from(u64::from_le_bytes(file.get(16..24)?.try_into().ok()?)).ok()?;
    let data = file.get(24..)?;
    (data.len() == len && hash_bytes(FNV_OFFSET, data) == sum).then_some(data)
}

/// Reads a cache file for `props`; anything missing, foreign or damaged gives `None`.
pub fn read_cache(path: &Path, props: &vk::PhysicalDeviceProperties) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_CACHE_BYTES {
        return None;
    }
    let file = std::fs::read(path).ok()?;
    let data = unwrap_cache(&file)?;
    cache_header_matches(data, props).then(|| data.to_vec())
}

/// Writes the cache atomically (a temporary file in the same directory, then a rename), 0600.
pub fn write_cache(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp: PathBuf = dir.join(format!(".pipeline-cache.{}.part", std::process::id()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(&wrap_cache(data))?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// Layouts, render passes and the pipeline for one colour format.
pub struct Pipelines {
    pub format: vk::Format,
    pub set_layout: vk::DescriptorSetLayout,
    pub layout: vk::PipelineLayout,
    /// Clears the target first (`UNDEFINED` in, colour attachment out).
    pub pass_clear: vk::RenderPass,
    /// Keeps the target's pixels (colour attachment in and out).
    pub pass_load: vk::RenderPass,
    pub pipeline: vk::Pipeline,
    pub cache: vk::PipelineCache,
    pub sampler: vk::Sampler,
    /// The cache came from disk (a warm start).
    pub cache_hit: bool,
}

fn pass(gpu: &Gpu, format: vk::Format, clear: bool) -> Result<vk::RenderPass> {
    let att = [vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(if clear {
            vk::AttachmentLoadOp::CLEAR
        } else {
            vk::AttachmentLoadOp::LOAD
        })
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(if clear {
            vk::ImageLayout::UNDEFINED
        } else {
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
        })
        .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
    let refs = [vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    }];
    let sub = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&refs)];
    let dep = [vk::SubpassDependency {
        src_subpass: vk::SUBPASS_EXTERNAL,
        dst_subpass: 0,
        src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
            | vk::PipelineStageFlags::FRAGMENT_SHADER
            | vk::PipelineStageFlags::TRANSFER,
        dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
            | vk::AccessFlags::SHADER_READ
            | vk::AccessFlags::TRANSFER_READ,
        dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
            | vk::AccessFlags::COLOR_ATTACHMENT_READ,
        dependency_flags: vk::DependencyFlags::empty(),
    }];
    let info = vk::RenderPassCreateInfo::default()
        .attachments(&att)
        .subpasses(&sub)
        .dependencies(&dep);
    // SAFETY: call on a live device; the arrays outlive it.
    unsafe { gpu.device.create_render_pass(&info, None) }.map_err(vk_err)
}

fn attrs() -> [vk::VertexInputAttributeDescription; 8] {
    let a = |location: u32, format: vk::Format, offset: u32| vk::VertexInputAttributeDescription {
        location,
        binding: 0,
        format,
        offset,
    };
    [
        a(0, vk::Format::R32G32B32A32_SFLOAT, 0),
        a(1, vk::Format::R32G32B32A32_SFLOAT, 16),
        a(2, vk::Format::R32_UINT, 32),
        a(3, vk::Format::R32_UINT, 36),
        a(4, vk::Format::R16G16B16A16_UINT, 40),
        a(5, vk::Format::R16G16B16A16_UINT, 48),
        a(6, vk::Format::R32_UINT, 56),
        a(7, vk::Format::R32_UINT, 60),
    ]
}

impl Pipelines {
    /// Builds everything for `format`, seeding the pipeline cache from `cache_file` if usable.
    pub fn new(gpu: &Gpu, format: vk::Format, cache_file: Option<&Path>) -> Result<Pipelines> {
        let mut p = Pipelines {
            format,
            set_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            pass_clear: vk::RenderPass::null(),
            pass_load: vk::RenderPass::null(),
            pipeline: vk::Pipeline::null(),
            cache: vk::PipelineCache::null(),
            sampler: vk::Sampler::null(),
            cache_hit: false,
        };
        match p.build(gpu, cache_file) {
            Ok(()) => Ok(p),
            Err(e) => {
                p.destroy(gpu);
                Err(e)
            }
        }
    }

    fn build(&mut self, gpu: &Gpu, cache_file: Option<&Path>) -> Result<()> {
        let d = &gpu.device;
        let seed = cache_file.and_then(|f| read_cache(f, &gpu.props));
        self.cache_hit = seed.is_some();
        let seed = seed.unwrap_or_default();
        let binding = |b: u32, ty: vk::DescriptorType| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(b)
                .descriptor_type(ty)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT)
        };
        let bindings = [
            binding(0, vk::DescriptorType::STORAGE_BUFFER),
            binding(1, vk::DescriptorType::COMBINED_IMAGE_SAMPLER),
            binding(2, vk::DescriptorType::COMBINED_IMAGE_SAMPLER),
            binding(3, vk::DescriptorType::COMBINED_IMAGE_SAMPLER),
        ];
        let push = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            offset: 0,
            size: PUSH_BYTES,
        }];
        let vert = words(VERT_SPV)?;
        let frag = words(FRAG_SPV)?;
        self.pass_clear = pass(gpu, self.format, true)?;
        self.pass_load = pass(gpu, self.format, false)?;
        // SAFETY: calls on a live device; every create-info and array outlives its call, and
        // every object is recorded in `self` at once so `destroy` frees it on any later error.
        unsafe {
            self.set_layout = d
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(vk_err)?;
            let sets = [self.set_layout];
            self.layout = d
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&sets)
                        .push_constant_ranges(&push),
                    None,
                )
                .map_err(vk_err)?;
            self.sampler = d
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::NEAREST)
                        .min_filter(vk::Filter::NEAREST)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(vk_err)?;
            self.cache = d
                .create_pipeline_cache(
                    &vk::PipelineCacheCreateInfo::default().initial_data(&seed),
                    None,
                )
                .or_else(|_| d.create_pipeline_cache(&vk::PipelineCacheCreateInfo::default(), None))
                .map_err(vk_err)?;
            let vs = d
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vert), None)
                .map_err(vk_err)?;
            let fs = match d
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&frag), None)
            {
                Ok(m) => m,
                Err(e) => {
                    d.destroy_shader_module(vs, None);
                    return Err(vk_err(e));
                }
            };
            let made = self.pipeline(gpu, vs, fs);
            d.destroy_shader_module(vs, None);
            d.destroy_shader_module(fs, None);
            self.pipeline = made?;
        }
        Ok(())
    }

    fn pipeline(
        &self,
        gpu: &Gpu,
        vs: vk::ShaderModule,
        fs: vk::ShaderModule,
    ) -> Result<vk::Pipeline> {
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vs)
                .name(c"main"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fs)
                .name(c"main"),
        ];
        let vb = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: INSTANCE_STRIDE,
            input_rate: vk::VertexInputRate::INSTANCE,
        }];
        let at = attrs();
        let vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vb)
            .vertex_attribute_descriptions(&at);
        let ia = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let vp = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rs = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // Premultiplied source-over, the CPU rasteriser's `over`.
        let blend = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let cb = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
        let dyns = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let ds = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dyns);
        let info = [vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vi)
            .input_assembly_state(&ia)
            .viewport_state(&vp)
            .rasterization_state(&rs)
            .multisample_state(&ms)
            .color_blend_state(&cb)
            .dynamic_state(&ds)
            .layout(self.layout)
            .render_pass(self.pass_load)
            .subpass(0)];
        // SAFETY: call on a live device; all state structs and arrays outlive it.
        let made = unsafe {
            gpu.device
                .create_graphics_pipelines(self.cache, &info, None)
        };
        match made {
            Ok(p) => p
                .first()
                .copied()
                .ok_or_else(|| vk_err(vk::Result::ERROR_UNKNOWN)),
            Err((_, e)) => Err(vk_err(e)),
        }
    }

    /// The driver's cache bytes, for [`write_cache`].
    pub fn cache_data(&self, gpu: &Gpu) -> Vec<u8> {
        // SAFETY: call on a live device with this device's cache.
        unsafe { gpu.device.get_pipeline_cache_data(self.cache) }.unwrap_or_default()
    }

    pub fn destroy(&mut self, gpu: &Gpu) {
        let d = &gpu.device;
        // SAFETY: the owner waited for the GPU to go idle; null handles are ignored by Vulkan.
        unsafe {
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_cache(self.cache, None);
            d.destroy_render_pass(self.pass_clear, None);
            d.destroy_render_pass(self.pass_load, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
            d.destroy_sampler(self.sampler, None);
        }
        self.pipeline = vk::Pipeline::null();
        self.cache = vk::PipelineCache::null();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props() -> vk::PhysicalDeviceProperties {
        vk::PhysicalDeviceProperties {
            vendor_id: 0x10005,
            device_id: 0,
            pipeline_cache_uuid: [7; 16],
            ..Default::default()
        }
    }

    fn header(p: &vk::PhysicalDeviceProperties) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&32u32.to_le_bytes());
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&p.vendor_id.to_le_bytes());
        h.extend_from_slice(&p.device_id.to_le_bytes());
        h.extend_from_slice(&p.pipeline_cache_uuid);
        h.extend_from_slice(b"driver data");
        h
    }

    #[test]
    fn cache_round_trips_and_rejects_damage() {
        let p = props();
        let blob = header(&p);
        let file = wrap_cache(&blob);
        assert_eq!(unwrap_cache(&file), Some(&blob[..]));
        let mut bad = file.clone();
        if let Some(b) = bad.last_mut() {
            *b ^= 1;
        }
        assert_eq!(unwrap_cache(&bad), None, "checksum");
        assert_eq!(unwrap_cache(&file[..file.len() - 1]), None, "truncated");
        assert!(cache_header_matches(&blob, &p));
        let mut other = p;
        other.pipeline_cache_uuid = [8; 16];
        assert!(!cache_header_matches(&blob, &other), "another driver build");
    }

    #[test]
    fn cache_file_is_private_and_atomic() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("wolkit-gpu-cache-{}", std::process::id()));
        let path = dir.join("pipeline.bin");
        let p = props();
        assert!(write_cache(&path, &header(&p)).is_ok());
        let mode = std::fs::metadata(&path)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0);
        assert_eq!(mode, 0o600);
        assert_eq!(read_cache(&path, &p), Some(header(&p)));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn embedded_spirv_is_well_formed() {
        assert!(words(VERT_SPV).is_ok_and(|w| w.first() == Some(&0x0723_0203)));
        assert!(words(FRAG_SPV).is_ok_and(|w| w.first() == Some(&0x0723_0203)));
    }
}
