// SPDX-License-Identifier: BSD-2-Clause
//! Plain geometry and colour types shared by paint, the CPU rasteriser and the batcher.
//!
//! Everything here is `Copy` data with no allocation. Coordinates are logical `f32` pixels (the
//! scale to device pixels is applied by the rasteriser).

/// An axis-aligned rectangle: origin and size. Width and height are never negative after
/// [`Rect::new`].
use crate::num::{ceil_i32, floor_i32};
use crate::scene::ids::to_u8;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    /// A rectangle; a negative size is clamped to zero and a NaN becomes zero.
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        let f = |v: f32| if v.is_finite() { v } else { 0.0 };
        Rect {
            x: f(x),
            y: f(y),
            w: f(w).max(0.0),
            h: f(h).max(0.0),
        }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    /// The smallest rectangle holding both; an empty operand is ignored.
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    /// The overlap, or an empty rectangle at the origin of the clip when they do not touch.
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            Rect {
                x: x0,
                y: y0,
                w: 0.0,
                h: 0.0,
            }
        } else {
            Rect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            }
        }
    }

    /// True when the two share any area (touching edges do not count).
    pub fn overlaps(&self, o: &Rect) -> bool {
        !self.is_empty()
            && !o.is_empty()
            && self.x < o.right()
            && o.x < self.right()
            && self.y < o.bottom()
            && o.y < self.bottom()
    }

    pub fn contains(&self, o: &Rect) -> bool {
        o.is_empty()
            || (o.x >= self.x
                && o.y >= self.y
                && o.right() <= self.right()
                && o.bottom() <= self.bottom())
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            w: self.w,
            h: self.h,
        }
    }

    pub fn inflate(&self, d: f32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2.0 * d, self.h + 2.0 * d)
    }

    pub fn area(&self) -> f32 {
        self.w * self.h
    }

    /// The covering integer pixel rectangle (floor of the origin, ceil of the far edge).
    pub fn to_pixels(&self) -> PixelRect {
        if self.is_empty() {
            return PixelRect::default();
        }
        let x0 = floor_i32(self.x.clamp(-1.0e9, 1.0e9));
        let y0 = floor_i32(self.y.clamp(-1.0e9, 1.0e9));
        let x1 = ceil_i32(self.right().clamp(-1.0e9, 1.0e9));
        let y1 = ceil_i32(self.bottom().clamp(-1.0e9, 1.0e9));
        PixelRect { x0, y0, x1, y1 }
    }
}

/// An integer pixel rectangle `[x0, x1) x [y0, y1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl PixelRect {
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn width(&self) -> i32 {
        (self.x1 - self.x0).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.y1 - self.y0).max(0)
    }

    pub fn intersect(&self, o: &PixelRect) -> PixelRect {
        PixelRect {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }

    pub fn area(&self) -> u64 {
        u64::from(self.width().unsigned_abs()) * u64::from(self.height().unsigned_abs())
    }
}

/// A straight (non-premultiplied) RGBA colour, 8 bits per channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// Packs as `0xAABBGGRR` little-endian bytes `[r, g, b, a]` (the GPU instance layout).
    pub const fn pack(self) -> u32 {
        u32::from_le_bytes([self.r, self.g, self.b, self.a])
    }

    pub const fn unpack(v: u32) -> Color {
        let b = v.to_le_bytes();
        Color {
            r: b[0],
            g: b[1],
            b: b[2],
            a: b[3],
        }
    }

    /// Premultiplied components, rounded exactly with [`div255`].
    pub fn premul(self) -> [u8; 4] {
        let a = u32::from(self.a);
        [
            to_u8(div255(u32::from(self.r) * a)),
            to_u8(div255(u32::from(self.g) * a)),
            to_u8(div255(u32::from(self.b) * a)),
            self.a,
        ]
    }

    /// This colour with its alpha scaled by `k / 255`.
    pub fn with_opacity(self, k: u8) -> Color {
        Color {
            a: to_u8(div255(u32::from(self.a) * u32::from(k))),
            ..self
        }
    }
}

/// `round(x / 255)` for `x <= 255 * 255 + 255`, in integers (bit-exact on every machine).
#[inline]
pub const fn div255(x: u32) -> u32 {
    let t = x + 128;
    (t + (t >> 8)) >> 8
}

/// Linear interpolation of two colours in straight space; `t` is `0..=255`.
pub fn mix(a: Color, b: Color, t: u8) -> Color {
    let t = u32::from(t);
    let lerp = |x: u8, y: u8| to_u8(div255(u32::from(x) * (255 - t) + u32::from(y) * t));
    Color {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: lerp(a.a, b.a),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div255_is_exact_rounding() {
        for x in 0..=(255 * 255 + 255u32) {
            let want = (x as f64 / 255.0).round() as u32;
            assert_eq!(div255(x), want, "x={x}");
        }
    }

    #[test]
    fn union_intersect() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert_eq!(a.intersect(&b), Rect::new(5.0, 5.0, 5.0, 5.0));
        assert!(a.intersect(&Rect::new(20.0, 20.0, 1.0, 1.0)).is_empty());
        assert_eq!(Rect::ZERO.union(&a), a);
    }

    #[test]
    fn pack_roundtrip() {
        let c = Color::rgba(1, 2, 3, 4);
        assert_eq!(Color::unpack(c.pack()), c);
    }
}
