// SPDX-License-Identifier: BSD-2-Clause
//! Budget 11 for the style and layout stages: once a window is warm, restyling and relayout after
//! an interaction (state flips, text and size changes, resize) allocate nothing.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use wolkit::env::Environment;
use wolkit::layout::LayoutCache;
use wolkit::scene::patch::{APPEND, Patch, Scratch, Writer};
use wolkit::scene::prop::{self, id};
use wolkit::scene::{Journal, Kind, Scene, SceneWrite, state};
use wolkit::style::{Styles, Table, compile_sources, resolve};

struct Counting;

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    if ON.with(Cell::get) {
        COUNT.with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: forwards every call to the system allocator; the counter is thread local.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        // SAFETY: same contract as ours.
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // SAFETY: same contract as ours.
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count();
        // SAFETY: same contract as ours.
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn counted<R>(f: impl FnOnce() -> R) -> (R, usize) {
    COUNT.with(|c| c.set(0));
    ON.with(|o| o.set(true));
    let r = f();
    ON.with(|o| o.set(false));
    (r, COUNT.with(Cell::get))
}

#[test]
fn steady_state_frames_allocate_nothing() {
    let sheet = ".row { padding: xs sm; gap: sm; } .row:hover { background: control; } .row:hover > .act { opacity: 1; } \
                 .act { opacity: 0; } .label { font-size: secondary; }";
    let c = compile_sources(&[("t.wolcss", sheet)]).unwrap();
    let table = Table::new(c.table);
    let (row, act, label) = (
        table.table.class_id("row").unwrap(),
        table.table.class_id("act").unwrap(),
        table.table.class_id("label").unwrap(),
    );
    let mut scene = Scene::new();
    let mut j = Journal::default();
    let mut w = Writer::new();
    w.set_prop(0, id::FLEX_DIRECTION, &[1]);
    let mut n = 1;
    for _ in 0..50u32 {
        let (r, t, a, v) = (n, n + 1, n + 2, n + 3);
        n += 4;
        w.create(r, Kind::Box)
            .insert(0, r, APPEND)
            .set_style(r, &[row]);
        w.create(t, Kind::Text)
            .insert(r, t, APPEND)
            .set_text(t, "Row title")
            .set_style(t, &[label]);
        w.create(a, Kind::Button)
            .insert(r, a, APPEND)
            .set_text(a, "Delete")
            .set_style(a, &[act]);
        w.create(v, Kind::Box).insert(r, v, APPEND);
        w.set_prop(v, id::WIDTH, &[prop::DIM_PX, 24f32.to_bits()]);
        w.set_prop(v, id::HEIGHT, &[prop::DIM_PX, 24f32.to_bits()]);
    }
    let bytes = w.finish(1).to_vec();
    scene
        .apply(
            &Patch::parse(&bytes, &mut Scratch::default()).unwrap(),
            &mut j,
        )
        .unwrap();
    let mut styles = Styles::default();
    let mut cache = LayoutCache::new();
    let env = Environment::default();
    let mut frame = |scene: &mut Scene, width: u32| {
        resolve(scene, Some(&table), &mut styles, &env, width);
        wolkit::layout::scene::run(scene, &styles, &mut cache, None, width as f32, 900.0);
    };
    frame(&mut scene, 400);
    // Warm every path once: hover on and off, a text change, a size change.
    for round in 0..2 {
        scene.set_engine_state(1, state::HOVER, round == 0);
        frame(&mut scene, 400);
        let bytes = {
            let mut w = Writer::new();
            w.set_text(
                2,
                if round == 0 {
                    "A longer title for this row"
                } else {
                    "Row title"
                },
            );
            w.set_prop(
                4,
                id::WIDTH,
                &[prop::DIM_PX, (24.0 + round as f32).to_bits()],
            );
            w.finish(2 + round).to_vec()
        };
        scene
            .apply(
                &Patch::parse(&bytes, &mut Scratch::default()).unwrap(),
                &mut j,
            )
            .unwrap();
        frame(&mut scene, 400);
        frame(&mut scene, 380 + round);
    }
    // Now measure. The patches are built outside the counted region.
    let mut patches = vec![];
    for k in 0..20u32 {
        let mut w = Writer::new();
        w.set_text(
            2 + 4 * (k % 50),
            if k % 2 == 0 {
                "A longer title for this row"
            } else {
                "Row title"
            },
        );
        w.set_prop(
            4 + 4 * (k % 50),
            id::WIDTH,
            &[prop::DIM_PX, (24.0 + (k % 2) as f32).to_bits()],
        );
        patches.push(w.finish(10 + k).to_vec());
    }
    for (k, p) in patches.iter().enumerate() {
        scene
            .apply(&Patch::parse(p, &mut Scratch::default()).unwrap(), &mut j)
            .unwrap();
        let hover_row = 1 + 4 * (k as u32 % 50);
        let (_, allocs) = counted(|| {
            scene.set_engine_state(hover_row, state::HOVER, true);
            frame(&mut scene, 400 + (k as u32 % 2) * 20);
            scene.set_engine_state(hover_row, state::HOVER, false);
            frame(&mut scene, 400);
        });
        // Rows share their classes, so the memo already holds every style after the warm-up.
        if k >= 10 {
            assert_eq!(
                allocs, 0,
                "round {k}: restyle and relayout allocated {allocs} times"
            );
        }
    }
    let (_, probe) = counted(|| std::hint::black_box(vec![1u8; 64]));
    assert!(probe > 0, "the counter is live");
}
