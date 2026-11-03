// SPDX-License-Identifier: BSD-2-Clause
//! Anchored placement: worked examples and properties over random anchors.
mod layout_support;
use layout_support::Rng;
use wolkit::layout::place::*;
use wolkit::layout::tree::{Rect, Size};

const BOUNDS: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 400.0,
    h: 300.0,
};

fn opts(side: Side, align: Align) -> PlaceOpts {
    PlaceOpts {
        side,
        align,
        gap: 4.0,
        flip: true,
        shift: true,
    }
}

#[test]
fn worked_examples() {
    let a = Rect::new(100.0, 100.0, 50.0, 20.0);
    let s = Size::new(80.0, 40.0);
    let p = place(a, s, BOUNDS, opts(Side::Bottom, Align::Start));
    assert_eq!(
        (p.rect.x, p.rect.y, p.side, p.shifted),
        (100.0, 124.0, Side::Bottom, false)
    );
    let p = place(a, s, BOUNDS, opts(Side::Bottom, Align::End));
    assert_eq!(p.rect.x, 70.0);
    let p = place(a, s, BOUNDS, opts(Side::Bottom, Align::Center));
    assert_eq!(p.rect.x, 85.0);
    let p = place(a, s, BOUNDS, opts(Side::Right, Align::Center));
    assert_eq!((p.rect.x, p.rect.y), (154.0, 90.0));
    // Near the bottom edge it flips above.
    let low = Rect::new(100.0, 270.0, 50.0, 20.0);
    let p = place(low, s, BOUNDS, opts(Side::Bottom, Align::Start));
    assert_eq!((p.side, p.rect.y), (Side::Top, 226.0));
    // Flip off: it stays and may leave the bounds on the main axis.
    let p = place(
        low,
        s,
        BOUNDS,
        PlaceOpts {
            flip: false,
            ..opts(Side::Bottom, Align::Start)
        },
    );
    assert_eq!(p.side, Side::Bottom);
    // Near the right edge it shifts back along the anchor.
    let edge = Rect::new(380.0, 100.0, 10.0, 20.0);
    let p = place(edge, s, BOUNDS, opts(Side::Bottom, Align::Start));
    assert_eq!((p.rect.x, p.shifted), (320.0, true));
    let p = place(
        edge,
        s,
        BOUNDS,
        PlaceOpts {
            shift: false,
            ..opts(Side::Bottom, Align::Start)
        },
    );
    assert_eq!((p.rect.x, p.shifted), (380.0, false));
}

#[test]
fn centred_when_there_is_no_anchor() {
    let r = centre(Size::new(100.0, 60.0), BOUNDS);
    assert_eq!((r.x, r.y), (150.0, 120.0));
    let big = centre(Size::new(500.0, 400.0), BOUNDS);
    assert_eq!(
        (big.x, big.y),
        (0.0, 0.0),
        "larger than the bounds: the start edge wins"
    );
}

#[test]
fn properties_over_random_anchors() {
    let mut r = Rng(0xfeed_beef_1234_5671);
    for _ in 0..5000 {
        let a = Rect::new(
            r.below(380) as f32,
            r.below(280) as f32,
            4.0 + r.below(60) as f32,
            4.0 + r.below(40) as f32,
        );
        let a = Rect {
            w: a.w.min(400.0 - a.x),
            h: a.h.min(300.0 - a.y),
            ..a
        };
        let s = Size::new(10.0 + r.below(150) as f32, 10.0 + r.below(120) as f32);
        let side = r.pick(&[Side::Bottom, Side::Top, Side::Right, Side::Left]);
        let align = r.pick(&[Align::Start, Align::Center, Align::End]);
        let p = place(a, s, BOUNDS, opts(side, align));
        let b = p.rect;
        assert_eq!((b.w, b.h), (s.w, s.h), "size is kept");
        // Shift keeps the box inside the bounds along the anchor's axis.
        if p.side.horizontal() {
            assert!(
                b.y >= 0.0 && b.y + b.h <= 300.0,
                "{a:?} {s:?} {side:?}: {b:?}"
            );
        } else {
            assert!(
                b.x >= 0.0 && b.x + b.w <= 400.0,
                "{a:?} {s:?} {side:?}: {b:?}"
            );
        }
        // The box sits on the chosen side, clear of the anchor by the gap.
        match p.side {
            Side::Bottom => assert!(b.y >= a.y + a.h + 4.0 - 0.001),
            Side::Top => assert!(b.y + b.h <= a.y - 4.0 + 0.001),
            Side::Right => assert!(b.x >= a.x + a.w + 4.0 - 0.001),
            Side::Left => assert!(b.x + b.w <= a.x - 4.0 + 0.001),
        }
        // It flips only when the asked side lacks room and the other has more.
        if p.side != side {
            let room = |sd: Side| match sd {
                Side::Bottom => 300.0 - (a.y + a.h) - 4.0,
                Side::Top => a.y - 4.0,
                Side::Right => 400.0 - (a.x + a.w) - 4.0,
                Side::Left => a.x - 4.0,
            };
            let need = if side.horizontal() { s.w } else { s.h };
            assert!(room(side) < need && room(p.side) > room(side));
        }
    }
}

#[test]
fn wire_enums_follow_the_property_table() {
    assert_eq!(Side::from_wire(0), Side::Bottom);
    assert_eq!(Side::from_wire(3), Side::Left);
    assert_eq!(Side::from_wire(99), Side::Bottom);
    assert_eq!(Align::from_wire(2), Align::End);
}
