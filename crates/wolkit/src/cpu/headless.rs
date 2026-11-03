// SPDX-License-Identifier: BSD-2-Clause
//! The headless entry point: scene in, RGBA bytes out.

use crate::env::Environment;
use crate::scene::{walk, SceneRead};

/// Render `scene` into `out` (RGBA8, `width * height * 4` bytes, straight alpha).
///
/// Interim lowering until the style stage publishes computed styles: the window background and a
/// one pixel outline per node, as W1's placeholder drew. The display-list renderer in
/// [`raster`] is the real engine; this function keeps its signature.
pub fn render(scene: &dyn SceneRead, width: u32, height: u32, scale: f32, env: &Environment, out: &mut [u8]) {
    use crate::paint::geom::{Color, Rect};
    use crate::paint::glyphs::NoGlyphs;
    use crate::paint::list::{DisplayList, Radii};
    let bg = if env.dark { Color::rgb(0x1c, 0x1c, 0x20) } else { Color::rgb(0xf6, 0xf6, 0xf8) };
    let line = if env.dark { Color::rgb(0x3a, 0x3a, 0x3a) } else { Color::rgb(0xd8, 0xd8, 0xd8) };
    let mut list = DisplayList::new();
    walk(scene, 0, &mut |i, _| {
        if i == 0 {
            return;
        }
        let r = scene.rect(i);
        list.border(Rect::new(r.x, r.y, r.w, r.h), Radii::ZERO, 1.0 / scale.max(0.01), line);
    });
    list.finish();
    let img = raster::render(
        &list,
        &mut NoGlyphs,
        width,
        height,
        &raster::RasterOptions { scale, clear: bg, tile: raster::TILE },
    );
    let straight = img.to_straight();
    let n = out.len().min(straight.len());
    out[..n].copy_from_slice(&straight[..n]);
}
