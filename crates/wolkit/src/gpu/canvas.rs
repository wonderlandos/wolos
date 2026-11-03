// SPDX-License-Identifier: BSD-2-Clause
//! The canvas op stream (`wolkit_window_canvas_submit`), format `WKC1`.
//!
//! A canvas node's drawing arrives as one byte stream per frame. It is decoded into display-list
//! items once, so the CPU rasteriser and the GPU batcher draw it with the same code as the rest of
//! the window (one decoder, no second renderer). Coordinates are node-local logical pixels;
//! [`Canvas::emit`] translates them to the node's rectangle and clips to it.
//!
//! Layout: the magic `WKC1`, then ops back to back. Each op is one opcode byte and a fixed
//! little-endian payload (`f32` numbers, `u32` colours packed `0xAABBGGRR`, i.e. bytes r g b a):
//!
//! | op | name | payload |
//! |---|---|---|
//! | 1 | `RECT` | x y w h, colour |
//! | 2 | `RRECT` | x y w h, r_tl r_tr r_br r_bl, colour |
//! | 3 | `BORDER` | x y w h, 4 radii, width, colour |
//! | 4 | `LINEAR` | x y w h, 4 radii, x0 y0 x1 y1 (fractions of the box), colour0, colour1 |
//! | 5 | `SHADOW` | x y w h, 4 radii, blur spread dx dy, colour |
//! | 6 | `PUSH_CLIP` | x y w h, radius |
//! | 7 | `POP_CLIP` | none |
//! | 8 | `PUSH_OPACITY` | alpha `u8`, overlap `u8` (1: children overlap, draw as a layer) |
//! | 9 | `POP_OPACITY` | none |
//!
//! Every number must be finite and within +-1,000,000; sizes are not negative; pushes and pops
//! balance and nest at most 64 deep; at most 65,536 ops per stream. A stream that breaks a rule
//! is refused as a whole, with one sentence.

use super::copy;
use crate::error::{Error, Result};
use crate::paint::geom::{Color, Rect};
use crate::paint::list::{DisplayList, Fill, Item, Radii, Shadow};

pub const MAGIC: &[u8; 4] = b"WKC1";
pub const MAX_OPS: usize = 65_536;
pub const MAX_DEPTH: usize = 64;
const MAX_COORD: f32 = 1.0e6;

pub mod op {
    pub const RECT: u8 = 1;
    pub const RRECT: u8 = 2;
    pub const BORDER: u8 = 3;
    pub const LINEAR: u8 = 4;
    pub const SHADOW: u8 = 5;
    pub const PUSH_CLIP: u8 = 6;
    pub const POP_CLIP: u8 = 7;
    pub const PUSH_OPACITY: u8 = 8;
    pub const POP_OPACITY: u8 = 9;
}

/// A decoded canvas: display-list items in node-local logical pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Canvas {
    pub items: Vec<Item>,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let s = self
            .b
            .get(self.at..self.at + N)
            .ok_or_else(|| Error::invalid(copy::BAD_CANVAS))?;
        self.at += N;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Ok(out)
    }

    fn f(&mut self) -> Result<f32> {
        let v = f32::from_le_bytes(self.take::<4>()?);
        if v.is_finite() && v.abs() <= MAX_COORD {
            Ok(v)
        } else {
            Err(Error::invalid(copy::BAD_CANVAS))
        }
    }

    fn size(&mut self) -> Result<f32> {
        let v = self.f()?;
        if v < 0.0 {
            return Err(Error::invalid(copy::BAD_CANVAS));
        }
        Ok(v)
    }

    fn rect(&mut self) -> Result<Rect> {
        Ok(Rect::new(self.f()?, self.f()?, self.size()?, self.size()?))
    }

    fn radii(&mut self) -> Result<Radii> {
        Ok(Radii([
            self.size()?,
            self.size()?,
            self.size()?,
            self.size()?,
        ]))
    }

    fn color(&mut self) -> Result<Color> {
        Ok(Color::unpack(u32::from_le_bytes(self.take::<4>()?)))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }
}

impl Canvas {
    /// Decodes and validates a whole stream.
    pub fn decode(bytes: &[u8]) -> Result<Canvas> {
        if bytes.get(..4) != Some(&MAGIC[..]) {
            return Err(Error::invalid(copy::BAD_CANVAS));
        }
        let mut r = Reader { b: bytes, at: 4 };
        let mut items = Vec::new();
        // true = clip, false = opacity
        let mut stack: Vec<bool> = Vec::new();
        while r.at < bytes.len() {
            if items.len() >= MAX_OPS {
                return Err(Error::limit(copy::CANVAS_LIMIT));
            }
            let code = r.u8()?;
            let item = match code {
                op::RECT => {
                    let rect = r.rect()?;
                    Item::Rect {
                        rect,
                        radii: Radii::ZERO,
                        fill: Fill::Solid(r.color()?),
                    }
                }
                op::RRECT => {
                    let rect = r.rect()?;
                    let radii = r.radii()?;
                    Item::Rect {
                        rect,
                        radii,
                        fill: Fill::Solid(r.color()?),
                    }
                }
                op::BORDER => {
                    let rect = r.rect()?;
                    let radii = r.radii()?;
                    let width = r.size()?;
                    Item::Border {
                        rect,
                        radii,
                        width,
                        color: r.color()?,
                    }
                }
                op::LINEAR => {
                    let rect = r.rect()?;
                    let radii = r.radii()?;
                    let p0 = (r.f()?, r.f()?);
                    let p1 = (r.f()?, r.f()?);
                    let from = r.color()?;
                    let to = r.color()?;
                    Item::Rect {
                        rect,
                        radii,
                        fill: Fill::Linear { from, to, p0, p1 },
                    }
                }
                op::SHADOW => {
                    let rect = r.rect()?;
                    let radii = r.radii()?;
                    let blur = r.size()?;
                    let spread = r.f()?;
                    let dx = r.f()?;
                    let dy = r.f()?;
                    let s = Shadow::new(blur, spread, dx, dy, r.color()?);
                    Item::Shadow {
                        rect,
                        radii,
                        blur: s.blur,
                        spread: s.spread,
                        dx: s.dx,
                        dy: s.dy,
                        color: s.color,
                    }
                }
                op::PUSH_CLIP => {
                    let rect = r.rect()?;
                    let radius = r.size()?;
                    stack.push(true);
                    Item::PushClip { rect, radius }
                }
                op::POP_CLIP => {
                    if stack.pop() != Some(true) {
                        return Err(Error::invalid(copy::CANVAS_STACK));
                    }
                    Item::PopClip
                }
                op::PUSH_OPACITY => {
                    let alpha = r.u8()?;
                    let overlap = match r.u8()? {
                        0 => false,
                        1 => true,
                        _ => return Err(Error::invalid(copy::BAD_CANVAS)),
                    };
                    stack.push(false);
                    Item::PushOpacity { alpha, overlap }
                }
                op::POP_OPACITY => {
                    if stack.pop() != Some(false) {
                        return Err(Error::invalid(copy::CANVAS_STACK));
                    }
                    Item::PopOpacity
                }
                _ => return Err(Error::invalid(copy::BAD_CANVAS)),
            };
            if stack.len() > MAX_DEPTH {
                return Err(Error::invalid(copy::CANVAS_STACK));
            }
            items.push(item);
        }
        if !stack.is_empty() {
            return Err(Error::invalid(copy::CANVAS_STACK));
        }
        Ok(Canvas { items })
    }

    /// Appends the canvas to `list`, translated to `node` (window logical pixels) and clipped
    /// to it.
    pub fn emit(&self, node: Rect, list: &mut DisplayList) {
        let (ox, oy) = (node.x, node.y);
        let t = |r: &Rect| r.translate(ox, oy);
        list.push_clip(node, 0.0);
        for it in &self.items {
            list.push(match it {
                Item::Rect { rect, radii, fill } => Item::Rect {
                    rect: t(rect),
                    radii: *radii,
                    fill: *fill,
                },
                Item::Border {
                    rect,
                    radii,
                    width,
                    color,
                } => Item::Border {
                    rect: t(rect),
                    radii: *radii,
                    width: *width,
                    color: *color,
                },
                Item::Shadow {
                    rect,
                    radii,
                    blur,
                    spread,
                    dx,
                    dy,
                    color,
                } => Item::Shadow {
                    rect: t(rect),
                    radii: *radii,
                    blur: *blur,
                    spread: *spread,
                    dx: *dx,
                    dy: *dy,
                    color: *color,
                },
                Item::PushClip { rect, radius } => Item::PushClip {
                    rect: t(rect),
                    radius: *radius,
                },
                other => other.clone(),
            });
        }
        list.pop_clip();
    }
}

/// Builds `WKC1` streams (tests, benches and the C example use it; the shard has its own).
#[derive(Default)]
pub struct Encoder {
    pub bytes: Vec<u8>,
}

impl Encoder {
    pub fn new() -> Encoder {
        Encoder {
            bytes: MAGIC.to_vec(),
        }
    }

    fn fs(&mut self, v: &[f32]) {
        for x in v {
            self.bytes.extend_from_slice(&x.to_le_bytes());
        }
    }

    fn c(&mut self, c: Color) {
        self.bytes.extend_from_slice(&c.pack().to_le_bytes());
    }

    pub fn rect(&mut self, r: Rect, c: Color) -> &mut Self {
        self.bytes.push(op::RECT);
        self.fs(&[r.x, r.y, r.w, r.h]);
        self.c(c);
        self
    }

    pub fn rrect(&mut self, r: Rect, radii: [f32; 4], c: Color) -> &mut Self {
        self.bytes.push(op::RRECT);
        self.fs(&[r.x, r.y, r.w, r.h]);
        self.fs(&radii);
        self.c(c);
        self
    }

    pub fn border(&mut self, r: Rect, radii: [f32; 4], width: f32, c: Color) -> &mut Self {
        self.bytes.push(op::BORDER);
        self.fs(&[r.x, r.y, r.w, r.h]);
        self.fs(&radii);
        self.fs(&[width]);
        self.c(c);
        self
    }

    pub fn linear(
        &mut self,
        r: Rect,
        radii: [f32; 4],
        p: [f32; 4],
        c0: Color,
        c1: Color,
    ) -> &mut Self {
        self.bytes.push(op::LINEAR);
        self.fs(&[r.x, r.y, r.w, r.h]);
        self.fs(&radii);
        self.fs(&p);
        self.c(c0);
        self.c(c1);
        self
    }

    pub fn shadow(
        &mut self,
        r: Rect,
        radii: [f32; 4],
        blur_spread_dx_dy: [f32; 4],
        c: Color,
    ) -> &mut Self {
        self.bytes.push(op::SHADOW);
        self.fs(&[r.x, r.y, r.w, r.h]);
        self.fs(&radii);
        self.fs(&blur_spread_dx_dy);
        self.c(c);
        self
    }

    pub fn push_clip(&mut self, r: Rect, radius: f32) -> &mut Self {
        self.bytes.push(op::PUSH_CLIP);
        self.fs(&[r.x, r.y, r.w, r.h, radius]);
        self
    }

    pub fn pop_clip(&mut self) -> &mut Self {
        self.bytes.push(op::POP_CLIP);
        self
    }

    pub fn push_opacity(&mut self, alpha: u8, overlap: bool) -> &mut Self {
        self.bytes
            .extend_from_slice(&[op::PUSH_OPACITY, alpha, u8::from(overlap)]);
        self
    }

    pub fn pop_opacity(&mut self) -> &mut Self {
        self.bytes.push(op::POP_OPACITY);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Status;

    #[test]
    fn encoded_ops_decode_to_display_items() {
        let mut e = Encoder::new();
        e.rect(Rect::new(1.0, 2.0, 3.0, 4.0), Color::rgb(9, 8, 7))
            .push_clip(Rect::new(0.0, 0.0, 10.0, 10.0), 2.0)
            .push_opacity(128, true)
            .rrect(Rect::new(0.0, 0.0, 5.0, 5.0), [1.0; 4], Color::rgb(1, 2, 3))
            .pop_opacity()
            .pop_clip()
            .shadow(
                Rect::new(0.0, 0.0, 5.0, 5.0),
                [1.0; 4],
                [4.0, 0.0, 0.0, 1.0],
                Color::rgba(0, 0, 0, 90),
            )
            .linear(
                Rect::new(0.0, 0.0, 5.0, 5.0),
                [0.0; 4],
                [0.0, 0.0, 0.0, 1.0],
                Color::rgb(1, 1, 1),
                Color::rgb(2, 2, 2),
            )
            .border(
                Rect::new(0.0, 0.0, 5.0, 5.0),
                [0.0; 4],
                1.0,
                Color::rgb(3, 3, 3),
            );
        let c = Canvas::decode(&e.bytes).unwrap_or_default();
        assert_eq!(c.items.len(), 9);
        assert!(matches!(
            c.items[2],
            Item::PushOpacity {
                alpha: 128,
                overlap: true
            }
        ));
        let mut l = DisplayList::new();
        c.emit(Rect::new(100.0, 50.0, 20.0, 20.0), &mut l);
        l.finish();
        assert!(
            l.items
                .iter()
                .any(|i| matches!(i, Item::Rect { rect, .. } if rect.x == 101.0 && rect.y == 52.0))
        );
    }

    #[test]
    fn malformed_streams_are_refused_whole() {
        let bad: [&[u8]; 5] = [b"", b"WKC0", b"WKC1\x01\x00", b"WKC1\x07", b"WKC1\x63"];
        for b in bad {
            let e = Canvas::decode(b).err().map(|e| e.status);
            assert_eq!(e, Some(Status::InvalidArgument), "{b:?}");
        }
        let mut open = Encoder::new();
        open.push_clip(Rect::new(0.0, 0.0, 1.0, 1.0), 0.0);
        assert!(Canvas::decode(&open.bytes).is_err());
        let mut mixed = Encoder::new();
        mixed
            .push_clip(Rect::new(0.0, 0.0, 1.0, 1.0), 0.0)
            .pop_opacity();
        assert!(Canvas::decode(&mixed.bytes).is_err());
        // Raw bytes: `Rect::new` itself would sanitise a NaN.
        let mut nan = Encoder::new();
        nan.bytes.push(op::RECT);
        nan.fs(&[f32::NAN, 0.0, 1.0, 1.0]);
        nan.c(Color::rgb(0, 0, 0));
        assert!(Canvas::decode(&nan.bytes).is_err());
        let mut neg = Encoder::new();
        neg.bytes.push(op::RECT);
        neg.fs(&[0.0, 0.0, -1.0, 1.0]);
        neg.c(Color::rgb(0, 0, 0));
        assert!(Canvas::decode(&neg.bytes).is_err());
    }

    #[test]
    fn limits_on_ops_and_nesting_hold() {
        let mut deep = Encoder::new();
        for _ in 0..=MAX_DEPTH {
            deep.push_opacity(200, false);
        }
        for _ in 0..=MAX_DEPTH {
            deep.pop_opacity();
        }
        assert!(Canvas::decode(&deep.bytes).is_err());
        let mut many = Encoder::new();
        for _ in 0..=MAX_OPS {
            many.pop_clip();
        }
        // The first pop is already unbalanced; build a valid long stream instead.
        let mut long = Encoder::new();
        for _ in 0..=MAX_OPS {
            long.rect(Rect::new(0.0, 0.0, 1.0, 1.0), Color::rgb(0, 0, 0));
        }
        let e = Canvas::decode(&long.bytes).err().map(|e| e.status);
        assert_eq!(e, Some(Status::Limit));
        assert!(Canvas::decode(&many.bytes).is_err());
    }
}
