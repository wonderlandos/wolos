// SPDX-License-Identifier: BSD-2-Clause
//! The renderer trait against the CPU reference: every golden scene of wolkit
//! (`crates/wolkit/tests/golden/scenes.rs`, the same display lists the Vulkan goldens draw) drawn
//! by each GPU implementation must stay within the GPU golden tolerance of the CPU image: no
//! channel off by more than 2, at most 0.1% of pixels off by more than 1.
//!
//! The wgpu implementation runs on a software Vulkan adapter (lavapipe) unless
//! `WOLKIT_GPU_HARDWARE=1`. Without an adapter the tests pass with a note, unless
//! `WOLKIT_GPU_REQUIRED=1`, which makes a missing adapter a failure (the forge lane sets it).

#[path = "support/png.rs"]
mod png;
#[path = "support/scenes.rs"]
mod scenes;
#[path = "support/text_support.rs"]
mod text_support;

use scenes::{BG, Ctx, Scene};
use wolkit::paint::list::DisplayList;
use wolkit_render::cpu::CpuRenderer;
use wolkit_render::wgpu_backend::WgpuRenderer;
use wolkit_render::{FrameIn, Image, Renderer, diff};

fn wgpu() -> Option<WgpuRenderer> {
    match WgpuRenderer::new_native(std::env::var_os("WOLKIT_GPU_HARDWARE").is_some()) {
        Ok(r) => Some(r),
        Err(e) => {
            assert!(
                std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(),
                "WOLKIT_GPU_REQUIRED is set but no wgpu adapter opened: {e}"
            );
            println!("note: no wgpu adapter ({e}); skipped");
            None
        }
    }
}

fn draw(r: &mut dyn Renderer, s: &Scene, ctx: &mut Ctx) -> Image {
    let mut list = DisplayList::new();
    (s.build)(ctx, &mut list);
    list.finish();
    r.render_image(FrameIn {
        list: &list,
        text: Some((&ctx.f.db, &mut ctx.glyphs)),
        width: s.width,
        height: s.height,
        scale: s.scale,
        clear: BG,
    })
    .unwrap_or_else(|e| panic!("{} on {}: {e}", s.name, r.name()))
}

fn out_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wgpu-golden")
}

fn assert_held_to_cpu(r: &mut dyn Renderer) {
    let mut fails = Vec::new();
    let _ = std::fs::create_dir_all(out_dir());
    for s in scenes::all() {
        let want = draw(&mut CpuRenderer, &s, &mut Ctx::new());
        let got = draw(r, &s, &mut Ctx::new());
        assert_eq!(
            (got.width, got.height),
            (want.width, want.height),
            "{}",
            s.name
        );
        let d = diff(&got.data, &want.data);
        println!(
            "{} {}: {} of {} pixels differ, {} by more than 1, worst {}",
            r.name(),
            s.name,
            d.any,
            d.pixels,
            d.over1,
            d.worst
        );
        if !d.within_tolerance() {
            let png = png::encode(got.width, got.height, &got.to_straight());
            let _ = std::fs::write(out_dir().join(format!("{}.png", s.name)), png);
            fails.push(format!("{}: {d:?}", s.name));
        }
    }
    assert!(
        fails.is_empty(),
        "{} outside tolerance:\n{}",
        r.name(),
        fails.join("\n")
    );
}

#[test]
fn the_cpu_renderer_is_the_reference() {
    // The trait's CPU implementation is the rasteriser itself: identical to the bit-exact goldens.
    for s in scenes::all() {
        let a = draw(&mut CpuRenderer, &s, &mut Ctx::new());
        let b = draw(&mut CpuRenderer, &s, &mut Ctx::new());
        assert_eq!(diff(&a.data, &b.data).any, 0, "{}", s.name);
    }
}

#[test]
fn wgpu_matches_the_cpu_reference_within_tolerance() {
    let Some(mut r) = wgpu() else {
        return;
    };
    assert_held_to_cpu(&mut r);
}

#[test]
fn wgpu_reuses_its_buffers_across_frames() {
    let Some(mut r) = wgpu() else {
        return;
    };
    // The same renderer draws every scene twice in a row; the second frame equals the first
    // (chunk cache, atlases and layers carried across frames, sizes changing between scenes).
    let mut ctx = Ctx::new();
    for s in scenes::all() {
        let a = draw(&mut r, &s, &mut ctx);
        let b = draw(&mut r, &s, &mut ctx);
        assert_eq!(diff(&a.data, &b.data).any, 0, "{}", s.name);
    }
}

#[cfg(feature = "native")]
#[test]
fn the_vulkan_renderer_behind_the_trait_matches_too() {
    let mut opts = wolkit::gpu::RendererOptions::default();
    opts.device.preference.prefer_software = std::env::var_os("WOLKIT_GPU_HARDWARE").is_none();
    match wolkit_render::vulkan::VulkanRenderer::new(opts) {
        Ok(mut r) => assert_held_to_cpu(&mut r),
        Err(e) => {
            assert!(std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(), "{e}");
            println!("note: no Vulkan device ({e}); skipped");
        }
    }
}
