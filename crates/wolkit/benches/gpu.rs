// SPDX-License-Identifier: BSD-2-Clause
//! GPU renderer benchmarks for plan budgets 1, 2, 7, 9 and 10 (section 4.1).
//!
//! * 1: input to frame submitted (engine side): a hover change rebuilds the list, records and
//!   submits; timed from the event to the return of the submit.
//! * 2: main-thread CPU per interaction frame (the renderer's part: instance build, ring write,
//!   upload, recording).
//! * 7: scrolling a 100,000-row list at 120 Hz: CPU per frame and frames over the 8.33 ms budget.
//! * 9: draw calls for a typical window.
//! * 10: damage of a caret blink and of an idle frame.
//!
//! On a software device (lavapipe) the GPU work also runs on the CPU, on the driver's threads;
//! the numbers here are main-thread times, which is what the budgets bound.
//! `cargo bench -p wolkit --bench gpu [-- --check]`; limits in `benches/budgets.toml` `[gpu]`.

#[path = "../tests/support/text.rs"]
mod text_support;
#[path = "w3_support/mod.rs"]
mod w3_support;

use std::time::Instant;

use wolkit::gpu::select::Preference;
use wolkit::gpu::{Frame, Renderer, RendererOptions};
use wolkit::paint::damage::Damage;
use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::list::{DisplayList, Fill, Radii, Shadow};
use wolkit::paint::text::push_layout;
use wolkit::text::cache::ShapeCache;
use wolkit::text::glyphcache::GlyphCache;
use wolkit::text::layout::{LayoutOptions, TextStyle, layout_text};

const W: u32 = 640;
const H: u32 = 480;
const ROW: f32 = 28.0;
const ROWS: u32 = 100_000;
const FRAME_NS: f64 = 8_333_333.0;
const BG: Color = Color::rgb(243, 241, 248);

struct Ctx {
    f: text_support::Fonts,
    shapes: ShapeCache,
    glyphs: GlyphCache,
}

fn text(c: &mut Ctx, l: &mut DisplayList, s: &str, x: f32, y: f32) {
    let st = TextStyle::new(c.f.latin, 13.0);
    let lay = layout_text(&c.f.db, &mut c.shapes, s, &st, &LayoutOptions::default());
    push_layout(l, &lay, x, y, Color::rgb(20, 20, 30));
}

/// A typical window: sidebar, toolbar, a scrolled list chunk with text rows (only visible rows,
/// as `vlist` emits them), a caret, and an overlay menu with a shadow.
fn window(c: &mut Ctx, scroll: f32, hover: u32, caret: bool) -> DisplayList {
    let mut l = DisplayList::new();
    l.begin_chunk(1, 1, Rect::new(0.0, 0.0, 640.0, 480.0), (0.0, 0.0), 0);
    l.solid(Rect::new(0.0, 0.0, 640.0, 480.0), Color::rgb(243, 241, 248));
    l.solid(Rect::new(0.0, 0.0, 160.0, 480.0), Color::rgb(236, 236, 242));
    for i in 0..8u16 {
        text(
            c,
            &mut l,
            &format!("Sidebar {i}"),
            16.0,
            20.0 + f32::from(i) * 30.0,
        );
    }
    l.rect(
        Rect::new(160.0, 0.0, 480.0, 44.0),
        Radii::ZERO,
        Fill::vertical(Color::rgb(255, 255, 255), Color::rgb(250, 250, 252)),
    );
    text(c, &mut l, "Toolbar title", 176.0, 14.0);
    let first = (scroll / ROW).floor().max(0.0);
    let first_u = first as u64;
    // The list content: rows [first, first + 17), version = first row and hover.
    l.begin_chunk(
        2,
        (first_u << 20) | u64::from(hover),
        Rect::new(160.0, 44.0, 480.0, 436.0),
        (0.0, -scroll),
        0,
    );
    for k in 0..17u64 {
        let row = first_u + k;
        if row >= u64::from(ROWS) {
            break;
        }
        let y = 44.0 + row as f32 * ROW;
        let c0 = if row == u64::from(hover) {
            Color::rgb(124, 92, 255)
        } else if row.is_multiple_of(2) {
            Color::rgb(255, 255, 255)
        } else {
            Color::rgb(248, 248, 252)
        };
        l.rect(
            Rect::new(168.0, y + 2.0, 464.0, ROW - 4.0),
            Radii::uniform(6.0),
            Fill::Solid(c0),
        );
        text(
            c,
            &mut l,
            &format!("Row {row} with a title and some detail"),
            180.0,
            y + 7.0,
        );
    }
    l.begin_chunk(
        3,
        u64::from(caret) + 1,
        Rect::new(0.0, 0.0, 640.0, 480.0),
        (0.0, 0.0),
        1,
    );
    if caret {
        l.solid(Rect::new(300.0, 14.0, 1.0, 16.0), Color::rgb(124, 92, 255));
    }
    l.begin_chunk(4, 1, Rect::new(420.0, 40.0, 200.0, 160.0), (0.0, 0.0), 2);
    l.shadow(
        Rect::new(420.0, 40.0, 200.0, 160.0),
        Radii::uniform(10.0),
        Shadow::new(16.0, 0.0, 0.0, 6.0, Color::rgba(20, 20, 60, 90)),
    );
    l.rect(
        Rect::new(420.0, 40.0, 200.0, 160.0),
        Radii::uniform(10.0),
        Fill::Solid(Color::rgb(255, 255, 255)),
    );
    for i in 0..5u16 {
        text(
            c,
            &mut l,
            &format!("Menu item {i}"),
            436.0,
            52.0 + f32::from(i) * 28.0,
        );
    }
    l.finish();
    l
}

fn frame<'a>(c: &'a mut Ctx, l: &'a DisplayList, damage: Option<&'a Damage>) -> Frame<'a> {
    Frame {
        list: l,
        text: Some((&c.f.db, &mut c.glyphs)),
        width: W,
        height: H,
        scale: 1.0,
        clear: BG,
        damage,
    }
}

fn pct(v: &mut [f64], p: usize) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get((v.len() * p / 100).min(v.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0)
}

fn main() {
    let mut b = w3_support::Bench::new("gpu");
    let mut opts = RendererOptions::default();
    opts.device.preference = Preference {
        prefer_software: std::env::var_os("WOLKIT_GPU_HARDWARE").is_none(),
        ..Preference::default()
    };
    let mut r = match Renderer::new(opts) {
        Ok(r) => r,
        Err(e) => {
            println!("bench gpu skipped: {e}");
            return;
        }
    };
    if let Some(g) = r.gpu() {
        println!("bench gpu device: {} ({:?})", g.facts.name, g.facts.kind);
    }
    let mut c = Ctx {
        f: text_support::fonts(),
        shapes: ShapeCache::new(4096),
        glyphs: GlyphCache::default(),
    };
    // Warm up: the first frame uploads the atlases and draws everything.
    let l = window(&mut c, 0.0, 3, false);
    let _ = r.render_image(frame(&mut c, &l, None));
    // Budget 9: draws for the typical window.
    b.value("draws", f64::from(r.stats.draws));
    b.value("instances", f64::from(r.stats.instances));

    // Budgets 1 and 2: hover moves one row per interaction.
    let (mut latency, mut cpu) = (Vec::new(), Vec::new());
    for i in 0..300u32 {
        let hover = 2 + i % 12;
        let t = Instant::now();
        let l = window(&mut c, 0.0, hover, false);
        let mut d = Damage::new();
        d.add(Rect::new(
            160.0,
            44.0 + (hover - 1) as f32 * ROW,
            480.0,
            2.0 * ROW,
        ));
        if r.render_submit(frame(&mut c, &l, Some(&d))).is_err() {
            break;
        }
        latency.push(t.elapsed().as_nanos() as f64);
        cpu.push(r.stats.record_ns.saturating_sub(r.stats.wait_ns) as f64);
    }
    let _ = r.wait_idle();
    b.value("hover_damage_px", r.stats.damage_px as f64);
    let (l50, l95) = (pct(&mut latency, 50), pct(&mut latency, 95));
    println!(
        "bench gpu.input_to_submit p50={l50:.0} p95={l95:.0} n={}",
        latency.len()
    );
    b.value("input_to_submit_p50_ns", l50);
    b.value("input_to_submit_p95_ns", l95);
    b.value("interaction_cpu_p95_ns", pct(&mut cpu, 95));

    // Budget 7: scrolling. Smooth: 3000 frames of 9 px. Fling: 3000 frames that cross all
    // 100,000 rows (every frame exposes new rows, the worst case for the list chunk).
    for (name, step) in [("scroll", 9.0f32), ("fling", ROWS as f32 * ROW / 3000.0)] {
        let (mut cpu, mut wall) = (Vec::new(), Vec::new());
        let mut over = 0u32;
        let mut y = 0.0f32;
        for _ in 0..3000 {
            y = (y + step).min(ROWS as f32 * ROW).floor();
            let l = window(&mut c, y, u32::MAX, false);
            let mut d = Damage::new();
            d.add(Rect::new(160.0, 44.0, 480.0, 436.0));
            let t = Instant::now();
            if r.render_submit(frame(&mut c, &l, Some(&d))).is_err() {
                break;
            }
            let ns = t.elapsed().as_nanos() as f64;
            let main = r.stats.record_ns.saturating_sub(r.stats.wait_ns) as f64;
            over += u32::from(main > FRAME_NS);
            cpu.push(main);
            wall.push(ns);
        }
        let _ = r.wait_idle();
        println!("bench gpu.{name} rows_crossed={:.0}", y / ROW);
        b.value(&format!("{name}_cpu_p50_ns"), pct(&mut cpu, 50));
        b.value(&format!("{name}_cpu_p95_ns"), pct(&mut cpu, 95));
        b.value(&format!("{name}_wall_p95_ns"), pct(&mut wall, 95));
        b.value(&format!("{name}_frames_over_budget"), f64::from(over));
    }

    // Budget 10: a caret blink damages its own bounds; an idle frame damages nothing.
    let l = window(&mut c, 0.0, 3, false);
    let _ = r.render_image(frame(&mut c, &l, None));
    let l = window(&mut c, 0.0, 3, true);
    let mut d = Damage::new();
    d.add(Rect::new(300.0, 14.0, 1.0, 16.0));
    let _ = r.render_image(frame(&mut c, &l, Some(&d)));
    b.value("caret_damage_px", r.stats.damage_px as f64);
    let _ = r.render_image(frame(&mut c, &l, Some(&Damage::new())));
    b.value("idle_damage_px", r.stats.damage_px as f64);
    b.value("idle_draws", f64::from(r.stats.draws));
    // Present spike (v1 vs v2): bytes written per frame for the measured damage. v1 renders the
    // damage into the persistent target and copies damage (union buffer age, the same rectangle
    // for a repeated update) into the swapchain image; v2 would render straight into a dmabuf.
    for (name, px) in [("caret", 16.0f64), ("full", f64::from(W * H))] {
        println!(
            "bench gpu.present_{name} v1_bytes={} v2_bytes={}",
            px * 8.0,
            px * 4.0
        );
    }
    b.finish();
}
