// SPDX-License-Identifier: BSD-2-Clause
//! wolkit-render: libWolkit's render targets behind one trait (ADR 0060, "wolkit on WebGPU
//! and WASM"; docs/modules/libwolkit.md, section "Render targets").
//!
//! Everything above the display list is the same for every target: the retained scene, style,
//! layout, text shaping (harfrust + skrifa, wolkit's own glyph rasteriser and atlas) and the
//! lowering to a [`DisplayList`]. GPU targets also share the instance batcher
//! (`wolkit::paint::batch`) and the frame preparation (`wolkit::gpu::prepare`), so a target only
//! uploads instances, clips and atlas pages and records its passes. The implementations:
//!
//! * [`cpu::CpuRenderer`]: the reference rasteriser (`wolkit::cpu::raster`); the oracle of every
//!   golden and the software path of the trait tests.
//! * [`vulkan::VulkanRenderer`] (feature `native`): the existing ash renderer (`wolkit::gpu`).
//! * [`wgpu_backend::WgpuRenderer`]: wgpu with a WGSL port of the uber-shader. Natively it runs
//!   on Vulkan (lavapipe in CI); in `wasm32-unknown-unknown` the same code runs on the browser's
//!   WebGPU and draws into a `<canvas>` (`crates/wolkit-web`).
//!
//! [`view`] raises a `shared::view` tree (what libwol apps send and the shell's Kit host draws in
//! GTK) into a wolkit scene, and [`page::Page`] is a window without a display: scene, kit look,
//! tokens, input in, display list out. The web page and the native tests use the same `Page`.

#![forbid(unsafe_code)]

pub mod cpu;
pub mod page;
pub mod view;
#[cfg(feature = "native")]
pub mod vulkan;
pub mod wgpu_backend;

pub use wolkit::cpu::raster::Image;
use wolkit::paint::geom::Color;
use wolkit::paint::list::DisplayList;
use wolkit::text::fontdb::FontDb;
use wolkit::text::glyphcache::GlyphCache;

/// One frame for a renderer: the display list, its fonts and glyph cache, the target size in
/// device pixels, the scale and the clear colour (the window's surface token).
pub struct FrameIn<'a> {
    pub list: &'a DisplayList,
    pub text: Option<(&'a FontDb, &'a mut GlyphCache)>,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub clear: Color,
}

/// Why a renderer could not draw: no adapter, a device error, a refused size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

/// A render target: a display list in, premultiplied RGBA8 pixels out. Every implementation is
/// held to the CPU reference within [`MAX_DIFF_SHARE`] and [`MAX_CHANNEL_DIFF`].
pub trait Renderer {
    /// A short name for reports (`cpu`, `vulkan`, `wgpu-vulkan`, `wgpu-webgpu`).
    fn name(&self) -> String;
    /// Draws the frame and reads it back (premultiplied RGBA8, `width * height * 4` bytes).
    fn render_image(&mut self, frame: FrameIn<'_>) -> Result<Image, RenderError>;
}

/// Share of pixels that may differ from the CPU reference by more than 1 (the Vulkan renderer's
/// tolerance, `crates/wolkit/tests/gpu_golden.rs`).
pub const MAX_DIFF_SHARE: f64 = 0.001;
/// Largest difference of one channel from the CPU reference.
pub const MAX_CHANNEL_DIFF: u8 = 2;

/// How two images differ: pixels that differ at all, pixels that differ by more than 1, and the
/// largest channel difference. Images of different sizes count every pixel of the larger one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub pixels: usize,
    pub any: usize,
    pub over1: usize,
    pub worst: u8,
}

impl Diff {
    /// Within the GPU golden tolerance.
    pub fn within_tolerance(&self) -> bool {
        self.worst <= MAX_CHANNEL_DIFF
            && (self.over1 as f64) <= (self.pixels as f64) * MAX_DIFF_SHARE
    }
}

/// Compares two RGBA8 buffers of the same layout.
pub fn diff(a: &[u8], b: &[u8]) -> Diff {
    let mut d = Diff {
        pixels: a.len().max(b.len()) / 4,
        ..Diff::default()
    };
    if a.len() != b.len() {
        d.any = d.pixels;
        d.over1 = d.pixels;
        d.worst = 255;
        return d;
    }
    let (a, b) = (a.as_chunks::<4>().0, b.as_chunks::<4>().0);
    for (p, q) in a.iter().zip(b) {
        let m = p
            .iter()
            .zip(q)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if m > 0 {
            d.any += 1;
            d.worst = d.worst.max(m);
        }
        if m > 1 {
            d.over1 += 1;
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_counts_and_tolerates() {
        let a = vec![10u8; 4000];
        let mut b = a.clone();
        assert_eq!(
            diff(&a, &b),
            Diff {
                pixels: 1000,
                ..Diff::default()
            }
        );
        b[0] = 11;
        b[4] = 12;
        let d = diff(&a, &b);
        assert_eq!((d.any, d.over1, d.worst), (2, 1, 2));
        assert!(d.within_tolerance());
        b[8] = 13;
        assert!(!diff(&a, &b).within_tolerance(), "a channel off by 3");
        assert!(!diff(&a, &a[..400]).within_tolerance(), "sizes differ");
    }
}
