// SPDX-License-Identifier: BSD-2-Clause
//! The GPU path's shadow masks (`gpu::shadow`, a copy of the rasteriser's blur so prototypes can
//! be nine-sliced) must equal what the CPU rasteriser draws, byte for byte: an opaque black
//! shadow on a transparent image leaves exactly the mask in the alpha channel.

use wolkit::cpu::raster::{RasterOptions, render};
use wolkit::gpu::shadow::{ShadowAtlas, mask, sample};
use wolkit::paint::geom::{Color, Rect};
use wolkit::paint::glyphs::NoGlyphs;
use wolkit::paint::list::{DisplayList, Radii, Shadow};

fn cpu_alpha(cast: Rect, radii: [f32; 4], blur: f32, w: u32, h: u32) -> Vec<u8> {
    let mut l = DisplayList::new();
    l.shadow(
        cast,
        Radii(radii),
        Shadow::new(blur, 0.0, 0.0, 0.0, Color::rgb(0, 0, 0)),
    );
    l.finish();
    let img = render(&l, &mut NoGlyphs, w, h, &RasterOptions::default());
    img.data.as_chunks::<4>().0.iter().map(|p| p[3]).collect()
}

const CASES: [(Rect, [f32; 4], f32); 6] = [
    (
        Rect {
            x: 40.0,
            y: 30.0,
            w: 130.0,
            h: 100.0,
        },
        [12.0; 4],
        24.0,
    ),
    (
        Rect {
            x: 40.0,
            y: 30.0,
            w: 130.0,
            h: 100.0,
        },
        [12.0; 4],
        4.0,
    ),
    (
        Rect {
            x: 33.5,
            y: 41.25,
            w: 97.3,
            h: 60.6,
        },
        [3.0, 9.0, 0.0, 14.0],
        9.0,
    ),
    (
        Rect {
            x: 60.0,
            y: 60.0,
            w: 20.0,
            h: 20.0,
        },
        [10.0; 4],
        30.0,
    ),
    (
        Rect {
            x: 50.0,
            y: 50.0,
            w: 80.0,
            h: 40.0,
        },
        [0.0; 4],
        0.0,
    ),
    (
        Rect {
            x: 20.0,
            y: 20.0,
            w: 160.0,
            h: 12.0,
        },
        [6.0; 4],
        2.0,
    ),
];

#[test]
fn shadow_masks_equal_the_cpu_rasteriser() {
    let (w, h) = (220u32, 180u32);
    for (cast, radii, blur) in CASES {
        let want = cpu_alpha(cast, radii, blur, w, h);
        let Some((ext, m)) = mask(&cast, &radii, blur) else {
            panic!("{cast:?}: no mask");
        };
        let mw = usize::try_from(ext.width()).unwrap_or(0);
        for (i, a) in want.iter().enumerate() {
            let (x, y) = (i % 220, i / 220);
            let (mx, my) = (
                i64::try_from(x).unwrap_or(0) - i64::from(ext.x0),
                i64::try_from(y).unwrap_or(0) - i64::from(ext.y0),
            );
            let inside =
                mx >= 0 && my >= 0 && mx < i64::from(ext.width()) && my < i64::from(ext.height());
            let got = if inside {
                m[usize::try_from(my).unwrap_or(0) * mw + usize::try_from(mx).unwrap_or(0)]
            } else {
                0
            };
            assert_eq!(
                got, *a,
                "{cast:?} r={radii:?} blur={blur}: pixel ({x}, {y})"
            );
        }
    }
}

#[test]
fn stretched_prototypes_equal_the_cpu_rasteriser() {
    let (w, h) = (220u32, 180u32);
    for (cast, radii, blur) in CASES {
        let want = cpu_alpha(cast, radii, blur, w, h);
        let mut atlas = ShadowAtlas::new(1024);
        let Some((p, ext)) = atlas.get(&cast, &radii, blur) else {
            panic!("{cast:?}: no prototype");
        };
        for (i, a) in want.iter().enumerate() {
            let (x, y) = (i % 220, i / 220);
            let (mx, my) = (
                i64::try_from(x).unwrap_or(0) - i64::from(ext.x0),
                i64::try_from(y).unwrap_or(0) - i64::from(ext.y0),
            );
            let inside =
                mx >= 0 && my >= 0 && mx < i64::from(ext.width()) && my < i64::from(ext.height());
            let got = if inside {
                sample(
                    &atlas,
                    &p,
                    u32::try_from(mx).unwrap_or(0),
                    u32::try_from(my).unwrap_or(0),
                )
            } else {
                0
            };
            assert_eq!(
                got, *a,
                "{cast:?} r={radii:?} blur={blur}: pixel ({x}, {y})"
            );
        }
    }
}
