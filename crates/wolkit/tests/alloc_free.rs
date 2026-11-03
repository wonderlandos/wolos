// SPDX-License-Identifier: BSD-2-Clause
//! Budget 11 (plan section 4.1): zero heap allocations in a steady-state apply and frame. A counting
//! allocator counts only on the measuring thread, so other tests and the Kit reader thread do not
//! count. "Steady state" is a window that has applied the same kind of patch before: the arena,
//! the journal, the validator's scratch and the string table already have their room.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use wolkit::abi::ENGINE_HEADLESS;
use wolkit::engine::Engine;
use wolkit::scene::patch::{APPEND, Writer};
use wolkit::scene::prop::id;
use wolkit::scene::{Kind, state};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
}

// SAFETY: forwards every call to the system allocator; the counter is an atomic.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.with(Cell::get) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: same contract as ours.
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // SAFETY: same contract as ours.
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ON.with(Cell::get) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: same contract as ours.
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn counted<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = ALLOCS.load(Ordering::Relaxed);
    ON.with(|o| o.set(true));
    let r = f();
    ON.with(|o| o.set(false));
    (r, ALLOCS.load(Ordering::Relaxed) - before)
}

/// 200 rows of (box > text + button + toggle) under one list box.
fn build(w: &mut Writer) {
    w.create(1, Kind::Box).insert(0, 1, APPEND);
    let mut n = 2;
    for _ in 0..50 {
        let (row, t, b, s) = (n, n + 1, n + 2, n + 3);
        n += 4;
        w.create(row, Kind::Box).create(t, Kind::Text).create(b, Kind::Button).create(s, Kind::Toggle);
        w.insert(1, row, APPEND).insert(row, t, APPEND).insert(row, b, APPEND).insert(row, s, APPEND);
        w.set_text(t, "Row").set_text(b, "Delete");
        w.bind_event(b, 1);
    }
}

/// A patch of the kind an interaction makes: states, a number, a text between two known values,
/// classes. `flip` alternates the values so every apply really changes something.
fn update(w: &mut Writer, flip: bool) {
    for row in 0..50u32 {
        let base = 2 + row * 4;
        let (text, btn, tog) = (base + 1, base + 2, base + 3);
        w.set_state(tog, if flip { state::CHECKED } else { 0 });
        w.set_state(btn, if flip { 0 } else { state::DISABLED });
        w.set_text(text, if flip { "Row A" } else { "Row B" });
        w.set_f32(row + 2, id::GROW, if flip { 1.0 } else { 2.0 });
        w.set_style(btn, if flip { &[1, 2] } else { &[3] });
    }
}

#[test]
fn steady_state_apply_and_frame_allocate_nothing() {
    let engine = Engine::new("org.example.alloc", ENGINE_HEADLESS).unwrap();
    let win = engine.create_window("Alloc", 400, 600, 0).unwrap();
    engine.show(win).unwrap();
    let mut w = Writer::new();
    build(&mut w);
    engine.apply(win, w.finish(1)).unwrap();
    w.clear();
    // The two update patches, built once: building is the app's allocation, not the engine's.
    let (mut a, mut b) = (Writer::new(), Writer::new());
    update(&mut a, true);
    update(&mut b, false);
    let (pa, pb) = (a.finish(2).to_vec(), b.finish(3).to_vec());
    // Warm up: every buffer reaches its steady size.
    for _ in 0..8 {
        engine.apply(win, &pa).unwrap();
        engine.apply(win, &pb).unwrap();
        engine.advance_time(win, 16_000_000).unwrap();
    }
    let (_, apply_allocs) = counted(|| {
        for _ in 0..100 {
            engine.apply(win, &pa).unwrap();
            engine.apply(win, &pb).unwrap();
        }
    });
    assert_eq!(apply_allocs, 0, "apply allocated in steady state");
    let (_, frame_allocs) = counted(|| {
        for _ in 0..100 {
            engine.advance_time(win, 16_000_000).unwrap();
        }
    });
    assert_eq!(frame_allocs, 0, "a frame allocated in steady state");
    let (_, both) = counted(|| {
        for _ in 0..100 {
            engine.apply(win, &pa).unwrap();
            engine.advance_time(win, 16_000_000).unwrap();
            engine.apply(win, &pb).unwrap();
            engine.advance_time(win, 16_000_000).unwrap();
        }
    });
    assert_eq!(both, 0, "apply plus frame allocated in steady state");
}
