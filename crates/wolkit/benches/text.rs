// SPDX-License-Identifier: BSD-2-Clause
//! Benchmark for plan budget 8 (text): shaped-run cache hit <= 2 us, cold shape of a 40-character
//! Latin line <= 50 us, atlas hit rate 100% in steady state.
//!
//! `cargo bench -p wolkit --bench text [-- --check]`

#[path = "w3_support/floor.rs"]
mod floor;
#[path = "../tests/support/text.rs"]
mod text_support;
#[path = "w3_support/timing.rs"]
mod timing;
#[path = "w3_support/mod.rs"]
mod w3_support;

use harfrust::Script;
use wolkit::paint::geom::Color;
use wolkit::paint::list::DisplayList;
use wolkit::paint::text::push_layout;
use wolkit::text::atlas::Atlas;
use wolkit::text::cache::ShapeCache;
use wolkit::text::fontdb::FontId;
use wolkit::text::glyph::{GlyphBitmap, GlyphKey, GlyphRasterizer};
use wolkit::text::glyphcache::{GlyphCache, Glyphs};
use wolkit::text::layout::{LayoutOptions, TextStyle, layout_text};
use wolkit::text::shape::{Dir, ShapeParams, shape_run};

const LINE: &str = "The quick brown fox jumps over the lazy."; // 40 characters

fn main() {
    let f = text_support::fonts();
    let mut b = w3_support::Bench::new("text");
    assert_eq!(LINE.chars().count(), 40);
    let p = ShapeParams {
        font: f.latin,
        size: 14.0,
        dir: Dir::Ltr,
        script: Script::LATIN,
        features: &[],
    };

    // Cache hit per run.
    let mut cache = ShapeCache::new(256);
    cache.get(&f.db, LINE, &p);
    b.time("shape_hit_ns", 41, 20_000, || {
        std::hint::black_box(cache.get(&f.db, LINE, &p));
    });

    // Cold shape of a 40-character Latin line (no cache): vary the text so nothing is reused by
    // the shaper's own state.
    let lines: Vec<String> = (0..64)
        .map(|i| format!("{} {:02}", &LINE[..LINE.len() - 3], i))
        .collect();
    let mut i = 0usize;
    b.time("shape_cold_40_ns", 41, 200, || {
        i += 1;
        std::hint::black_box(shape_run(&f.db, &lines[i % lines.len()], &p));
    });

    // A full paragraph layout with every run warm in the cache.
    let st = TextStyle::new(f.latin, 14.0);
    let o = LayoutOptions {
        max_width: 240.0,
        ..LayoutOptions::default()
    };
    let para = "Wonderland apps wrap long paragraphs at word boundaries, align them, and keep every shaped run in a cache.";
    layout_text(&f.db, &mut cache, para, &st, &o);
    b.time("layout_warm_para_ns", 41, 2_000, || {
        std::hint::black_box(layout_text(&f.db, &mut cache, para, &st, &o));
    });

    // Cold glyph rasterisation.
    let font = f.db.face(f.latin).unwrap().font().unwrap();
    let ids: Vec<u32> = ('a'..='z')
        .map(|c| font.charmap().map_unicode(c).unwrap().to_u32())
        .collect();
    let mut rast = GlyphRasterizer::new();
    let mut n = 0usize;
    b.time("glyph_raster_cold_ns", 41, 200, || {
        n += 1;
        std::hint::black_box(rast.rasterize(
            &f.db,
            GlyphKey::new(f.latin, ids[n % 26], 14.0, (n % 4) as u8),
        ));
    });

    // Atlas lookup (hit).
    let mut atlas = Atlas::default();
    let key = GlyphKey::new(FontId(0), 5, 14.0, 0);
    atlas.insert(
        key,
        &GlyphBitmap {
            width: 8,
            height: 10,
            left: 0,
            top: 10,
            data: vec![1; 80],
        },
    );
    b.time("atlas_hit_ns", 41, 50_000, || {
        std::hint::black_box(atlas.get(&key));
    });

    // Steady-state atlas hit rate over many frames of the same text.
    let mut shapes = ShapeCache::new(256);
    let mut glyphs = GlyphCache::default();
    let frame = |glyphs: &mut GlyphCache, shapes: &mut ShapeCache| {
        let mut list = DisplayList::new();
        for (i, t) in [
            para,
            "你好，世界。日本語のテキスト",
            "Hello \u{5E9}\u{5DC}\u{5D5}\u{5DD} world",
        ]
        .iter()
        .enumerate()
        {
            let lay = layout_text(&f.db, shapes, t, &st, &o);
            push_layout(
                &mut list,
                &lay,
                4.0,
                4.0 + i as f32 * 60.0,
                Color::rgb(0, 0, 0),
            );
        }
        list.finish();
        glyphs.atlas.begin_frame();
        let mut src = Glyphs {
            db: &f.db,
            cache: glyphs,
        };
        wolkit::cpu::raster::render(&list, &mut src, 300, 200, &Default::default());
    };
    frame(&mut glyphs, &mut shapes);
    let warm = glyphs.atlas.stats;
    for _ in 0..50 {
        frame(&mut glyphs, &mut shapes);
    }
    let s = glyphs.atlas.stats;
    let rate =
        (s.hits - warm.hits) as f64 / ((s.hits - warm.hits) + (s.misses - warm.misses)) as f64;
    b.at_least("atlas_hit_rate_steady", rate, 1.0);
    b.value("atlas_pages_used", glyphs.atlas.page_count() as f64);
    b.finish();
}
