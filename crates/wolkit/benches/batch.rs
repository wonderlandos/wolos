// SPDX-License-Identifier: BSD-2-Clause
//! Benchmark for plan budget 9 (batching): at most 10 draw calls for a typical window, and the
//! cost of building the instances, tracking damage and (for reference) rasterising on the CPU.
//!
//! `cargo bench -p wolkit --bench batch [-- --check]`

#[path = "../tests/support/text.rs"]
mod text_support;
#[path = "w3_support/mod.rs"]
mod w3_support;

use wolkit::cpu::raster::{RasterOptions, render};
use wolkit::paint::batch::InstanceBuilder;
use wolkit::paint::damage::{Damage, DamageTracker};
use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::list::{DisplayList, Fill, Radii, Shadow};
use wolkit::paint::text::push_layout;
use wolkit::text::cache::ShapeCache;
use wolkit::text::glyphcache::{GlyphCache, Glyphs};
use wolkit::text::layout::{LayoutOptions, TextStyle, layout_text};

/// A typical window: sidebar, toolbar, 40 scrolling rows with text, and an overlay menu.
fn window(f: &text_support::Fonts, shapes: &mut ShapeCache, scroll: f32) -> DisplayList {
    let st = TextStyle::new(f.latin, 13.0);
    let mut l = DisplayList::new();
    let mut text = |l: &mut DisplayList, s: &str, x: f32, y: f32| {
        let lay = layout_text(&f.db, shapes, s, &st, &LayoutOptions::default());
        push_layout(l, &lay, x, y, Color::rgb(20, 20, 30));
    };
    l.begin_chunk(1, 1, Rect::new(0.0, 0.0, 640.0, 480.0), (0.0, 0.0), 0);
    l.rect(
        Rect::new(0.0, 0.0, 640.0, 480.0),
        Radii::ZERO,
        Fill::Solid(Color::rgb(243, 241, 248)),
    );
    l.rect(
        Rect::new(0.0, 0.0, 160.0, 480.0),
        Radii::ZERO,
        Fill::Solid(Color::rgb(236, 236, 242)),
    );
    for i in 0..8 {
        text(
            &mut l,
            &format!("Sidebar {i}"),
            16.0,
            20.0 + i as f32 * 30.0,
        );
    }
    l.rect(
        Rect::new(160.0, 0.0, 480.0, 44.0),
        Radii::ZERO,
        Fill::vertical(Color::rgb(255, 255, 255), Color::rgb(250, 250, 252)),
    );
    text(&mut l, "Toolbar title", 176.0, 14.0);
    l.begin_chunk(
        2,
        1,
        Rect::new(160.0, 44.0, 480.0, 436.0),
        (0.0, -scroll),
        0,
    );
    l.push_clip(Rect::new(160.0, 44.0, 480.0, 436.0 + scroll), 0.0);
    for i in 0..40 {
        let y = 44.0 + i as f32 * 30.0;
        l.rect(
            Rect::new(168.0, y, 464.0, 26.0),
            Radii::uniform(6.0),
            Fill::Solid(if i % 2 == 0 {
                Color::rgb(255, 255, 255)
            } else {
                Color::rgb(242, 243, 248)
            }),
        );
        text(
            &mut l,
            &format!("Row {i} with some text 日本語"),
            180.0,
            y + 5.0,
        );
    }
    l.pop_clip();
    l.begin_chunk(3, 1, Rect::new(0.0, 0.0, 640.0, 480.0), (400.0, 100.0), 5);
    l.shadow(
        Rect::new(0.0, 0.0, 180.0, 120.0),
        Radii::uniform(10.0),
        Shadow::new(20.0, 0.0, 0.0, 8.0, Color::rgba(0, 0, 0, 70)),
    );
    l.rect(
        Rect::new(0.0, 0.0, 180.0, 120.0),
        Radii::uniform(10.0),
        Fill::Solid(Color::rgb(255, 255, 255)),
    );
    text(&mut l, "Menu item", 14.0, 12.0);
    l.finish();
    l
}

fn main() {
    let f = text_support::fonts();
    let mut b = w3_support::Bench::new("batch");
    let mut shapes = ShapeCache::new(512);
    let mut glyphs = GlyphCache::default();
    let list = window(&f, &mut shapes, 0.0);

    // Budget 9: draw calls.
    let mut builder = InstanceBuilder::new();
    {
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        builder.build(&list, &mut src, 1.0, 0);
    }
    b.value("draws", f64::from(builder.stats.draws));
    b.value("instances", f64::from(builder.stats.instances));
    b.value("instance_bytes", f64::from(builder.stats.instances) * 64.0);

    // Cold build: every chunk rebuilt each time.
    let mut v = 0u64;
    let mut cold_list = list.clone();
    b.time("build_cold_ns", 21, 20, || {
        v += 1;
        for c in &mut cold_list.chunks {
            c.version = v + 100;
        }
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        builder.build(&cold_list, &mut src, 1.0, 0);
    });

    // Warm build, scroll only: nothing rebuilt.
    let mut scrolled = list.clone();
    let mut k = 0.0f32;
    b.time("build_scroll_ns", 41, 2_000, || {
        k += 1.0;
        scrolled.chunks[1].offset.1 = -(k % 200.0);
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        builder.build(&scrolled, &mut src, 1.0, 0);
    });

    // Damage: a caret blink.
    let mut t = DamageTracker::new();
    let mut out = Damage::new();
    let win = Rect::new(0.0, 0.0, 640.0, 480.0);
    b.time("damage_blink_ns", 41, 20_000, || {
        t.update(7, Rect::new(200.0, 100.0, 1.0, 16.0), true);
        t.take(win, &mut out);
    });
    b.value("damage_blink_px", f64::from(out.area()));

    // For reference: the CPU rasteriser on the whole window.
    let mut shapes2 = ShapeCache::new(512);
    let l2 = window(&f, &mut shapes2, 0.0);
    b.time("cpu_raster_640x480_ns", 5, 1, || {
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        std::hint::black_box(render(&l2, &mut src, 640, 480, &RasterOptions::default()));
    });
    b.finish();
}
