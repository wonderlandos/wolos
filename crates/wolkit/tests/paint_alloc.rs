// SPDX-License-Identifier: BSD-2-Clause
//! Allocation discipline of the steady-state paint path (plan budget 11): once warm, a frame whose
//! content did not change must not touch the heap in the instance builder, the damage tracker, the
//! shaped-run cache or the atlas lookup.

mod text_support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use harfrust::Script;
use text_support::fonts;
use wolkit::paint::batch::InstanceBuilder;
use wolkit::paint::damage::{Damage, DamageTracker};
use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::list::{DisplayList, Fill, Radii};
use wolkit::paint::text::push_layout;
use wolkit::text::atlas::Atlas;
use wolkit::text::cache::ShapeCache;
use wolkit::text::fontdb::FontId;
use wolkit::text::glyph::{GlyphBitmap, GlyphKey};
use wolkit::text::glyphcache::{GlyphCache, Glyphs};
use wolkit::text::layout::{LayoutOptions, TextStyle, layout_text};
use wolkit::text::shape::{Dir, ShapeParams};

struct Counting;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

// SAFETY: forwards every call to the system allocator unchanged; only a thread-local counter is
// touched, and a `const`-initialised `Cell` never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn count<R>(f: impl FnOnce() -> R) -> (R, u64) {
    let before = ALLOCS.with(Cell::get);
    let r = f();
    (r, ALLOCS.with(Cell::get) - before)
}

#[test]
fn the_counter_sees_allocations() {
    let (_, n) = count(|| std::hint::black_box(vec![1u8; 4096]));
    assert!(n >= 1);
}

#[test]
fn instance_builder_is_allocation_free_when_nothing_changed() {
    let f = fonts();
    let mut shapes = ShapeCache::new(64);
    let mut glyphs = GlyphCache::default();
    let st = TextStyle::new(f.latin, 13.0);
    let mut list = DisplayList::new();
    list.begin_chunk(1, 1, Rect::new(0.0, 0.0, 400.0, 300.0), (0.0, 0.0), 0);
    list.rect(
        Rect::new(0.0, 0.0, 400.0, 300.0),
        Radii::uniform(4.0),
        Fill::Solid(Color::rgb(250, 250, 250)),
    );
    let lay = layout_text(
        &f.db,
        &mut shapes,
        "Steady state text",
        &st,
        &LayoutOptions::default(),
    );
    push_layout(&mut list, &lay, 10.0, 10.0, Color::rgb(0, 0, 0));
    list.begin_chunk(2, 1, Rect::new(0.0, 0.0, 400.0, 300.0), (0.0, 12.0), 1);
    list.solid(Rect::new(0.0, 0.0, 50.0, 50.0), Color::rgb(9, 9, 9));
    list.finish();

    let mut b = InstanceBuilder::new();
    for _ in 0..3 {
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        b.build(&list, &mut src, 1.0, 0);
    }
    // Same content, the scrolled chunk's offset changes every frame.
    for k in 0..10 {
        list.chunks[1].offset.1 = k as f32;
        let mut src = Glyphs {
            db: &f.db,
            cache: &mut glyphs,
        };
        let (_, n) = count(|| b.build(&list, &mut src, 1.0, 0));
        assert_eq!(n, 0, "frame {k}: {} allocations", n);
        assert_eq!(b.stats.chunks_rebuilt, 0);
    }
}

#[test]
fn damage_tracker_is_allocation_free_after_warmup() {
    let mut t = DamageTracker::new();
    let mut out = Damage::new();
    let win = Rect::new(0.0, 0.0, 800.0, 600.0);
    for slot in 0..64 {
        t.update(slot, Rect::new(slot as f32 * 10.0, 0.0, 8.0, 8.0), true);
    }
    t.take(win, &mut out);
    for frame in 0..20 {
        let (_, n) = count(|| {
            // A caret blink and a hover: two small rectangles.
            t.update(3, Rect::new(30.0, 0.0, 8.0, 8.0), true);
            t.update(
                40,
                Rect::new(400.0, 0.0 + (frame % 2) as f32, 8.0, 8.0),
                false,
            );
            t.take(win, &mut out);
        });
        assert_eq!(n, 0, "frame {frame}");
        assert!(!out.is_empty());
    }
}

#[test]
fn shape_cache_hit_is_allocation_free() {
    let f = fonts();
    let mut c = ShapeCache::new(16);
    let p = ShapeParams {
        font: f.latin,
        size: 14.0,
        dir: Dir::Ltr,
        script: Script::LATIN,
        features: &[],
    };
    let a = c.get(&f.db, "A cached line", &p);
    let (b, n) = count(|| c.get(&f.db, "A cached line", &p));
    assert_eq!(n, 0);
    assert!(std::sync::Arc::ptr_eq(&a, &b));
}

#[test]
fn atlas_lookup_hit_is_allocation_free() {
    let mut a = Atlas::new(64, 64 * 64);
    let key = GlyphKey::new(FontId(0), 7, 14.0, 1);
    a.insert(
        key,
        &GlyphBitmap {
            width: 6,
            height: 8,
            left: 0,
            top: 8,
            data: vec![9; 48],
        },
    )
    .unwrap();
    let (s, n) = count(|| a.get(&key));
    assert_eq!(n, 0);
    assert!(s.is_some());
}
