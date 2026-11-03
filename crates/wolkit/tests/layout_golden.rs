// SPDX-License-Identifier: BSD-2-Clause
//! Hand-worked layouts (the numbers are what CSS flexbox gives for the same boxes, worked on
//! paper), so the oracle and the engine are not only held to each other.
mod layout_support;
use layout_support::*;
use wolkit::layout::tree::*;
use wolkit::style::value::*;

fn st() -> LayoutStyle {
    LayoutStyle::default()
}
fn fixed(w: f32, h: f32) -> LayoutStyle {
    LayoutStyle {
        width: Dim::Px(w),
        height: Dim::Px(h),
        shrink: 0.0,
        ..st()
    }
}
fn rect(r: Rect) -> (f32, f32, f32, f32) {
    (r.x, r.y, r.w, r.h)
}
fn assert_rect(got: Rect, want: (f32, f32, f32, f32), what: &str) {
    let (x, y, w, h) = rect(got);
    let ok = (x - want.0).abs() < 0.01
        && (y - want.1).abs() < 0.01
        && (w - want.2).abs() < 0.01
        && (h - want.3).abs() < 0.01;
    assert!(ok, "{what}: got {:?} want {:?}", rect(got), want);
}
fn both(t: &TTree, vw: f32, vh: f32) -> Vec<Rect> {
    let e = run_engine(t, vw, vh);
    let o = Oracle { t }.layout(f64::from(vw), f64::from(vh));
    for (i, r) in o.iter().enumerate() {
        assert!(
            close(r.0, e[i].x) && close(r.1, e[i].y) && close(r.2, e[i].w) && close(r.3, e[i].h),
            "oracle and engine differ at {i}: {r:?} vs {:?}",
            e[i]
        );
    }
    e
}

#[test]
fn grow_shares_the_free_space() {
    let mut t = TTree::default();
    let root = t.add(NIL, st());
    t.add(root, fixed(50.0, 20.0));
    t.add(root, LayoutStyle { grow: 1.0, ..st() });
    t.add(root, LayoutStyle { grow: 2.0, ..st() });
    let r = both(&t, 300.0, 100.0);
    assert_rect(r[1], (0.0, 0.0, 50.0, 20.0), "fixed");
    assert_rect(r[2], (50.0, 0.0, 250.0 / 3.0, 100.0), "grow 1 (stretched)");
    assert_rect(
        r[3],
        (50.0 + 250.0 / 3.0, 0.0, 500.0 / 3.0, 100.0),
        "grow 2",
    );
}

#[test]
fn shrink_is_weighted_by_base_size() {
    let mut t = TTree::default();
    let root = t.add(NIL, st());
    t.add(
        root,
        LayoutStyle {
            basis: Dim::Px(100.0),
            shrink: 1.0,
            ..st()
        },
    );
    t.add(
        root,
        LayoutStyle {
            basis: Dim::Px(50.0),
            shrink: 2.0,
            ..st()
        },
    );
    let r = both(&t, 100.0, 40.0);
    // free space -50, scaled factors 100 and 100: each gives up 25.
    assert_rect(r[1], (0.0, 0.0, 75.0, 40.0), "first");
    assert_rect(r[2], (75.0, 0.0, 25.0, 40.0), "second");
}

#[test]
fn max_width_freezes_and_the_rest_takes_the_space() {
    let mut t = TTree::default();
    let root = t.add(NIL, st());
    t.add(
        root,
        LayoutStyle {
            grow: 1.0,
            max_width: Dim::Px(50.0),
            ..st()
        },
    );
    t.add(root, LayoutStyle { grow: 1.0, ..st() });
    let r = both(&t, 200.0, 30.0);
    assert_rect(r[1], (0.0, 0.0, 50.0, 30.0), "capped");
    assert_rect(r[2], (50.0, 0.0, 150.0, 30.0), "takes the rest");
}

#[test]
fn gaps_and_justify() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            col_gap: 10.0,
            justify: Justify::SpaceBetween,
            ..st()
        },
    );
    for _ in 0..3 {
        t.add(root, fixed(20.0, 10.0));
    }
    let r = both(&t, 200.0, 50.0);
    assert_rect(r[1], (0.0, 0.0, 20.0, 10.0), "first");
    assert_rect(r[2], (90.0, 0.0, 20.0, 10.0), "middle");
    assert_rect(r[3], (180.0, 0.0, 20.0, 10.0), "last");
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            justify: Justify::SpaceEvenly,
            ..st()
        },
    );
    for _ in 0..3 {
        t.add(root, fixed(20.0, 10.0));
    }
    let r = both(&t, 200.0, 50.0);
    // free 140, four gaps of 35.
    assert_rect(r[1], (35.0, 0.0, 20.0, 10.0), "evenly 1");
    assert_rect(r[3], (145.0, 0.0, 20.0, 10.0), "evenly 3");
}

#[test]
fn wrapping_breaks_lines_and_stacks_them() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            wrap: Wrap::Wrap,
            col_gap: 10.0,
            row_gap: 6.0,
            align_content: AlignContent::Start,
            align_items: AlignItems::Start,
            ..st()
        },
    );
    for _ in 0..3 {
        t.add(root, fixed(40.0, 20.0));
    }
    let r = both(&t, 100.0, 200.0);
    assert_rect(r[1], (0.0, 0.0, 40.0, 20.0), "line 1 a");
    assert_rect(r[2], (50.0, 0.0, 40.0, 20.0), "line 1 b");
    assert_rect(r[3], (0.0, 26.0, 40.0, 20.0), "line 2");
}

#[test]
fn auto_margins_centre_and_push() {
    let mut t = TTree::default();
    let root = t.add(NIL, st());
    let mut s = fixed(20.0, 20.0);
    s.margin[1] = Dim::Auto;
    s.margin[3] = Dim::Auto;
    s.margin[0] = Dim::Auto;
    s.margin[2] = Dim::Auto;
    t.add(root, s);
    let r = both(&t, 100.0, 100.0);
    assert_rect(r[1], (40.0, 40.0, 20.0, 20.0), "centred by auto margins");
    let mut t = TTree::default();
    let root = t.add(NIL, st());
    t.add(root, fixed(20.0, 20.0));
    let mut s = fixed(20.0, 20.0);
    s.margin[3] = Dim::Auto;
    t.add(root, s);
    let r = both(&t, 100.0, 20.0);
    assert_rect(r[2], (80.0, 0.0, 20.0, 20.0), "pushed to the end");
}

#[test]
fn cross_axis_alignment() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            align_items: AlignItems::Center,
            ..st()
        },
    );
    t.add(root, fixed(20.0, 20.0));
    t.add(
        root,
        LayoutStyle {
            align_self: AlignSelf::End,
            ..fixed(20.0, 30.0)
        },
    );
    t.add(
        root,
        LayoutStyle {
            align_self: AlignSelf::Stretch,
            width: Dim::Px(10.0),
            shrink: 0.0,
            ..st()
        },
    );
    let r = both(&t, 100.0, 100.0);
    assert_rect(r[1], (0.0, 40.0, 20.0, 20.0), "centre");
    assert_rect(r[2], (20.0, 70.0, 20.0, 30.0), "end");
    assert_rect(r[3], (40.0, 0.0, 10.0, 100.0), "stretch");
}

#[test]
fn reverse_directions_mirror() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            direction: Direction::RowReverse,
            ..st()
        },
    );
    t.add(root, fixed(20.0, 10.0));
    t.add(root, fixed(30.0, 10.0));
    let r = both(&t, 100.0, 10.0);
    assert_rect(r[1], (80.0, 0.0, 20.0, 10.0), "first sits at the right");
    assert_rect(r[2], (50.0, 0.0, 30.0, 10.0), "second to its left");
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            direction: Direction::ColumnReverse,
            justify: Justify::End,
            ..st()
        },
    );
    t.add(root, fixed(10.0, 20.0));
    t.add(root, fixed(10.0, 30.0));
    let r = both(&t, 10.0, 100.0);
    // main-start is the bottom, main-end the top: `end` packs at the top, the first item below the second.
    assert_rect(r[2], (0.0, 0.0, 10.0, 30.0), "second item on top");
    assert_rect(r[1], (0.0, 30.0, 10.0, 20.0), "first item under it");
}

#[test]
fn padding_border_and_percent() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            padding: [10.0, 10.0, 10.0, 10.0],
            border_width: 2.0,
            ..st()
        },
    );
    t.add(
        root,
        LayoutStyle {
            width: Dim::Pct(50.0),
            ..st()
        },
    );
    let r = both(&t, 200.0, 100.0);
    // inner box 176 x 76; 50% of 176.
    assert_rect(r[1], (12.0, 12.0, 88.0, 76.0), "percent child");
}

#[test]
fn absolute_children_use_insets() {
    let mut t = TTree::default();
    let root = t.add(NIL, st());
    let mut s = fixed(20.0, 10.0);
    s.position = Position::Absolute;
    s.inset[1] = Dim::Px(10.0);
    s.inset[2] = Dim::Px(5.0);
    t.add(root, s);
    let mut s = LayoutStyle {
        position: Position::Absolute,
        ..st()
    };
    s.inset = [Dim::Px(4.0), Dim::Px(4.0), Dim::Px(4.0), Dim::Px(4.0)];
    t.add(root, s);
    let r = both(&t, 100.0, 100.0);
    assert_rect(r[1], (70.0, 85.0, 20.0, 10.0), "right and bottom");
    assert_rect(r[2], (4.0, 4.0, 92.0, 92.0), "four insets stretch");
}

#[test]
fn aspect_ratio_fills_the_open_side() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            align_items: AlignItems::Start,
            ..st()
        },
    );
    t.add(
        root,
        LayoutStyle {
            width: Dim::Px(100.0),
            aspect: 2.0,
            shrink: 0.0,
            ..st()
        },
    );
    let r = both(&t, 300.0, 300.0);
    assert_rect(r[1], (0.0, 0.0, 100.0, 50.0), "aspect");
}

#[test]
fn text_wraps_at_the_room_it_gets() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            direction: Direction::Column,
            align_items: AlignItems::Start,
            ..st()
        },
    );
    // 16 px text: 8 px per character, 21 px per line.
    let n = t.text(
        root,
        LayoutStyle {
            width: Dim::Px(50.0),
            shrink: 0.0,
            ..st()
        },
        "hello world",
    );
    let m = t.text(root, st(), "hello world");
    let r = both(&t, 200.0, 200.0);
    assert_rect(r[n as usize], (0.0, 0.0, 50.0, 42.0), "two lines in 50 px");
    assert_rect(
        r[m as usize],
        (0.0, 42.0, 88.0, 21.0),
        "one line when there is room",
    );
}

#[test]
fn scroll_containers_do_not_shrink_their_children() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            direction: Direction::Column,
            overflow: Overflow::Auto,
            ..st()
        },
    );
    for _ in 0..3 {
        t.add(
            root,
            LayoutStyle {
                width: Dim::Px(50.0),
                height: Dim::Px(60.0),
                ..st()
            },
        );
    }
    let r = both(&t, 100.0, 100.0);
    assert_rect(
        r[3],
        (0.0, 120.0, 50.0, 60.0),
        "third row runs past the bottom",
    );
    let mut cache = wolkit::layout::LayoutCache::new();
    wolkit::layout::layout(
        &t,
        &mut TMeasure(&t),
        &mut cache,
        0,
        Size::new(100.0, 100.0),
        &mut |_, _| {},
    );
    assert_eq!(cache.content_size(0).h, 180.0, "the extent to scroll over");
    // The same column without overflow shrinks them to fit.
    t.style_mut(0).overflow = Overflow::Visible;
    let r = both(&t, 100.0, 100.0);
    assert!(r[3].y < 120.0);
}

#[test]
fn display_none_takes_no_room() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            col_gap: 10.0,
            ..st()
        },
    );
    t.add(root, fixed(20.0, 10.0));
    t.add(
        root,
        LayoutStyle {
            display: Display::None,
            ..fixed(99.0, 99.0)
        },
    );
    t.add(root, fixed(20.0, 10.0));
    let r = both(&t, 100.0, 10.0);
    assert_rect(r[3], (30.0, 0.0, 20.0, 10.0), "the gap is not doubled");
}

#[test]
fn content_sized_containers_wrap_their_children() {
    let mut t = TTree::default();
    let root = t.add(
        NIL,
        LayoutStyle {
            direction: Direction::Column,
            align_items: AlignItems::Start,
            ..st()
        },
    );
    let card = t.add(
        root,
        LayoutStyle {
            direction: Direction::Column,
            padding: [8.0; 4],
            row_gap: 4.0,
            ..st()
        },
    );
    t.add(card, fixed(60.0, 10.0));
    t.add(card, fixed(40.0, 20.0));
    let r = both(&t, 200.0, 200.0);
    assert_rect(
        r[card as usize],
        (0.0, 0.0, 76.0, 50.0),
        "8 + 60 + 8 wide, 8 + 10 + 4 + 20 + 8 high",
    );
}
