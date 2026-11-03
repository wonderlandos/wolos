// SPDX-License-Identifier: BSD-2-Clause
//! The instance builder on a realistic window: draw-call budget (plan budget 9), glyph instances,
//! chunk reuse when only the scroll offset changes, and the 64-byte instance layout.

#[path = "support/text.rs"]
mod text_support;

use text_support::{Fonts, fonts};
use wolkit::paint::batch::{InstanceBuilder, Pass, kind};
use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::list::{DisplayList, Fill, Radii, Shadow};
use wolkit::paint::text::push_layout;
use wolkit::text::cache::ShapeCache;
use wolkit::text::glyphcache::{GlyphCache, Glyphs};
use wolkit::text::layout::{LayoutOptions, TextStyle, layout_text};

/// A "typical window": a sidebar, a toolbar, a scrolling list of 40 text rows and an overlay.
fn window(f: &Fonts, shapes: &mut ShapeCache, scroll: f32, list_version: u64) -> DisplayList {
    let st = TextStyle::new(f.latin, 13.0);
    let mut l = DisplayList::new();
    let mut text = |l: &mut DisplayList, s: &str, x: f32, y: f32| {
        let lay = layout_text(&f.db, shapes, s, &st, &LayoutOptions::default());
        push_layout(l, &lay, x, y, Color::rgb(20, 20, 30));
    };
    // Window chrome (chunk 1, z 0).
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
    // Scrolling content (chunk 2, z 0 but later).
    l.begin_chunk(
        2,
        list_version,
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
    // Overlay (chunk 3, z 5).
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

#[test]
fn a_typical_window_is_a_handful_of_draws() {
    let f = fonts();
    let mut shapes = ShapeCache::new(512);
    let mut glyphs = GlyphCache::default();
    let list = window(&f, &mut shapes, 0.0, 1);
    let mut b = InstanceBuilder::new();
    let mut src = Glyphs {
        db: &f.db,
        cache: &mut glyphs,
    };
    b.build(&list, &mut src, 1.0, 0);
    assert!(
        b.stats.draws <= 10,
        "budget 9: at most 10 draws, got {:?}",
        b.stats
    );
    assert!(
        b.stats.instances > 500,
        "many instances in few draws: {:?}",
        b.stats
    );
    // Scrolled and overlay chunks have their own transforms.
    assert_eq!(b.stats.draws, 2, "{:?}", b.stats);
    assert_eq!(b.stats.layers, 0);
    assert_eq!(b.stats.path_glyphs, 0);
}

#[test]
fn glyph_instances_point_into_the_atlas() {
    let f = fonts();
    let mut shapes = ShapeCache::new(64);
    let mut glyphs = GlyphCache::default();
    let list = window(&f, &mut shapes, 0.0, 1);
    let mut b = InstanceBuilder::new();
    {
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        b.build(&list, &mut src, 1.0, 0);
    }
    let g: Vec<_> = b
        .instances
        .iter()
        .filter(|i| i.kind() == kind::GLYPH)
        .collect();
    assert!(g.len() > 300);
    for i in &g {
        assert!((i.page() as usize) < glyphs.atlas.page_count());
        assert!(
            i.uv[2] > 0 && i.uv[3] > 0,
            "glyph rectangles have a size in texels"
        );
        assert!(u32::from(i.uv[0]) + u32::from(i.uv[2]) <= glyphs.atlas.page_size());
        assert_eq!(i.rect[2], f32::from(i.uv[2]), "quad is 1:1 with the texels");
    }
}

#[test]
fn scrolling_reuses_every_chunk() {
    let f = fonts();
    let mut shapes = ShapeCache::new(512);
    let mut glyphs = GlyphCache::default();
    let mut b = InstanceBuilder::new();
    let l0 = window(&f, &mut shapes, 0.0, 1);
    {
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        b.build(&l0, &mut src, 1.0, glyphs_gen());
    }
    let first = b.instances.clone();
    // Whole-pixel steps: the sub-pixel part of the offset stays 0, so nothing is rebuilt.
    for step in 1..20 {
        let l = window(&f, &mut shapes, step as f32 * 14.0, 1);
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        b.build(&l, &mut src, 1.0, glyphs_gen());
        assert_eq!(b.stats.chunks_rebuilt, 0, "step {step}");
        assert_eq!(b.instances, first, "instances are scroll independent");
        let scrolled = b
            .draws
            .iter()
            .find(|d| d.offset.1 < 0.0)
            .expect("scrolled chunk has its own transform");
        assert_eq!(scrolled.offset, (0.0, -(step as f32) * 14.0));
    }
    // A half-pixel step changes the sub-pixel part: the scrolled chunk is rebuilt once (its
    // glyphs move to the CPU rasteriser's pixel grid), then whole-pixel steps reuse it again.
    let half =
        |y: f32, b: &mut InstanceBuilder, glyphs: &mut GlyphCache, shapes: &mut ShapeCache| {
            let l = window(&f, shapes, y, 1);
            let mut src = Glyphs {
                db: &f.db,
                cache: glyphs,
            };
            b.build(&l, &mut src, 1.0, glyphs_gen());
            b.stats.chunks_rebuilt
        };
    assert_eq!(half(300.5, &mut b, &mut glyphs, &mut shapes), 1);
    assert_eq!(half(314.5, &mut b, &mut glyphs, &mut shapes), 0);
    assert_eq!(half(328.0, &mut b, &mut glyphs, &mut shapes), 1);
}

fn glyphs_gen() -> u64 {
    0
}

#[test]
fn changing_one_chunks_version_rebuilds_only_that_chunk() {
    let f = fonts();
    let mut shapes = ShapeCache::new(512);
    let mut glyphs = GlyphCache::default();
    let mut b = InstanceBuilder::new();
    for (v, expect_rebuilt) in [(1u64, 3), (1, 0), (2, 1), (2, 0)] {
        let l = window(&f, &mut shapes, 0.0, v);
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        b.build(&l, &mut src, 1.0, 0);
        assert_eq!(b.stats.chunks_rebuilt, expect_rebuilt, "version {v}");
    }
}

#[test]
fn hidpi_scales_instance_geometry() {
    let f = fonts();
    let mut shapes = ShapeCache::new(64);
    let mut glyphs = GlyphCache::default();
    let l = window(&f, &mut shapes, 0.0, 1);
    let (mut b1, mut b2) = (InstanceBuilder::new(), InstanceBuilder::new());
    {
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        b1.build(&l, &mut src, 1.0, 0);
        b2.build(&l, &mut src, 2.0, 0);
    }
    let r1 = b1
        .instances
        .iter()
        .find(|i| i.kind() == kind::RECT)
        .unwrap();
    let r2 = b2
        .instances
        .iter()
        .find(|i| i.kind() == kind::RECT)
        .unwrap();
    assert_eq!(r2.rect[2], r1.rect[2] * 2.0);
    assert!(
        b2.draws.iter().any(|d| d.offset.0 == 800.0),
        "overlay offset in device pixels"
    );
}

#[test]
fn layer_markers_are_balanced_even_for_unbalanced_lists() {
    let mut l = DisplayList::new();
    l.push_opacity(100, true);
    l.solid(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1, 2, 3));
    l.push_opacity(100, true);
    l.solid(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1, 2, 3));
    // Neither group is closed.
    l.finish();
    let mut b = InstanceBuilder::new();
    b.build(&l, &mut wolkit::paint::glyphs::NoGlyphs, 1.0, 0);
    let begins = b
        .draws
        .iter()
        .filter(|d| d.pass == Pass::LayerBegin)
        .count();
    let ends = b
        .draws
        .iter()
        .filter(|d| matches!(d.pass, Pass::LayerEnd { .. }))
        .count();
    assert_eq!(begins, 2);
    assert_eq!(begins, ends);
}

#[test]
fn degenerate_input_does_not_panic_or_emit_garbage() {
    let mut l = DisplayList::new();
    l.solid(Rect::new(f32::NAN, 0.0, 10.0, 10.0), Color::rgb(1, 1, 1));
    l.rect(
        Rect::new(0.0, 0.0, f32::INFINITY, 4.0),
        Radii([f32::NAN, -5.0, 1e9, 3.0]),
        Fill::Solid(Color::rgb(1, 1, 1)),
    );
    l.border(
        Rect::new(0.0, 0.0, 0.0, 0.0),
        Radii::ZERO,
        3.0,
        Color::rgb(1, 1, 1),
    );
    l.pop_clip();
    l.pop_opacity();
    l.finish();
    let mut b = InstanceBuilder::new();
    b.build(&l, &mut wolkit::paint::glyphs::NoGlyphs, 1.0, 0);
    for i in &b.instances {
        assert!(i.rect.iter().all(|v| v.is_finite()), "{i:?}");
    }
    let img = wolkit::cpu::raster::render(
        &l,
        &mut wolkit::paint::glyphs::NoGlyphs,
        16,
        16,
        &Default::default(),
    );
    assert_eq!(img.data.len(), 16 * 16 * 4);
}
