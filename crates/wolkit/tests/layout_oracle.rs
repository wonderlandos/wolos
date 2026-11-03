// SPDX-License-Identifier: BSD-2-Clause
//! Property tests: the engine against the reference oracle on random trees, and incremental layout
//! against a fresh layout after random edits.
#[path = "support/layout.rs"]
mod layout_support;
use layout_support::*;
use wolkit::layout::tree::*;
use wolkit::layout::{LayoutCache, layout};
use wolkit::style::value::*;

fn check_oracle(t: &TTree, vw: f32, vh: f32, seed: u64) {
    let got = run_engine(t, vw, vh);
    let want = Oracle { t }.layout(f64::from(vw), f64::from(vh));
    let seen = reachable(t);
    if let Some(d) = first_diff(t, &want, &got, &|i| seen[i as usize]) {
        panic!("seed {seed:#x}: {d}\ntree: {t:#?}");
    }
}

#[test]
fn engine_matches_oracle_on_random_trees() {
    let mut fails = 0;
    for seed in 1..=3000u64 {
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let t = random_tree(&mut r, 4, 4);
        let vw = r.pick(&[200.0, 320.0, 480.0, 640.0, 800.0]);
        let vh = r.pick(&[120.0, 240.0, 400.0, 600.0]);
        let res = std::panic::catch_unwind(|| check_oracle(&t, vw, vh, seed));
        if res.is_err() {
            fails += 1;
            if fails > 3 {
                break;
            }
        }
    }
    assert_eq!(
        fails, 0,
        "engine and oracle disagree on {fails} trees (first three printed above)"
    );
}

#[test]
fn engine_matches_oracle_on_deeper_trees() {
    for seed in 1..=300u64 {
        let mut r = Rng(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1);
        let t = random_tree(&mut r, 6, 3);
        check_oracle(&t, 480.0, 360.0, seed);
    }
}

/// Apply one random edit; return the nodes whose own inputs changed and those whose children did.
fn edit(t: &mut TTree, r: &mut Rng, cache: &mut LayoutCache) {
    let live: Vec<u32> = (0..t.nodes.len() as u32)
        .filter(|n| *n == 0 || t.nodes[*n as usize].parent != NIL)
        .collect();
    let n = r.pick(&live);
    let which = r.below(6);
    if std::env::var("DBG").is_ok() {
        eprintln!(
            "EDIT node {n} kind {which} parent {} display {:?}",
            t.nodes[n as usize].parent, t.nodes[n as usize].style.display
        );
    }
    match which {
        0 | 1 => {
            // restyle
            let leaf =
                t.nodes[n as usize].leaf != Leaf::None || t.nodes[n as usize].kids.is_empty();
            let mut s = random_style(r, leaf);
            if n == 0 {
                s.display = Display::Flex;
                s.position = Position::Relative;
            }
            *t.style_mut(n) = s;
            cache.invalidate_own(&*t, n);
        }
        2 => {
            // change a single field: padding
            let v = r.pick(&[0.0f32, 4.0, 8.0, 16.0]);
            t.style_mut(n).padding[r.below(4) as usize] = v;
            cache.invalidate_own(&*t, n);
        }
        3 => {
            // add a child
            if t.nodes[n as usize].leaf == Leaf::None {
                let s = random_style(r, true);
                let id = t.text(n, s, "added text here");
                let _ = id;
                cache.invalidate_content(&*t, n);
            }
        }
        4 => {
            // remove a child (never the root)
            if n != 0 {
                let p = t.nodes[n as usize].parent;
                t.detach(n);
                cache.invalidate_content(&*t, p);
            }
        }
        _ => {
            // change text
            if t.nodes[n as usize].leaf == Leaf::Text {
                t.nodes[n as usize].text.push_str(" more words");
                cache.invalidate_own(&*t, n);
            }
        }
    }
}

#[test]
fn incremental_layout_equals_fresh_layout_after_edits() {
    for seed in 1..=400u64 {
        let mut r = Rng(seed.wrapping_mul(0xd6e8_feb8_6659_fd93) | 1);
        let mut t = random_tree(&mut r, 4, 4);
        let (mut vw, mut vh) = (480.0f32, 360.0f32);
        let mut cache = LayoutCache::new();
        let mut seen = vec![Rect::default(); t.nodes.len() + 64];
        let go = |t: &TTree, cache: &mut LayoutCache, vw: f32, vh: f32, seen: &mut Vec<Rect>| {
            if seen.len() < t.nodes.len() {
                seen.resize(t.nodes.len(), Rect::default());
            }
            layout(
                t,
                &mut TMeasure(t),
                cache,
                0,
                Size::new(vw, vh),
                &mut |n, rc| seen[n as usize] = rc,
            );
        };
        go(&t, &mut cache, vw, vh, &mut seen);
        for step in 0..8 {
            if r.chance(15) {
                vw = r.pick(&[240.0, 480.0, 700.0]);
                vh = r.pick(&[200.0, 360.0]);
            } else {
                for _ in 0..r.below(3) + 1 {
                    edit(&mut t, &mut r, &mut cache);
                }
            }
            if std::env::var("DBG").is_ok() {
                eprintln!(
                    "seed {seed:#x} step {step} nodes: {:?}",
                    t.nodes
                        .iter()
                        .map(|n| (n.parent, n.style.display, n.style.position, n.kids.clone()))
                        .collect::<Vec<_>>()
                );
            }
            go(&t, &mut cache, vw, vh, &mut seen);
            let fresh = run_engine(&t, vw, vh);
            let reach = reachable(&t);
            for i in 0..t.nodes.len() {
                if !reach[i] {
                    continue;
                }
                let (a, b) = (seen[i], fresh[i]);
                let same = |x: f32, y: f32| (x - y).abs() < 1e-3;
                assert!(
                    same(a.x, b.x) && same(a.y, b.y) && same(a.w, b.w) && same(a.h, b.h),
                    "seed {seed:#x} step {step} node {i}: incremental {a:?} fresh {b:?}\n{}\nparent: {}",
                    cache.debug_node(i as u32),
                    cache.debug_node(t.nodes[i].parent)
                );
            }
        }
    }
}

#[test]
fn a_clean_second_pass_does_no_work() {
    let mut r = Rng(0x1234_5678_9abc_def1);
    let t = random_tree(&mut r, 5, 4);
    let mut cache = LayoutCache::new();
    layout(
        &t,
        &mut TMeasure(&t),
        &mut cache,
        0,
        Size::new(480.0, 360.0),
        &mut |_, _| {},
    );
    let before = cache.stats;
    let mut changed = 0;
    layout(
        &t,
        &mut TMeasure(&t),
        &mut cache,
        0,
        Size::new(480.0, 360.0),
        &mut |_, _| changed += 1,
    );
    assert_eq!(changed, 0);
    assert_eq!(
        cache.stats.measures, before.measures,
        "nothing is measured again"
    );
    assert_eq!(
        cache.stats.arranges, before.arranges,
        "nothing is arranged again"
    );
}

#[test]
fn a_change_inside_a_boundary_stays_inside_it() {
    // root column > fixed 200x200 panel > row of two texts, plus a sibling that must not be touched.
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            direction: Direction::Column,
            ..LayoutStyle::default()
        },
    );
    let panel = LayoutStyle {
        width: Dim::Px(200.0),
        height: Dim::Px(200.0),
        shrink: 0.0,
        ..LayoutStyle::default()
    };
    let p = t.add(root, panel);
    let a = t.text(p, LayoutStyle::default(), "alpha");
    let _b = t.text(p, LayoutStyle::default(), "beta");
    let sib = t.add(root, LayoutStyle::default());
    for _ in 0..20 {
        t.text(sib, LayoutStyle::default(), "sibling text");
    }
    let mut cache = LayoutCache::new();
    layout(
        &t,
        &mut TMeasure(&t),
        &mut cache,
        root,
        Size::new(400.0, 400.0),
        &mut |_, _| {},
    );
    let before = cache.stats;
    t.nodes[a as usize].text = "alpha alpha alpha".into();
    cache.invalidate_own(&t, a);
    let mut moved = vec![];
    layout(
        &t,
        &mut TMeasure(&t),
        &mut cache,
        root,
        Size::new(400.0, 400.0),
        &mut |n, _| moved.push(n),
    );
    assert!(
        moved
            .iter()
            .all(|n| *n == a || *n == p || t.nodes[*n as usize].parent == p),
        "only the panel's subtree moves: {moved:?}"
    );
    let arranges = cache.stats.arranges - before.arranges;
    assert!(
        arranges <= 3,
        "root skipped, panel and its row re-arranged only: {arranges}"
    );
}

#[test]
fn extreme_styles_never_panic_or_produce_nan() {
    let extremes: &[Dim] = &[
        Dim::Px(0.0),
        Dim::Px(0.001),
        Dim::Px(1.0e7),
        Dim::Pct(0.0),
        Dim::Pct(1000.0),
        Dim::Auto,
    ];
    for seed in 1..=400u64 {
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut t = random_tree(&mut r, 4, 4);
        for n in 0..t.nodes.len() {
            let s = &mut t.nodes[n].style;
            if r.chance(40) {
                s.width = r.pick(extremes);
                s.height = r.pick(extremes);
            }
            if r.chance(20) {
                s.min_width = r.pick(extremes);
                s.max_width = r.pick(extremes);
                s.min_height = r.pick(extremes);
                s.max_height = r.pick(extremes);
            }
            if r.chance(20) {
                s.grow = r.pick(&[0.0, 1000.0, 1.0e-6, 1.0e6]);
                s.shrink = r.pick(&[0.0, 1000.0, 1.0e-6, 1.0e6]);
                s.basis = r.pick(extremes);
            }
            if r.chance(15) {
                s.aspect = r.pick(&[0.05, 20.0, 1.0]);
                s.row_gap = r.pick(&[0.0, 1.0e6]);
                s.col_gap = r.pick(&[0.0, 1.0e6]);
                s.padding = [r.pick(&[0.0, 1.0e6]); 4];
                s.margin = [r.pick(extremes); 4];
            }
        }
        let rects = run_engine(
            &t,
            r.pick(&[0.0, 1.0, 320.0, 4000.0]),
            r.pick(&[0.0, 1.0, 240.0, 4000.0]),
        );
        for (i, rc) in rects.iter().enumerate() {
            assert!(
                rc.x.is_finite() && rc.y.is_finite() && rc.w.is_finite() && rc.h.is_finite(),
                "seed {seed:#x} node {i}: {rc:?}"
            );
            assert!(
                rc.w >= 0.0 && rc.h >= 0.0,
                "seed {seed:#x} node {i}: negative size {rc:?}"
            );
        }
    }
}

/// The generator has to reach the features the oracle is meant to check.
#[test]
fn the_random_trees_exercise_the_features() {
    let mut seen = std::collections::BTreeMap::<&str, usize>::new();
    let mut bump = |k: &'static str| *seen.entry(k).or_default() += 1;
    for seed in 1..=3000u64 {
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let t = random_tree(&mut r, 4, 4);
        let (vw, vh) = (
            r.pick(&[200.0f32, 320.0, 480.0, 640.0, 800.0]),
            r.pick(&[120.0f32, 240.0, 400.0, 600.0]),
        );
        let rects = run_engine(&t, vw, vh);
        for (i, n) in t.nodes.iter().enumerate() {
            let s = &n.style;
            if s.wrap == Wrap::Wrap && n.kids.len() >= 3 {
                // Lines actually broke: two children of a wrapping container share no row/column band.
                let ys: std::collections::BTreeSet<i32> =
                    n.kids.iter().map(|k| rects[*k as usize].y as i32).collect();
                let xs: std::collections::BTreeSet<i32> =
                    n.kids.iter().map(|k| rects[*k as usize].x as i32).collect();
                if ys.len() > 1 && xs.len() > 1 {
                    bump("wrapped onto a second line");
                }
            }
            if matches!(
                s.direction,
                Direction::RowReverse | Direction::ColumnReverse
            ) && n.kids.len() >= 2
            {
                bump("reverse");
            }
            if s.margin.iter().any(|m| matches!(m, Dim::Auto)) {
                bump("auto margin");
            }
            if s.position == Position::Absolute && i > 0 {
                bump("absolute");
            }
            if s.aspect > 0.0 {
                bump("aspect");
            }
            if matches!(s.width, Dim::Pct(_)) || matches!(s.height, Dim::Pct(_)) {
                bump("percent size");
            }
            if s.grow > 0.0 && n.parent != NIL {
                bump("grow");
            }
            if s.overflow == Overflow::Scroll {
                bump("scroll");
            }
            if n.leaf == Leaf::Text && rects[i].h > 25.0 && rects[i].w > 0.0 {
                bump("wrapped text");
            }
            if s.display == Display::None {
                bump("display none");
            }
            if s.border_width > 0.0 {
                bump("border");
            }
            if s.justify == Justify::SpaceBetween && n.kids.len() >= 2 {
                bump("space-between");
            }
        }
    }
    for k in [
        "wrapped onto a second line",
        "reverse",
        "auto margin",
        "absolute",
        "aspect",
        "percent size",
        "grow",
        "scroll",
        "wrapped text",
        "display none",
        "border",
        "space-between",
    ] {
        assert!(
            seen.get(k).copied().unwrap_or(0) >= 100,
            "the generator rarely reaches `{k}`: {seen:?}"
        );
    }
}
