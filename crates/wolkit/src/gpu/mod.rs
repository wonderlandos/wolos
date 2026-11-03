// SPDX-License-Identifier: BSD-2-Clause
//! gpu: the Vulkan renderer (plan section 2.6, owner WK-W5; ADR 0039).
//!
//! One uber-pipeline draws every primitive as an instanced 64-byte quad (`paint::batch`): rounded
//! rectangles, borders and 2-stop gradients are signed distance functions in the fragment shader,
//! glyphs are quads from the R8 glyph atlas (a texture array), shadows are nine-sliced prototypes
//! from an R8 shadow atlas, pictures are quads sampling premultiplied RGBA8 atlas pages
//! ([`images`]), clips are rows of a table the shader indexes (axis-aligned and one rounded clip
//! per instance), and group opacity is pushed into the leaves' colours; only groups whose
//! children overlap get an offscreen layer. A typical window is a few draw calls.
//!
//! The shader evaluates each primitive the way the CPU reference rasteriser does (`cpu::raster`),
//! so `tests/gpu_golden.rs` holds the GPU image of every golden scene within 0.1% of pixels
//! differing by at most 2 per channel, on a software Vulkan device in CI.
//!
//! Files: [`select`] (device policy, blocklist), [`device`] (instance, device, buffers, images),
//! [`pipelines`] (pipeline, render passes, pipeline cache), [`live`] (per-device state, frame
//! slots, the persistent instance ring), [`frame`] (the renderer, damage, layers, readback,
//! device-lost recovery), [`present`] (Wayland swapchain, incremental present), [`surface`]
//! (`wolkit_surface_*` Vulkan surfaces for apps), [`shadow`] (nine-slice masks), [`images`] (the
//! picture atlas), [`canvas`] (the canvas op stream), [`copy`] (sentences). `unsafe` is confined
//! to `device`, `pipelines`, `live`, `frame`, `present` and `surface`; every block names its
//! invariant.

pub mod canvas;
pub mod copy;
pub mod device;
pub mod frame;
pub mod images;
pub mod live;
pub mod pipelines;
pub mod present;
pub mod select;
pub mod shadow;
pub mod surface;

pub use frame::{Format, Frame, FrameStats, Renderer, RendererOptions};

/// Side of the shadow prototype atlas, in pixels (R8, 1 MiB).
pub const SHADOW_ATLAS: u32 = 1024;
/// `Instance::params` bit: the rectangle is in absolute window pixels (shadow masks, composites).
pub const ABSOLUTE: u32 = 1 << 10;
/// Instance kind for compositing an offscreen layer (after `paint::batch::kind::GLYPH`).
pub const K_LAYER: u32 = 5;

/// `f32` to `u32`, saturating (NaN and negatives give 0). RR-017 allows only the `u64` cast.
pub fn f2u(f: f32) -> u32 {
    u32::try_from(f.max(0.0) as u64).unwrap_or(u32::MAX)
}

/// `f32` to `i32`, truncating toward zero like `as`, saturating.
pub fn f2i(f: f32) -> i32 {
    if f >= 0.0 {
        i32::try_from(f as u64).unwrap_or(i32::MAX)
    } else {
        i32::try_from((-f) as u64).map_or(i32::MIN, |v| -v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_conversions_truncate_and_saturate() {
        assert_eq!(f2u(3.9), 3);
        assert_eq!(f2u(-2.0), 0);
        assert_eq!(f2u(f32::NAN), 0);
        assert_eq!(f2u(1.0e12), u32::MAX);
        assert_eq!(f2i(-3.9), -3);
        assert_eq!(f2i(7.5), 7);
        assert_eq!(f2i(-1.0e12), i32::MIN);
    }
}
