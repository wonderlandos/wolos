// SPDX-License-Identifier: BSD-2-Clause
//! GPU goldens: every CPU golden scene drawn by the Vulkan renderer must match the CPU reference
//! within the plan's tolerance (section 2.6): no channel of any pixel differs by more than 2, and
//! at most 0.1% of pixels differ by more than 1. Differences of exactly 1 are not counted against
//! the 0.1%: they come from the blend unit's rounding of `d * (1 - a)` (lavapipe blends 8-bit
//! targets in fixed point; float blenders round it exactly), and they are reported per scene.
//!
//! CI runs this on a software Vulkan device (the forge's lavapipe). Without a Vulkan loader or
//! device the tests pass with a note, unless `WOLKIT_GPU_REQUIRED=1` (the forge lane sets it), in
//! which case a missing device is a failure. `WOLKIT_GPU_HARDWARE=1` prefers a hardware device.

#[path = "golden/png.rs"]
mod png;
#[path = "golden/scenes.rs"]
mod scenes;
#[path = "support/text.rs"]
mod text_support;

use std::path::PathBuf;

use scenes::{BG, Ctx, Scene};
use wolkit::cpu::raster::{Image, RasterOptions, render};
use wolkit::gpu::select::Preference;
use wolkit::gpu::{Frame, Renderer, RendererOptions};
use wolkit::paint::list::DisplayList;
use wolkit::text::glyphcache::Glyphs;

/// Share of pixels that may differ by more than 1.
pub const MAX_DIFF_SHARE: f64 = 0.001;
/// Largest difference of one channel.
pub const MAX_CHANNEL_DIFF: u8 = 2;

fn renderer() -> Option<Renderer> {
    let mut opts = RendererOptions::default();
    opts.device.preference = Preference {
        prefer_software: std::env::var_os("WOLKIT_GPU_HARDWARE").is_none(),
        ..Preference::default()
    };
    match Renderer::new(opts) {
        Ok(r) => Some(r),
        Err(e) => {
            assert!(
                std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(),
                "WOLKIT_GPU_REQUIRED is set but no Vulkan device opened: {e}"
            );
            println!("note: no Vulkan device ({e}); GPU goldens skipped");
            None
        }
    }
}

fn cpu(s: &Scene, ctx: &mut Ctx) -> Image {
    let mut list = DisplayList::new();
    (s.build)(ctx, &mut list);
    list.finish();
    let mut src = Glyphs {
        db: &ctx.f.db,
        cache: &mut ctx.glyphs,
    };
    render(
        &list,
        &mut src,
        s.width,
        s.height,
        &RasterOptions {
            scale: s.scale,
            clear: BG,
            tile: 64,
        },
    )
}

fn gpu(r: &mut Renderer, s: &Scene, ctx: &mut Ctx) -> Image {
    r.invalidate();
    let mut list = DisplayList::new();
    (s.build)(ctx, &mut list);
    list.finish();
    let frame = Frame {
        list: &list,
        text: Some((&ctx.f.db, &mut ctx.glyphs)),
        width: s.width,
        height: s.height,
        scale: s.scale,
        clear: BG,
        damage: None,
    };
    match r.render_image(frame) {
        Ok(img) => img,
        Err(e) => panic!("{}: the GPU renderer refused the scene: {e}", s.name),
    }
}

/// `(pixels that differ at all, pixels that differ by more than 1, largest channel difference)`.
pub fn compare(a: &Image, b: &Image) -> (usize, usize, u8) {
    let (mut any, mut over1, mut worst) = (0, 0, 0u8);
    for (p, q) in a
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data.as_chunks::<4>().0)
    {
        let d = p
            .iter()
            .zip(q)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if d > 0 {
            any += 1;
            worst = worst.max(d);
        }
        if d > 1 {
            over1 += 1;
        }
    }
    (any, over1, worst)
}

/// The `n` most different pixels, for the failure message.
fn first_diffs(got: &Image, want: &Image, n: usize) -> String {
    let mut all: Vec<(u8, usize, [u8; 4], [u8; 4])> = Vec::new();
    for (i, (p, q)) in got
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(want.data.as_chunks::<4>().0)
        .enumerate()
    {
        let d = p
            .iter()
            .zip(q)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if d > 1 {
            all.push((d, i, *p, *q));
        }
    }
    all.sort_by_key(|d| std::cmp::Reverse(d.0));
    let w = usize::try_from(got.width).unwrap_or(1);
    all.iter()
        .take(n)
        .map(|(d, i, p, q)| format!("  ({}, {}) off by {d}: gpu {p:?} cpu {q:?}", i % w, i / w))
        .collect::<Vec<_>>()
        .join("\n")
}

fn out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gpu-golden")
}

#[test]
fn gpu_matches_the_cpu_reference_within_tolerance() {
    let Some(mut r) = renderer() else {
        return;
    };
    let mut failures = Vec::new();
    let mut report = Vec::new();
    for s in scenes::all() {
        let want = cpu(&s, &mut Ctx::new());
        let got = gpu(&mut r, &s, &mut Ctx::new());
        assert_eq!(
            (got.width, got.height),
            (want.width, want.height),
            "{}",
            s.name
        );
        let (n, over1, worst) = compare(&got, &want);
        let total = (want.width * want.height) as f64;
        let share = over1 as f64 / total;
        report.push(format!(
            "{}: {n} of {total} pixels differ ({:.4}%), {over1} by more than 1 ({:.4}%), worst channel {worst}, draws {}",
            s.name,
            n as f64 / total * 100.0,
            share * 100.0,
            r.stats.draws
        ));
        if share > MAX_DIFF_SHARE || worst > MAX_CHANNEL_DIFF {
            report.push(first_diffs(&got, &want, 6));
            let dir = out_dir();
            std::fs::create_dir_all(&dir).unwrap_or_default();
            let path = dir.join(format!("{}.png", s.name));
            std::fs::write(
                &path,
                png::encode(got.width, got.height, &got.to_straight()),
            )
            .unwrap_or_default();
            failures.push(format!(
                "{} (actual at {})",
                report.last().cloned().unwrap_or_default(),
                path.display()
            ));
        }
    }
    println!("{}", report.join("\n"));
    assert!(
        failures.is_empty(),
        "GPU goldens out of tolerance:\n{}",
        failures.join("\n")
    );
}

#[test]
fn warm_caches_and_repeated_frames_give_the_same_image() {
    let Some(mut r) = renderer() else {
        return;
    };
    let mut ctx = Ctx::new();
    for s in scenes::all() {
        let a = gpu(&mut r, &s, &mut ctx);
        let b = gpu(&mut r, &s, &mut ctx);
        assert_eq!(a, b, "{}: a second frame changed pixels", s.name);
    }
}
