// SPDX-License-Identifier: BSD-2-Clause
//! Hit-rate tests: the shaped-run LRU and the glyph atlas must serve steady-state text entirely
//! from cache (plan budget 8: atlas hit rate 100% in steady state).

#[path = "support/text.rs"]
mod text_support;

use std::sync::Arc;

use harfrust::Script;
use text_support::fonts;
use wolkit::paint::geom::Color;
use wolkit::paint::list::DisplayList;
use wolkit::paint::text::push_layout;
use wolkit::text::atlas::{Atlas, DEFAULT_CAP_BYTES, DEFAULT_PAGE_SIZE};
use wolkit::text::cache::ShapeCache;
use wolkit::text::glyphcache::{GlyphCache, Glyphs};
use wolkit::text::layout::{LayoutOptions, TextStyle, layout_text};
use wolkit::text::shape::{Dir, FontFeature, ShapeParams};

#[test]
fn second_pass_over_the_same_text_is_all_hits() {
    let f = fonts();
    let mut c = ShapeCache::new(256);
    let st = TextStyle::new(f.latin, 14.0);
    let o = LayoutOptions {
        max_width: 180.0,
        ..LayoutOptions::default()
    };
    let docs = [
        "The quick brown fox jumps",
        "你好，世界。",
        "Hello \u{5E9}\u{5DC}\u{5D5}\u{5DD} world",
        "plain",
    ];
    for d in docs {
        layout_text(&f.db, &mut c, d, &st, &o);
    }
    let misses = c.stats.misses;
    assert!(misses > 0);
    for _ in 0..50 {
        for d in docs {
            layout_text(&f.db, &mut c, d, &st, &o);
        }
    }
    assert_eq!(c.stats.misses, misses, "no new misses once warm");
    assert!(c.stats.hit_rate() > 0.97, "hit rate {}", c.stats.hit_rate());
}

#[test]
fn a_hit_returns_the_same_shared_run() {
    let f = fonts();
    let mut c = ShapeCache::new(8);
    let p = ShapeParams {
        font: f.latin,
        size: 16.0,
        dir: Dir::Ltr,
        script: Script::LATIN,
        features: &[],
    };
    let a = c.get(&f.db, "Wonderland", &p);
    let b = c.get(&f.db, "Wonderland", &p);
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!((c.stats.hits, c.stats.misses), (1, 1));
}

#[test]
fn every_key_part_makes_a_separate_entry() {
    let f = fonts();
    let mut c = ShapeCache::new(32);
    let base = ShapeParams {
        font: f.latin,
        size: 16.0,
        dir: Dir::Ltr,
        script: Script::LATIN,
        features: &[],
    };
    let nolig = [FontFeature::new(b"liga", 0)];
    let variants = [
        ("abc", base),
        ("abd", base),
        ("abc", ShapeParams { size: 17.0, ..base }),
        (
            "abc",
            ShapeParams {
                font: f.latin_bold,
                ..base
            },
        ),
        (
            "abc",
            ShapeParams {
                dir: Dir::Rtl,
                ..base
            },
        ),
        (
            "abc",
            ShapeParams {
                features: &nolig,
                ..base
            },
        ),
        (
            "abc",
            ShapeParams {
                script: Script::HEBREW,
                ..base
            },
        ),
    ];
    let runs: Vec<_> = variants.iter().map(|(t, p)| c.get(&f.db, t, p)).collect();
    assert_eq!(c.stats.misses as usize, variants.len());
    assert_eq!(c.len(), variants.len());
    for i in 0..runs.len() {
        for j in i + 1..runs.len() {
            assert!(!Arc::ptr_eq(&runs[i], &runs[j]));
        }
    }
    // And each is found again.
    for (t, p) in &variants {
        c.get(&f.db, t, p);
    }
    assert_eq!(c.stats.hits as usize, variants.len());
}

#[test]
fn least_recently_used_entry_is_evicted_first() {
    let f = fonts();
    let mut c = ShapeCache::new(3);
    let p = ShapeParams {
        font: f.latin,
        size: 12.0,
        dir: Dir::Ltr,
        script: Script::LATIN,
        features: &[],
    };
    for t in ["a", "b", "c"] {
        c.get(&f.db, t, &p);
    }
    c.get(&f.db, "a", &p); // a is now the most recent; b the oldest
    c.get(&f.db, "d", &p); // evicts b
    assert_eq!(c.stats.evictions, 1);
    let hits = c.stats.hits;
    c.get(&f.db, "a", &p);
    c.get(&f.db, "c", &p);
    c.get(&f.db, "d", &p);
    assert_eq!(c.stats.hits, hits + 3, "a, c and d survived");
    let misses = c.stats.misses;
    c.get(&f.db, "b", &p);
    assert_eq!(c.stats.misses, misses + 1, "b was evicted");
    assert_eq!(c.len(), 3);
}

#[test]
fn cache_never_exceeds_its_capacity() {
    let f = fonts();
    let mut c = ShapeCache::new(16);
    let p = ShapeParams {
        font: f.latin,
        size: 12.0,
        dir: Dir::Ltr,
        script: Script::LATIN,
        features: &[],
    };
    for i in 0..500 {
        c.get(&f.db, &format!("text {i}"), &p);
        assert!(c.len() <= 16);
    }
    assert_eq!(c.stats.evictions, 500 - 16);
}

// ---- the atlas -------------------------------------------------------------------------------

fn frame(f: &text_support::Fonts, shapes: &mut ShapeCache, cache: &mut GlyphCache, lines: &[&str]) {
    let st = TextStyle::new(f.latin, 13.0);
    let mut list = DisplayList::new();
    for (i, t) in lines.iter().enumerate() {
        let lay = layout_text(&f.db, shapes, t, &st, &LayoutOptions::default());
        push_layout(
            &mut list,
            &lay,
            4.25,
            4.0 + i as f32 * 20.0,
            Color::rgb(0, 0, 0),
        );
    }
    list.finish();
    cache.atlas.begin_frame();
    let mut src = Glyphs { db: &f.db, cache };
    // Draw through the rasteriser so the lookups are the real ones.
    wolkit::cpu::raster::render(
        &list,
        &mut src,
        400,
        200,
        &wolkit::cpu::raster::RasterOptions::default(),
    );
}

#[test]
fn atlas_hit_rate_is_100_percent_in_steady_state() {
    let f = fonts();
    let mut shapes = ShapeCache::new(128);
    let mut glyphs = GlyphCache::default();
    let lines = [
        "Wonderland text 0123456789",
        "你好，世界。日本語のテキスト",
        "Hello \u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{645}\u{631}\u{62D}\u{628}\u{627}",
    ];
    frame(&f, &mut shapes, &mut glyphs, &lines);
    assert!(glyphs.atlas.stats.misses > 0, "cold frame renders glyphs");
    let before = glyphs.atlas.stats;
    for _ in 0..30 {
        frame(&f, &mut shapes, &mut glyphs, &lines);
    }
    let after = glyphs.atlas.stats;
    assert_eq!(
        after.misses, before.misses,
        "steady state has no atlas misses"
    );
    assert!(after.hits > before.hits);
    assert_eq!(after.evictions, 0);
    // Rate over the steady-state window is exactly 1.
    let window_hits = after.hits - before.hits;
    assert!(window_hits > 100, "{window_hits}");
}

#[test]
fn defaults_match_the_plan_32_mib_of_2048_pages() {
    let a = Atlas::default();
    assert_eq!(a.page_size(), DEFAULT_PAGE_SIZE);
    assert_eq!(DEFAULT_PAGE_SIZE, 2048);
    assert_eq!(DEFAULT_CAP_BYTES, 32 * 1024 * 1024);
    assert_eq!(a.max_pages(), 8);
    assert_eq!(a.allocated_bytes(), 0, "pages are allocated on first use");
}

#[test]
fn glyph_churn_stays_inside_the_atlas_cap_and_recovers() {
    // A small atlas under heavy churn: many sizes of every glyph, then back to a small working set.
    let f = fonts();
    let mut glyphs = GlyphCache::new(Atlas::new(128, 4 * 128 * 128));
    let mut shapes = ShapeCache::new(1024);
    let chars: String = ('a'..='z').chain('A'..='Z').collect();
    for size in 10..60 {
        let st = TextStyle::new(f.latin, size as f32);
        let mut list = DisplayList::new();
        let lay = layout_text(&f.db, &mut shapes, &chars, &st, &LayoutOptions::default());
        push_layout(&mut list, &lay, 0.0, 0.0, Color::rgb(0, 0, 0));
        list.finish();
        glyphs.atlas.begin_frame();
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        wolkit::cpu::raster::render(
            &list,
            &mut src,
            64,
            64,
            &wolkit::cpu::raster::RasterOptions::default(),
        );
        assert!(glyphs.atlas.allocated_bytes() <= 4 * 128 * 128);
    }
    assert!(glyphs.atlas.stats.evictions > 0, "churn forced evictions");
    // A small working set fits and then hits 100%.
    let lines = ["steady"];
    frame(&f, &mut shapes, &mut glyphs, &lines);
    let s0 = glyphs.atlas.stats;
    frame(&f, &mut shapes, &mut glyphs, &lines);
    assert_eq!(glyphs.atlas.stats.misses, s0.misses);
}

#[test]
fn trim_drops_every_page() {
    let f = fonts();
    let mut shapes = ShapeCache::new(16);
    let mut glyphs = GlyphCache::default();
    frame(&f, &mut shapes, &mut glyphs, &["trim me"]);
    assert!(glyphs.atlas.allocated_bytes() > 0);
    glyphs.trim();
    assert_eq!(glyphs.atlas.allocated_bytes(), 0);
    let m = glyphs.atlas.stats.misses;
    frame(&f, &mut shapes, &mut glyphs, &["trim me"]);
    assert!(glyphs.atlas.stats.misses > m, "re-rendered after the trim");
}
