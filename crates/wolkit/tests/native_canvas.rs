// SPDX-License-Identifier: BSD-2-Clause
//! The canvas hook (WK-W4 for WK-W5): a `WKC1` stream submitted for a canvas node is stored on
//! the window (`Engine::set_canvas`) and drawn inside the node by the CPU path (and the GPU path,
//! which lowers with the same `paint::scene::build_with`). Wrong parts are refused with a sentence;
//! a GPU surface needs the native backend.

use wolkit::abi::ENGINE_HEADLESS;
use wolkit::engine::Engine;
use wolkit::gpu::canvas::{Canvas, Encoder};
use wolkit::paint::geom::{Color, Rect};
use wolkit::scene::Kind;
use wolkit::scene::patch::{APPEND, Writer};
use wolkit::scene::prop::{DIM_PX, id};

fn px(v: f32) -> [u32; 2] {
    [DIM_PX, v.to_bits()]
}

#[test]
fn a_submitted_canvas_is_drawn_inside_its_node() {
    let e = Engine::new("org.example.canvas", ENGINE_HEADLESS).unwrap();
    let win = e.create_window("Canvas", 100, 80, 0).unwrap();
    e.show(win).unwrap();
    let mut p = Writer::new();
    p.create(1, Kind::Canvas).insert(0, 1, APPEND);
    p.set_prop(1, id::POSITION, &[1]);
    p.set_prop(1, id::INSET_LEFT, &px(10.0));
    p.set_prop(1, id::INSET_TOP, &px(20.0));
    p.set_prop(1, id::WIDTH, &px(40.0));
    p.set_prop(1, id::HEIGHT, &px(30.0));
    p.create(2, Kind::Box).insert(0, 2, APPEND);
    e.apply(win, p.finish(1)).unwrap();
    let mut enc = Encoder::new();
    // Larger than the node: the part outside is clipped away.
    enc.rect(Rect::new(0.0, 0.0, 200.0, 200.0), Color::rgb(255, 0, 0));
    let c = Canvas::decode(&enc.bytes).unwrap();
    e.set_canvas(win, 1, c).unwrap();
    let mut out = vec![0u8; 100 * 80 * 4];
    e.render_rgba(win, &mut out).unwrap();
    let at = |x: usize, y: usize| &out[(y * 100 + x) * 4..(y * 100 + x) * 4 + 3];
    assert_eq!(at(25, 30), &[255, 0, 0], "inside the node");
    assert_ne!(at(60, 30), &[255, 0, 0], "clipped to the node");
    assert_ne!(at(5, 5), &[255, 0, 0]);
    let again = Canvas::decode(&enc.bytes).unwrap();
    let err = e.set_canvas(win, 2, again).unwrap_err();
    assert!(err.message.contains("not a canvas"), "{}", err.message);
}

#[test]
fn gpu_surfaces_need_the_native_backend() {
    let e = Engine::new("org.example.canvas", ENGINE_HEADLESS).unwrap();
    let win = e.create_window("Surface", 100, 80, 0).unwrap();
    let mut p = Writer::new();
    p.create(1, Kind::Surface).insert(0, 1, APPEND);
    e.apply(win, p.finish(1)).unwrap();
    let err = e.create_subsurface(win, 1, 10, 10).unwrap_err();
    assert_eq!(err.message, wolkit::copy::NO_GPU_SURFACE);
}
