// SPDX-License-Identifier: BSD-2-Clause
//! Pictures (WK-image): decoding a 12-megapixel JPEG (4000 x 3000, p95; whole and capped to a
//! 384-pixel thumbnail) and 200 thumbnails (160 x 120 pictures drawn at 80 x 50, cover, rounded)
//! in one 1280 x 800 frame: the instance build, the GPU frame (engine side, warm atlas) when a
//! Vulkan device opens, and the CPU rasteriser for reference.
//!
//! The JPEG is made once by the system's JPEG encoder command from a synthetic picture into the
//! target directory; without that command the decode rows are skipped with a note.
//!
//! `cargo bench -p wolkit --bench image [-- --check]`; limits in `benches/budgets.toml` `[image]`.

#[path = "w3_support/timing.rs"]
mod timing;
#[path = "w3_support/mod.rs"]
mod w3_support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use wolkit::cpu::raster::{RasterOptions, render};
use wolkit::gpu::select::Preference;
use wolkit::gpu::{Frame, Renderer, RendererOptions};
use wolkit::image::fit::{Fit, dest};
use wolkit::image::{ImageRef, Level, Pixels, decode};
use wolkit::paint::batch::InstanceBuilder;
use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::glyphs::NoGlyphs;
use wolkit::paint::list::{DisplayList, Fill, Radii};

const W: u32 = 1280;
const H: u32 = 800;
const BG: Color = Color::rgb(243, 241, 248);

/// A 4000 x 3000 JPEG at quality 90 with photo-like detail (gradients and noise), made once.
fn jpeg_12mp() -> Option<Vec<u8>> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wolkit-image-bench");
    let jpg = dir.join("12mp.jpg");
    if let Ok(b) = std::fs::read(&jpg) {
        return Some(b);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let (w, h) = (4000u32, 3000u32);
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    let mut x: u32 = 0x9e37_79b9;
    for y in 0..h {
        for i in 0..w {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let n = (x >> 27) as u8;
            ppm.extend_from_slice(&[
                ((i * 255 / w) as u8).wrapping_add(n),
                ((y * 255 / h) as u8).wrapping_add(n),
                (((i / 37 + y / 23) % 2) as u8 * 120).wrapping_add(n),
            ]);
        }
    }
    let src = dir.join("12mp.ppm");
    std::fs::write(&src, ppm).ok()?;
    let ok = std::process::Command::new("cjpeg")
        .args(["-quality", "90", "-outfile"])
        .arg(&jpg)
        .arg(&src)
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&src);
    if ok { std::fs::read(&jpg).ok() } else { None }
}

fn thumb(k: u32) -> Arc<Pixels> {
    let (w, h) = (160u32, 120u32);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[(x + k * 7) as u8, (y + k * 3) as u8, (k * 40) as u8, 255]);
        }
    }
    Arc::new(Pixels::from_level(w, h, Level { w, h, px }))
}

/// The thumbnail grid: 15 columns of 80 x 50 tiles, 4-pixel gaps, on a page.
fn grid(thumbs: &[Arc<Pixels>]) -> DisplayList {
    let mut l = DisplayList::new();
    l.begin_chunk(1, 1, Rect::new(0.0, 0.0, W as f32, H as f32), (0.0, 0.0), 0);
    l.rect(
        Rect::new(0.0, 0.0, W as f32, H as f32),
        Radii::ZERO,
        Fill::Solid(BG),
    );
    for (i, p) in thumbs.iter().enumerate() {
        let (cx, cy) = ((i % 15) as f32, (i / 15) as f32);
        let node = Rect::new(8.0 + cx * 84.0, 8.0 + cy * 54.0, 80.0, 50.0);
        let d = dest(Fit::Cover, node, p.width, p.height);
        l.push_clip(node, 6.0);
        l.image(node, d, ImageRef(Arc::clone(p)), 255);
        l.pop_clip();
    }
    l.finish();
    l
}

fn frame(l: &DisplayList) -> Frame<'_> {
    Frame {
        list: l,
        text: None,
        width: W,
        height: H,
        scale: 1.0,
        clear: BG,
        damage: None,
    }
}

fn pct(v: &mut [f64], p: usize) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get((v.len() * p / 100).min(v.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0)
}

fn main() {
    let mut b = w3_support::Bench::new("image");

    match jpeg_12mp() {
        Some(jpg) => {
            let (_, p95) = b.time("decode_12mp_jpeg_ns", 11, 1, || {
                std::hint::black_box(decode::decode(&jpg, 0).ok());
            });
            b.value("decode_12mp_jpeg_p95_ns", p95);
            let (_, p95) = b.time("decode_12mp_jpeg_thumb_ns", 11, 1, || {
                std::hint::black_box(decode::decode(&jpg, 384).ok());
            });
            b.value("decode_12mp_jpeg_thumb_p95_ns", p95);
        }
        None => println!(
            "bench image.decode_12mp_jpeg skipped: no JPEG encoder command to make the picture"
        ),
    }

    let thumbs: Vec<_> = (0..200).map(thumb).collect();
    let list = grid(&thumbs);

    // The instance build alone (no atlas: places are skipped), cold every time.
    let mut builder = InstanceBuilder::new();
    let mut generation = 0u64;
    b.time("build_200_thumbs_ns", 21, 20, || {
        generation += 1;
        builder.build(&list, &mut NoGlyphs, 1.0, generation);
    });

    let mut opts = RendererOptions::default();
    opts.device.preference = Preference {
        prefer_software: std::env::var_os("WOLKIT_GPU_HARDWARE").is_none(),
        ..Preference::default()
    };
    match Renderer::new(opts) {
        Ok(mut r) => {
            // First frame: places and uploads every thumbnail.
            let t = Instant::now();
            let _ = r.render_image(frame(&list));
            b.value("gpu_200_thumbs_first_ns", t.elapsed().as_nanos() as f64);
            b.value("gpu_200_thumbs_upload_bytes", r.stats.upload_bytes as f64);
            b.value("gpu_200_thumbs_draws", f64::from(r.stats.draws));
            // Steady state: a new list each frame (as a scrolling grid would), atlas warm.
            let mut cpu = Vec::new();
            for k in 0..120u64 {
                let mut l = grid(&thumbs);
                for c in &mut l.chunks {
                    c.version = 10 + k;
                }
                if r.render_submit(frame(&l)).is_err() {
                    break;
                }
                cpu.push(r.stats.record_ns.saturating_sub(r.stats.wait_ns) as f64);
            }
            let _ = r.wait_idle();
            b.value("gpu_200_thumbs_cpu_p95_ns", pct(&mut cpu, 95));
            b.value(
                "gpu_200_thumbs_upload_steady_bytes",
                r.stats.upload_bytes as f64,
            );
        }
        Err(e) => println!("bench image.gpu skipped: {e}"),
    }

    // For reference: the CPU rasteriser draws the grid.
    b.time("cpu_200_thumbs_ns", 5, 1, || {
        std::hint::black_box(render(
            &list,
            &mut NoGlyphs,
            W,
            H,
            &RasterOptions {
                clear: BG,
                ..RasterOptions::default()
            },
        ));
    });
    b.finish();
}
