// SPDX-License-Identifier: BSD-2-Clause
//! Frame behaviour of the GPU renderer on a (software) Vulkan device: resize and scale changes,
//! damage-only redraws, idle frames, device loss and recovery, the pipeline cache, and canvases.
//! Every image is held against the CPU rasteriser with the golden tolerance (`gpu_golden.rs`).
//! Without a device the tests pass with a note unless `WOLKIT_GPU_REQUIRED=1`.

use wolkit::cpu::raster::{Image, RasterOptions, render};
use wolkit::error::Status;
use wolkit::gpu::canvas::{Canvas, Encoder};
use wolkit::gpu::select::Preference;
use wolkit::gpu::{Frame, Renderer, RendererOptions};
use wolkit::paint::damage::Damage;
use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::glyphs::NoGlyphs;
use wolkit::paint::list::{DisplayList, Fill, Radii, Shadow};

const BG: Color = Color::rgb(243, 241, 248);

fn opts() -> RendererOptions {
    let mut o = RendererOptions::default();
    o.device.preference = Preference {
        prefer_software: std::env::var_os("WOLKIT_GPU_HARDWARE").is_none(),
        ..Preference::default()
    };
    o
}

fn renderer() -> Option<Renderer> {
    match Renderer::new(opts()) {
        Ok(r) => Some(r),
        Err(e) => {
            assert!(
                std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(),
                "WOLKIT_GPU_REQUIRED is set but no Vulkan device opened: {e}"
            );
            println!("note: no Vulkan device ({e}); skipped");
            None
        }
    }
}

/// A card window whose `hover` row is highlighted; chunk version = 1 + hover.
fn window(w: f32, h: f32, hover: u32) -> DisplayList {
    let mut l = DisplayList::new();
    l.begin_chunk(
        1,
        u64::from(hover) + 1,
        Rect::new(0.0, 0.0, w, h),
        (0.0, 0.0),
        0,
    );
    l.rect(
        Rect::new(0.0, 0.0, w, 40.0),
        Radii::ZERO,
        Fill::vertical(Color::rgb(255, 255, 255), Color::rgb(236, 236, 242)),
    );
    l.shadow(
        Rect::new(16.0, 56.0, w - 32.0, h - 72.0),
        Radii::uniform(10.0),
        Shadow::new(12.0, 0.0, 0.0, 4.0, Color::rgba(20, 20, 60, 90)),
    );
    l.rect(
        Rect::new(16.0, 56.0, w - 32.0, h - 72.0),
        Radii::uniform(10.0),
        Fill::Solid(Color::rgb(255, 255, 255)),
    );
    l.push_clip(Rect::new(16.0, 56.0, w - 32.0, h - 72.0), 10.0);
    for i in 0..8u32 {
        let y = 60.0 + i as f32 * 24.0;
        let c = if i == hover {
            Color::rgb(124, 92, 255)
        } else {
            Color::rgb(240, 241, 247)
        };
        l.rect(
            Rect::new(20.0, y, w - 40.0, 20.0),
            Radii::uniform(4.0),
            Fill::Solid(c),
        );
    }
    l.pop_clip();
    l.border(
        Rect::new(16.0, 56.0, w - 32.0, h - 72.0),
        Radii::uniform(10.0),
        1.0,
        Color::rgb(214, 216, 226),
    );
    l.finish();
    l
}

fn cpu(l: &DisplayList, w: u32, h: u32, scale: f32) -> Image {
    render(
        l,
        &mut NoGlyphs,
        w,
        h,
        &RasterOptions {
            scale,
            clear: BG,
            ..RasterOptions::default()
        },
    )
}

fn gpu(
    r: &mut Renderer,
    l: &DisplayList,
    w: u32,
    h: u32,
    scale: f32,
    damage: Option<&Damage>,
) -> Image {
    let f = Frame {
        list: l,
        text: None,
        width: w,
        height: h,
        scale,
        clear: BG,
        damage,
    };
    match r.render_image(f) {
        Ok(i) => i,
        Err(e) => panic!("frame refused: {e}"),
    }
}

/// Golden tolerance: no channel off by more than 2, at most 0.1% of pixels off by more than 1.
fn within(a: &Image, b: &Image, what: &str) {
    assert_eq!((a.width, a.height), (b.width, b.height), "{what}: size");
    let (mut over1, mut worst) = (0usize, 0u8);
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
        worst = worst.max(d);
        over1 += usize::from(d > 1);
    }
    let share = over1 as f64 / f64::from(a.width * a.height);
    assert!(
        worst <= 2 && share <= 0.001,
        "{what}: worst {worst}, {over1} pixels off by more than 1"
    );
}

#[test]
fn resize_and_scale_change_match_the_cpu_at_every_size() {
    let Some(mut r) = renderer() else { return };
    for (w, h, scale) in [
        (320u32, 260u32, 1.0f32),
        (480, 300, 1.0),
        (200, 180, 1.0),
        (640, 520, 2.0),
        (400, 325, 1.25),
        (320, 260, 1.0),
    ] {
        let (lw, lh) = (w as f32 / scale, h as f32 / scale);
        let l = window(lw, lh, 2);
        r.invalidate();
        within(
            &gpu(&mut r, &l, w, h, scale, None),
            &cpu(&l, w, h, scale),
            &format!("{w}x{h}@{scale}"),
        );
        assert!(r.stats.full, "a resized target is drawn whole");
    }
}

#[test]
fn hover_redraws_only_its_damage_and_matches_a_full_render() {
    let Some(mut r) = renderer() else { return };
    let (w, h) = (320u32, 260u32);
    let first = window(320.0, 260.0, 1);
    let _ = gpu(&mut r, &first, w, h, 1.0, None);
    let full_px = u64::from(w * h);
    // Hover moves from row 1 to row 4: the old and the new row bounds are the damage.
    let next = window(320.0, 260.0, 4);
    let mut d = Damage::new();
    d.add(Rect::new(20.0, 84.0, 280.0, 20.0));
    d.add(Rect::new(20.0, 156.0, 280.0, 20.0));
    let img = gpu(&mut r, &next, w, h, 1.0, Some(&d));
    assert!(!r.stats.full);
    assert!(
        r.stats.damage_px > 0 && r.stats.damage_px < full_px / 2,
        "{:?}",
        r.stats
    );
    within(&img, &cpu(&next, w, h, 1.0), "after a damage-only frame");
    // Nothing changed: nothing is drawn.
    let idle = gpu(&mut r, &next, w, h, 1.0, Some(&Damage::new()));
    assert_eq!(r.stats.damage_px, 0);
    assert_eq!(r.stats.draws, 0);
    assert_eq!(idle, img, "an idle frame keeps every pixel");
}

#[test]
fn a_lost_device_falls_back_then_recovers_with_the_same_pixels() {
    let Some(mut r) = renderer() else { return };
    let (w, h) = (320u32, 260u32);
    let l = window(320.0, 260.0, 3);
    let before = gpu(&mut r, &l, w, h, 1.0, None);
    r.simulate_device_loss();
    let lost = r.render_image(Frame {
        list: &l,
        text: None,
        width: w,
        height: h,
        scale: 1.0,
        clear: BG,
        damage: None,
    });
    assert_eq!(lost.err().map(|e| e.status), Some(Status::DeviceLost));
    // The caller draws this frame on the CPU; the next GPU frame makes a new device.
    let after = gpu(&mut r, &l, w, h, 1.0, Some(&Damage::new()));
    assert_eq!(r.devices_made, 2);
    assert!(
        r.stats.full,
        "a new device redraws everything even without damage"
    );
    assert_eq!(after, before);
}

#[test]
fn three_losses_in_a_minute_make_the_renderer_give_up() {
    let Some(mut r) = renderer() else { return };
    let l = window(200.0, 180.0, 0);
    for _ in 0..3 {
        let _ = gpu(&mut r, &l, 200, 180, 1.0, None);
        r.simulate_device_loss();
        let e = r.render_image(Frame {
            list: &l,
            text: None,
            width: 200,
            height: 180,
            scale: 1.0,
            clear: BG,
            damage: None,
        });
        assert!(e.is_err());
    }
    assert!(r.gave_up());
    let e = r.render_image(Frame {
        list: &l,
        text: None,
        width: 200,
        height: 180,
        scale: 1.0,
        clear: BG,
        damage: None,
    });
    assert_eq!(e.err().map(|e| e.status), Some(Status::Unavailable));
}

#[test]
fn pipeline_cache_is_written_and_used_on_the_next_start() {
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("gpu-cache-{}/pipeline-cache", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut o = opts();
    o.cache_file = Some(path.clone());
    let first = match Renderer::new(o.clone()) {
        Ok(r) => r,
        Err(e) => {
            assert!(std::env::var_os("WOLKIT_GPU_REQUIRED").is_none(), "{e}");
            return;
        }
    };
    assert!(!first.pipeline_cache_hit(), "no file yet");
    let written = first
        .save_pipeline_cache()
        .unwrap_or_else(|e| panic!("{e}"));
    drop(first);
    if !written {
        // The driver returned a blob without a valid header for this device (lavapipe 26.2
        // returns 20 bytes); nothing is written and the next start is cold, by design.
        assert!(!path.exists());
        let again = Renderer::new(o).unwrap_or_else(|e| panic!("{e}"));
        assert!(!again.pipeline_cache_hit());
        println!("note: the driver gave no usable pipeline cache; warm start not exercised");
        return;
    }
    let second = Renderer::new(o.clone()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        second.pipeline_cache_hit(),
        "the saved cache seeds the second start"
    );
    drop(second);
    // A damaged file is ignored, not handed to the driver.
    let mut bytes = std::fs::read(&path).unwrap_or_default();
    if let Some(b) = bytes.get_mut(40) {
        *b ^= 0xff;
    }
    std::fs::write(&path, &bytes).unwrap_or_default();
    let third = Renderer::new(o).unwrap_or_else(|e| panic!("{e}"));
    assert!(!third.pipeline_cache_hit());
}

#[test]
fn canvas_ops_draw_the_same_on_gpu_and_cpu() {
    let Some(mut r) = renderer() else { return };
    let mut e = Encoder::new();
    e.rect(Rect::new(0.0, 0.0, 200.0, 120.0), Color::rgb(250, 250, 252))
        .shadow(
            Rect::new(20.0, 20.0, 70.0, 50.0),
            [8.0; 4],
            [10.0, 0.0, 0.0, 3.0],
            Color::rgba(0, 0, 0, 80),
        )
        .linear(
            Rect::new(20.0, 20.0, 70.0, 50.0),
            [8.0; 4],
            [0.0, 0.0, 1.0, 1.0],
            Color::rgb(60, 120, 255),
            Color::rgb(160, 60, 255),
        )
        .push_clip(Rect::new(110.0, 10.0, 80.0, 100.0), 12.0)
        .push_opacity(160, true)
        .rect(Rect::new(100.0, 30.0, 60.0, 60.0), Color::rgb(240, 90, 120))
        .rect(Rect::new(130.0, 50.0, 70.0, 60.0), Color::rgb(40, 160, 120))
        .pop_opacity()
        .pop_clip()
        .border(
            Rect::new(4.0, 4.0, 192.0, 112.0),
            [6.0; 4],
            2.0,
            Color::rgb(120, 120, 140),
        );
    let c = Canvas::decode(&e.bytes).unwrap_or_else(|e| panic!("{e}"));
    let mut l = DisplayList::new();
    c.emit(Rect::new(30.0, 20.0, 200.0, 120.0), &mut l);
    l.finish();
    within(
        &gpu(&mut r, &l, 260, 160, 1.0, None),
        &cpu(&l, 260, 160, 1.0),
        "canvas",
    );
    assert!(r.stats.layers == 1 && r.stats.draws <= 10, "{:?}", r.stats);
}
