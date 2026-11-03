// SPDX-License-Identifier: BSD-2-Clause
//! Benchmark for plan budget 5 (style resolve): at most 0.2 ms per 100 dirty nodes, memoised on
//! (parent style, classes, state). Also the cost of compiling and loading a table.
//!
//! `cargo bench -p wolkit --bench style [-- --check]`
#[path = "w3_support/mod.rs"]
mod w3_support;

use wolkit::env::Environment;
use wolkit::scene::patch::{APPEND, Patch, Scratch, Writer};
use wolkit::scene::prop::id;
use wolkit::scene::{Journal, Kind, Scene, SceneWrite, dirty, state};
use wolkit::style::{Styles, Table, compile_sources, resolve, wsb};

const SHEET: &str = "
.row { padding: xs sm; gap: sm; align-items: center; }
.row:hover { background: control; }
.row:hover > .act { opacity: 1; }
.act { opacity: 0; transition: opacity fast ease; }
.title { font-size: title; font-weight: 600; }
.hint { font-size: caption; color: dim; }
.card { background: card; padding: md; border-radius: m; box-shadow: sm; border: 1px line; }
.card:hover { background: control; box-shadow: md; }
@dark { .card { box-shadow: lg; } }
button { cursor: pointer; padding: sm md; border-radius: s; background: control; }
button:hover { background: alpha(accent, 20%); }
button:active { transform: scale(0.98); }
";

fn many_rules(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        let n = name(i);
        s.push_str(&format!(
            ".c{n} {{ padding: sm md; background: card; border-radius: m; opacity: 0.9; }}\n"
        ));
        s.push_str(&format!(".c{n}:hover {{ background: control; }}\n"));
    }
    s
}

fn name(i: usize) -> String {
    let mut s = String::new();
    let mut n = i;
    loop {
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
        if n == 0 {
            return s;
        }
    }
}

fn main() {
    let mut b = w3_support::Bench::new("style");
    let c = compile_sources(&[("bench.wolcss", SHEET)]).unwrap();
    let table = Table::new(c.table);
    let (row, act, title, hint, card) = (
        table.table.class_id("row").unwrap(),
        table.table.class_id("act").unwrap(),
        table.table.class_id("title").unwrap(),
        table.table.class_id("hint").unwrap(),
        table.table.class_id("card").unwrap(),
    );
    // 1000 nodes: 200 rows of (row box, title, hint, action button).
    let mut scene = Scene::new();
    let mut j = Journal::default();
    let mut w = Writer::new();
    let mut n = 1u32;
    let mut rows = vec![];
    for i in 0..200u32 {
        let (r, t, h, a) = (n, n + 1, n + 2, n + 3);
        n += 4;
        w.create(r, Kind::Box)
            .insert(0, r, APPEND)
            .set_style(r, &[if i % 5 == 0 { card } else { row }]);
        w.create(t, Kind::Text)
            .insert(r, t, APPEND)
            .set_style(t, &[title])
            .set_text(t, "Title");
        w.create(h, Kind::Text)
            .insert(r, h, APPEND)
            .set_style(h, &[hint])
            .set_text(h, "hint");
        w.create(a, Kind::Button)
            .insert(r, a, APPEND)
            .set_style(a, &[act])
            .set_text(a, "Go");
        rows.push(r);
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
    resolve(&mut scene, Some(&table), &mut styles, &env, 800);

    // 100 nodes change state each call: hover on, then off. Memo hits, one set_computed each.
    let mut on = false;
    b.time("resolve_100_state_ns", 41, 200, || {
        on = !on;
        for r in rows.iter().take(100) {
            scene.set_engine_state(*r, state::HOVER, on);
        }
        resolve(&mut scene, Some(&table), &mut styles, &env, 800);
    });

    // 100 nodes with inline properties: no memo, a cascade and an intern each.
    let mut k = 0u32;
    b.time("resolve_100_inline_ns", 41, 200, || {
        k += 1;
        let v = wolkit::scene::PropValue::new(&[1, ((k % 4) as f32 * 4.0 + 4.0).to_bits()]);
        for r in rows.iter().skip(100).take(100) {
            scene.set_prop(*r, id::WIDTH, &v);
            scene.mark_dirty(*r, dirty::STYLE);
        }
        resolve(&mut scene, Some(&table), &mut styles, &env, 800);
    });

    // Nothing dirty: the walk stops at the root (the later stages have cleared their bits).
    for i in 0..wolkit::scene::SceneRead::capacity(&scene) {
        scene.clear_dirty(i, dirty::ALL | dirty::SUBTREE);
    }
    b.time("resolve_clean_ns", 41, 20_000, || {
        resolve(&mut scene, Some(&table), &mut styles, &env, 800);
    });

    // A new table or a dark switch restyles all 1000 nodes (through the memo).
    let mut dark = false;
    let mut env2 = env;
    b.time("restyle_all_1000_ns", 21, 20, || {
        dark = !dark;
        env2.dark = dark;
        resolve(&mut scene, Some(&table), &mut styles, &env2, 800);
    });

    // The compiler and the loader.
    let big = many_rules(500);
    b.time("compile_1000_rules_ns", 11, 5, || {
        std::hint::black_box(compile_sources(&[("big.wolcss", &big)]).unwrap());
    });
    let bytes = compile_sources(&[("big.wolcss", &big)]).unwrap().bytes;
    b.value("table_1000_rules_bytes", bytes.len() as f64);
    b.time("load_1000_rules_ns", 21, 20, || {
        std::hint::black_box(wsb::load(&bytes).unwrap());
    });
    b.finish();
}
