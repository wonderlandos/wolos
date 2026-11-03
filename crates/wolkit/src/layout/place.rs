// SPDX-License-Identifier: BSD-2-Clause
//! Anchored placement for popovers, menus and tooltips: put a box of a given size on one side of an
//! anchor rectangle, aligned to it, flipped to the other side when it does not fit and shifted back
//! inside the bounds. Pure arithmetic; `tests/layout_place.rs` holds it to its properties.
use super::tree::{Rect, Size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Bottom,
    Top,
    Right,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Bottom => Side::Top,
            Side::Top => Side::Bottom,
            Side::Right => Side::Left,
            Side::Left => Side::Right,
        }
    }
    pub fn horizontal(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }
    /// From the wire enum order of `anchor-side`: bottom, top, right, left.
    pub fn from_wire(v: u32) -> Side {
        [Side::Bottom, Side::Top, Side::Right, Side::Left]
            .get(v as usize)
            .copied()
            .unwrap_or(Side::Bottom)
    }
}

impl Align {
    /// From the wire enum order of `anchor-align`: start, center, end.
    pub fn from_wire(v: u32) -> Align {
        [Align::Start, Align::Center, Align::End]
            .get(v as usize)
            .copied()
            .unwrap_or(Align::Start)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub rect: Rect,
    /// The side it ended up on (after a flip).
    pub side: Side,
    /// The box was moved along the anchor to stay inside the bounds.
    pub shifted: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PlaceOpts {
    pub side: Side,
    pub align: Align,
    /// Space between anchor and box.
    pub gap: f32,
    pub flip: bool,
    pub shift: bool,
}

impl Default for PlaceOpts {
    fn default() -> Self {
        PlaceOpts {
            side: Side::Bottom,
            align: Align::Start,
            gap: 4.0,
            flip: true,
            shift: true,
        }
    }
}

fn along(anchor_lo: f32, anchor_len: f32, len: f32, a: Align) -> f32 {
    match a {
        Align::Start => anchor_lo,
        Align::Center => anchor_lo + (anchor_len - len) / 2.0,
        Align::End => anchor_lo + anchor_len - len,
    }
}

fn at(anchor: &Rect, size: Size, side: Side, align: Align, gap: f32) -> Rect {
    let (x, y) = match side {
        Side::Bottom => (
            along(anchor.x, anchor.w, size.w, align),
            anchor.y + anchor.h + gap,
        ),
        Side::Top => (
            along(anchor.x, anchor.w, size.w, align),
            anchor.y - gap - size.h,
        ),
        Side::Right => (
            anchor.x + anchor.w + gap,
            along(anchor.y, anchor.h, size.h, align),
        ),
        Side::Left => (
            anchor.x - gap - size.w,
            along(anchor.y, anchor.h, size.h, align),
        ),
    };
    Rect::new(x, y, size.w, size.h)
}

/// Room on the main axis beyond the anchor on `side`, inside `bounds`.
fn room(anchor: &Rect, bounds: &Rect, side: Side, gap: f32) -> f32 {
    match side {
        Side::Bottom => bounds.y + bounds.h - (anchor.y + anchor.h) - gap,
        Side::Top => anchor.y - bounds.y - gap,
        Side::Right => bounds.x + bounds.w - (anchor.x + anchor.w) - gap,
        Side::Left => anchor.x - bounds.x - gap,
    }
}

/// Place a box of `size` against `anchor`, inside `bounds`.
///
/// 1. Try `opts.side`. If it does not fit and `flip` is on and the opposite side has more room,
///    use the opposite side.
/// 2. Align along the anchor as asked.
/// 3. With `shift` on, slide the box along the anchor's edge so it lies inside the bounds (the
///    start edge wins when the box is larger than the bounds).
pub fn place(anchor: Rect, size: Size, bounds: Rect, opts: PlaceOpts) -> Placement {
    let need = if opts.side.horizontal() {
        size.w
    } else {
        size.h
    };
    let mut side = opts.side;
    if opts.flip {
        let here = room(&anchor, &bounds, side, opts.gap);
        let there = room(&anchor, &bounds, side.opposite(), opts.gap);
        if here < need && there > here {
            side = side.opposite();
        }
    }
    let mut r = at(&anchor, size, side, opts.align, opts.gap);
    let mut shifted = false;
    if opts.shift {
        if side.horizontal() {
            let y = r.y.min(bounds.y + bounds.h - r.h).max(bounds.y);
            shifted = y != r.y;
            r.y = y;
        } else {
            let x = r.x.min(bounds.x + bounds.w - r.w).max(bounds.x);
            shifted = x != r.x;
            r.x = x;
        }
    }
    Placement {
        rect: r,
        side,
        shifted,
    }
}

/// A box with no anchor (a dialog): centred in the bounds, kept inside.
pub fn centre(size: Size, bounds: Rect) -> Rect {
    let x = (bounds.x + (bounds.w - size.w) / 2.0).max(bounds.x);
    let y = (bounds.y + (bounds.h - size.h) / 2.0).max(bounds.y);
    Rect::new(x, y, size.w, size.h)
}
