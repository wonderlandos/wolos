// SPDX-License-Identifier: BSD-2-Clause
//! Where a picture lands in its node (`image-fit`) and how a texel is sampled. The CPU
//! rasteriser calls [`bilinear`]; the GPU shader (`shaders/ui.frag`, `K_IMAGE`) does the same
//! integer arithmetic on the same level, so the two agree within the golden tolerance.

use super::Level;
use crate::num::{floor_i32, trunc_u32};
use crate::paint::geom::{Rect, div255};
use crate::scene::ids::to_u8;

/// `image-fit`, in the order of the property's enum (`scene::prop`, `FIT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// Whole picture inside the box, aspect kept (the default).
    Contain,
    /// The box filled, aspect kept, the overflow cut.
    Cover,
    /// Stretched to the box.
    Fill,
    /// One picture pixel per logical pixel, centred.
    None,
}

impl Fit {
    pub fn from_index(i: u32) -> Fit {
        match i {
            1 => Fit::Cover,
            2 => Fit::Fill,
            3 => Fit::None,
            _ => Fit::Contain,
        }
    }
}

/// The rectangle the whole picture (`iw x ih`) maps onto inside `node`, centred. Its overlap
/// with `node` is what is drawn.
pub fn dest(fit: Fit, node: Rect, iw: u32, ih: u32) -> Rect {
    let (iw, ih) = (iw.max(1) as f32, ih.max(1) as f32);
    let (w, h) = match fit {
        Fit::Fill => return node,
        Fit::None => (iw, ih),
        Fit::Contain | Fit::Cover => {
            let (kx, ky) = (node.w / iw, node.h / ih);
            let k = if fit == Fit::Cover {
                kx.max(ky)
            } else {
                kx.min(ky)
            };
            (iw * k, ih * k)
        }
    };
    Rect::new(
        node.x + (node.w - w) / 2.0,
        node.y + (node.h - h) / 2.0,
        w,
        h,
    )
}

#[inline]
fn weight(f: f32) -> u32 {
    trunc_u32(f.clamp(0.0, 1.0) * 255.0 + 0.5)
}

#[inline]
fn mix4(a: [u8; 4], b: [u8; 4], t: u32) -> [u8; 4] {
    let m = |x: u8, y: u8| to_u8(div255(u32::from(x) * (255 - t) + u32::from(y) * t));
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2]), m(a[3], b[3])]
}

/// The premultiplied colour at texel coordinates `(u, v)` (texel centres at whole numbers):
/// the four neighbours mixed with weights quantised to 0..255, edges clamped.
#[inline]
pub fn bilinear(l: &Level, u: f32, v: f32) -> [u8; 4] {
    let (fu, fv) = (u.floor(), v.floor());
    let (wx, wy) = (weight(u - fu), weight(v - fv));
    let (x0, y0) = (floor_i32(fu), floor_i32(fv));
    let x1 = x0.saturating_add(1);
    let y1 = y0.saturating_add(1);
    let top = mix4(l.texel(x0, y0), l.texel(x1, y0), wx);
    let bot = mix4(l.texel(x0, y1), l.texel(x1, y1), wx);
    mix4(top, bot, wy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_modes_place_the_picture_like_css() {
        let node = Rect::new(10.0, 20.0, 200.0, 100.0);
        assert_eq!(dest(Fit::Fill, node, 50, 50), node);
        assert_eq!(
            dest(Fit::Contain, node, 50, 50),
            Rect::new(60.0, 20.0, 100.0, 100.0)
        );
        assert_eq!(
            dest(Fit::Cover, node, 50, 50),
            Rect::new(10.0, -30.0, 200.0, 200.0)
        );
        assert_eq!(
            dest(Fit::None, node, 40, 20),
            Rect::new(90.0, 60.0, 40.0, 20.0)
        );
        assert_eq!(Fit::from_index(1), Fit::Cover);
        assert_eq!(Fit::from_index(99), Fit::Contain);
    }

    #[test]
    fn bilinear_hits_texels_exactly_at_their_centres() {
        let l = Level {
            w: 2,
            h: 1,
            px: vec![0, 0, 0, 255, 255, 255, 255, 255],
        };
        assert_eq!(bilinear(&l, 0.0, 0.0), [0, 0, 0, 255]);
        assert_eq!(bilinear(&l, 1.0, 0.0), [255, 255, 255, 255]);
        assert_eq!(bilinear(&l, 0.5, 0.0), [128, 128, 128, 255]);
        // Clamped at the edges.
        assert_eq!(bilinear(&l, -3.0, 9.0), [0, 0, 0, 255]);
        assert_eq!(bilinear(&l, f32::NAN, 0.0)[3], 255);
    }
}
