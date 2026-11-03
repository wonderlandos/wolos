// SPDX-License-Identifier: BSD-2-Clause
//! Benchmark for plan budget 6 (layout): at most 0.5 ms for a 1k-node relayout boundary and 4 ms
//! for a full relayout of 10k nodes. Relayout boundaries and the measure cache keep an edit local.
//!
//! `cargo bench -p wolkit --bench layout [-- --check]`
#[path = "../tests/support/layout.rs"]
mod layout_support;
#[path = "w3_support/mod.rs"]
mod w3_support;

use layout_support::*;
use wolkit::layout::tree::*;
use wolkit::layout::{LayoutCache, layout};
use wolkit::style::value::*;

/// A list under `parent`: `rows` rows of (box > text, control, box with two children).
fn list(t: &mut TTree, parent: u32, rows: usize) -> Vec<u32> {
    let mut texts = vec![];
    for _ in 0..rows {
        let r = t.add(
            parent,
            LayoutStyle {
                padding: [4.0; 4],
                col_gap: 8.0,
                align_items: AlignItems::Center,
                ..LayoutStyle::default()
            },
        );
        let tx = t.text(
            r,
            LayoutStyle {
                grow: 1.0,
                ..LayoutStyle::default()
            },
            "A row of the list with some text",
        );
        t.control(r, LayoutStyle::default(), Size::new(40.0, 24.0));
        let g = t.add(
            r,
            LayoutStyle {
                direction: Direction::Column,
                row_gap: 4.0,
                ..LayoutStyle::default()
            },
        );
        t.add(
            g,
            LayoutStyle {
                width: Dim::Px(20.0),
                height: Dim::Px(8.0),
                ..LayoutStyle::default()
            },
        );
        t.add(
            g,
            LayoutStyle {
                width: Dim::Px(20.0),
                height: Dim::Px(8.0),
                ..LayoutStyle::default()
            },
        );
        texts.push(tx);
    }
    texts
}

fn column() -> LayoutStyle {
    LayoutStyle {
        direction: Direction::Column,
        ..LayoutStyle::default()
    }
}

/// Text of a fixed size: the engine's own cost without the shaper (the text budget is separate).
struct Fixed<'a>(&'a TTree);

impl Measure for Fixed<'_> {
    fn text(&mut self, _n: u32, max_w: Option<f32>) -> Size {
        Size::new(max_w.map_or(200.0, |m| m.min(200.0)), 18.0)
    }
    fn intrinsic(&mut self, n: u32) -> Size {
        self.0.nodes[n as usize].intrinsic
    }
}

fn main() {
    let mut b = w3_support::Bench::new("layout");

    // ---- a 1k-node boundary: a fixed-size scroll panel of 167 rows (about 1000 nodes).
    let mut t = TTree::default();
    let root = t.add(NIL, column());
    let panel = t.add(
        root,
        LayoutStyle {
            direction: Direction::Column,
            width: Dim::Px(600.0),
            height: Dim::Px(800.0),
            overflow: Overflow::Auto,
            shrink: 0.0,
            ..LayoutStyle::default()
        },
    );
    let texts = list(&mut t, panel, 167);
    let n_nodes = t.nodes.len();
    b.value("boundary_nodes", n_nodes as f64);
    let mut cache = LayoutCache::new();
    layout(
        &t,
        &mut TMeasure(&t),
        &mut cache,
        root,
        Size::new(800.0, 900.0),
        &mut |_, _| {},
    );

    // One text changes inside the boundary.
    let mut flip = false;
    b.time("boundary_1k_edit_ns", 41, 200, || {
        flip = !flip;
        t.nodes[texts[83] as usize].text = if flip {
            "A much longer row of the list with a lot more text in it".into()
        } else {
            "A row of the list with some text".into()
        };
        cache.invalidate_own(&t, texts[83]);
        layout(
            &t,
            &mut TMeasure(&t),
            &mut cache,
            root,
            Size::new(800.0, 900.0),
            &mut |_, _| {},
        );
    });

    // The whole boundary laid out again from nothing (caches forgotten).
    b.time("boundary_1k_full_ns", 41, 50, || {
        cache.invalidate_all();
        layout(
            &t,
            &mut Fixed(&t),
            &mut cache,
            root,
            Size::new(800.0, 900.0),
            &mut |_, _| {},
        );
    });

    // Nothing changed.
    b.time("clean_ns", 41, 20_000, || {
        layout(
            &t,
            &mut Fixed(&t),
            &mut cache,
            root,
            Size::new(800.0, 900.0),
            &mut |_, _| {},
        );
    });

    // ---- 10k nodes, no boundary: a plain column of 1667 rows.
    let mut big = TTree::default();
    let broot = big.add(NIL, column());
    list(&mut big, broot, 1667);
    b.value("full_nodes", big.nodes.len() as f64);
    let mut c2 = LayoutCache::new();
    b.time("full_10k_ns", 21, 5, || {
        c2.invalidate_all();
        layout(
            &big,
            &mut Fixed(&big),
            &mut c2,
            broot,
            Size::new(800.0, 900.0),
            &mut |_, _| {},
        );
    });
    // The same with the stand-in text measure (word wrapping done per call): informational.
    b.time("full_10k_approx_text_ns", 11, 3, || {
        c2.invalidate_all();
        layout(
            &big,
            &mut TMeasure(&big),
            &mut c2,
            broot,
            Size::new(800.0, 900.0),
            &mut |_, _| {},
        );
    });
    // A cold cache, the first frame of a window.
    b.time("first_10k_ns", 11, 3, || {
        let mut c = LayoutCache::new();
        layout(
            &big,
            &mut Fixed(&big),
            &mut c,
            broot,
            Size::new(800.0, 900.0),
            &mut |_, _| {},
        );
    });
    // Resizing the window: every row is asked again with a new width.
    let mut w = 800.0;
    b.time("resize_10k_ns", 21, 5, || {
        w = if w == 800.0 { 760.0 } else { 800.0 };
        layout(
            &big,
            &mut Fixed(&big),
            &mut c2,
            broot,
            Size::new(w, 900.0),
            &mut |_, _| {},
        );
    });

    // ---- the same shape through the scene: styles come from the computed-style store.
    {
        use wolkit::env::Environment;
        use wolkit::scene::patch::{APPEND, Patch, Scratch, Writer};
        use wolkit::scene::prop::{self, id};
        use wolkit::scene::{Journal, Kind, Scene};
        use wolkit::style::{Styles, resolve};
        let mut scene = Scene::new();
        let mut j = Journal::default();
        let mut w = Writer::new();
        w.set_prop(0, id::FLEX_DIRECTION, &[1]);
        let mut n = 1u32;
        for _ in 0..1667 {
            let (r, t, c, g, a1, a2) = (n, n + 1, n + 2, n + 3, n + 4, n + 5);
            n += 6;
            w.create(r, Kind::Box).insert(0, r, APPEND);
            w.set_f32(r, id::PADDING, 4.0)
                .set_f32(r, id::COLUMN_GAP, 8.0);
            w.create(t, Kind::Text)
                .insert(r, t, APPEND)
                .set_text(t, "A row of the list with some text");
            w.set_f32(t, id::GROW, 1.0);
            w.create(c, Kind::Toggle).insert(r, c, APPEND);
            w.create(g, Kind::Box).insert(r, g, APPEND);
            w.set_prop(g, id::FLEX_DIRECTION, &[1]);
            for a in [a1, a2] {
                w.create(a, Kind::Box).insert(g, a, APPEND);
                w.set_prop(a, id::WIDTH, &[prop::DIM_PX, 20f32.to_bits()]);
                w.set_prop(a, id::HEIGHT, &[prop::DIM_PX, 8f32.to_bits()]);
            }
        }
        let bytes = w.finish(1).to_vec();
        scene
            .apply(
                &Patch::parse(&bytes, &mut Scratch::default()).unwrap(),
                &mut j,
            )
            .unwrap();
        let mut styles = Styles::default();
        let env = Environment::default();
        resolve(&mut scene, None, &mut styles, &env, 800);
        let mut sc = LayoutCache::new();
        struct FixedScene;
        impl Measure for FixedScene {
            fn text(&mut self, _n: u32, max_w: Option<f32>) -> Size {
                Size::new(max_w.map_or(200.0, |m| m.min(200.0)), 18.0)
            }
            fn intrinsic(&mut self, _n: u32) -> Size {
                Size::new(40.0, 24.0)
            }
        }
        b.value(
            "scene_nodes",
            wolkit::scene::SceneRead::node_count(&scene) as f64,
        );
        b.time("scene_full_ns", 11, 3, || {
            sc.invalidate_all();
            wolkit::layout::scene::run(
                &mut scene,
                &styles,
                &mut sc,
                Some(&mut FixedScene),
                800.0,
                900.0,
            );
        });
    }
    b.finish();
}
