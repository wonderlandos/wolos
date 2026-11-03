// SPDX-License-Identifier: BSD-2-Clause
//! Virtual list geometry against a naive model, scrolling helpers, and a list laid out in a tree.
mod layout_support;
use layout_support::*;
use wolkit::layout::tree::*;
use wolkit::layout::vlist::*;
use wolkit::layout::{LayoutCache, layout};
use wolkit::style::value::*;

#[test]
fn offsets_match_a_naive_prefix_sum() {
    let mut r = Rng(0x600d_cafe_0000_0001);
    for _ in 0..200 {
        let count = r.below(300) + 1;
        let est = 8.0 + r.below(40) as f32;
        let mut m = VlistModel::new(count, est);
        let mut heights = vec![est; count as usize];
        for _ in 0..r.below(60) {
            let i = r.below(count);
            let h = 4.0 + r.below(80) as f32;
            m.set_measured(i, h);
            heights[i as usize] = h;
        }
        let mut acc = 0.0f64;
        for i in 0..=count {
            let got = m.offset(i);
            assert!(
                (f64::from(got) - acc).abs() < 0.05,
                "offset {i}: {got} vs {acc}"
            );
            if i < count {
                acc += f64::from(heights[i as usize]);
            }
        }
        assert!((f64::from(m.total()) - acc).abs() < 0.05);
        for _ in 0..50 {
            let y = r.below(acc as u32 + 10) as f32;
            let row = m.row_at(y);
            assert!(row < count);
            assert!(
                m.offset(row) <= y + 0.01,
                "row {row} starts at {} after y {y}",
                m.offset(row)
            );
            if row + 1 < count {
                assert!(y < m.offset(row + 1) + 0.01);
            }
        }
    }
}

#[test]
fn visible_range_covers_the_viewport_with_overscan() {
    let m = VlistModel::new(1000, 20.0);
    let v = m.visible(100.0, 100.0, 0);
    assert_eq!(v, 5..10);
    let v = m.visible(100.0, 100.0, 2);
    assert_eq!(v, 3..12);
    assert_eq!(m.visible(0.0, 100.0, 3), 0..8, "clamped at the start");
    assert_eq!(
        m.visible(19_950.0, 100.0, 3),
        994..1000,
        "clamped at the end"
    );
    assert_eq!(VlistModel::new(0, 20.0).visible(0.0, 100.0, 3), 0..0);
    assert_eq!(m.scroll_to(10, 100.0), 200.0);
    assert_eq!(
        m.scroll_to(999, 100.0),
        19_900.0,
        "does not scroll past the end"
    );
}

#[test]
fn a_hundred_thousand_rows_stay_cheap() {
    let mut m = VlistModel::new(100_000, 24.0);
    for i in (0..100_000).step_by(7) {
        m.set_measured(i, 20.0 + (i % 9) as f32);
    }
    let t = std::time::Instant::now();
    let mut acc = 0u64;
    for k in 0..10_000 {
        let y = (k as f32) * 190.0;
        let v = m.visible(y, 800.0, 4);
        acc += u64::from(v.end - v.start);
    }
    assert!(acc > 0);
    assert!(
        t.elapsed().as_millis() < 500,
        "10k range queries took {:?}",
        t.elapsed()
    );
}

#[test]
fn reset_keeps_measurements_and_drops_the_tail() {
    let mut m = VlistModel::new(100, 10.0);
    m.set_measured(5, 50.0);
    m.set_measured(90, 70.0);
    m.reset(100, 10.0);
    assert_eq!(m.offset(6), 5.0 * 10.0 + 50.0, "row 5 is 50 tall");
    m.reset(50, 10.0);
    assert_eq!(m.count(), 50);
    assert_eq!(
        m.total(),
        540.0,
        "the row at 90 is gone, the one at 5 stays"
    );
    m.reset(60, 12.0);
    assert_eq!(m.total(), 720.0, "a new estimate forgets measurements");
}

#[test]
fn scroll_helpers() {
    assert_eq!(clamp_scroll(-5.0, 1000.0, 300.0), 0.0);
    assert_eq!(clamp_scroll(900.0, 1000.0, 300.0), 700.0);
    assert_eq!(clamp_scroll(10.0, 100.0, 300.0), 0.0);
    let (d, v) = fling(1000.0, 0.016, 4.0);
    assert!(d > 0.0 && v < 1000.0 && v > 0.0);
    let (mut pos, mut vel) = (0.0, 2000.0);
    for _ in 0..2000 {
        let (d, nv) = fling(vel, 0.016, 4.0);
        pos += d;
        vel = nv;
    }
    assert_eq!(vel, 0.0, "it comes to rest");
    assert!(
        (pos - 2000.0 / 4.0).abs() < 5.0,
        "total distance is v0 / friction: {pos}"
    );
}

#[test]
fn rows_are_placed_by_their_index_in_a_tree() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            direction: Direction::Column,
            ..LayoutStyle::default()
        },
    );
    let list = t.add(
        root,
        LayoutStyle {
            direction: Direction::Column,
            overflow: Overflow::Auto,
            height: Dim::Px(200.0),
            shrink: 0.0,
            ..LayoutStyle::default()
        },
    );
    t.nodes[list as usize].vlist = Some(VlistInfo {
        count: 1000,
        estimate: 30.0,
    });
    let mut rows = vec![];
    for i in 100..105 {
        let r = t.add(
            list,
            LayoutStyle {
                height: Dim::Px(30.0),
                shrink: 0.0,
                ..LayoutStyle::default()
            },
        );
        t.nodes[r as usize].row = i;
        rows.push(r);
    }
    let mut cache = LayoutCache::new();
    let mut rects = vec![Rect::default(); t.nodes.len()];
    layout(
        &t,
        &mut TMeasure(&t),
        &mut cache,
        root,
        Size::new(300.0, 400.0),
        &mut |n, r| rects[n as usize] = r,
    );
    assert_eq!(
        rects[rows[0] as usize].y, 3000.0,
        "row 100 is 3000 px down the list"
    );
    assert_eq!(rects[rows[4] as usize].y, 3120.0);
    assert_eq!(
        rects[rows[0] as usize].w, 300.0,
        "rows stretch across the list"
    );
    assert_eq!(
        cache.content_size(list).h,
        30_000.0,
        "the extent to scroll over"
    );
    // A taller measured row pushes the ones below it.
    t.style_mut(rows[1]).height = Dim::Px(50.0);
    cache.invalidate_own(&t, rows[1]);
    layout(
        &t,
        &mut TMeasure(&t),
        &mut cache,
        root,
        Size::new(300.0, 400.0),
        &mut |n, r| rects[n as usize] = r,
    );
    assert_eq!(rects[rows[2] as usize].y, 3000.0 + 30.0 + 50.0);
    assert_eq!(cache.content_size(list).h, 30_020.0);
}
