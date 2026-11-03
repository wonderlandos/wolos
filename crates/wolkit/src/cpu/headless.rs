// SPDX-License-Identifier: BSD-2-Clause
//! The headless entry point: a laid-out scene in, RGBA bytes out. The scene is lowered to a
//! display list with its resolved styles, real text and the token palette (`paint::scene`), and
//! the reference rasteriser draws it onto the theme's surface colour.

use super::raster;
use crate::env::Environment;
use crate::paint::list::DisplayList;
use crate::paint::palette::Palette;
use crate::paint::scene::{Lower, build_with};
use crate::scene::SceneRead;
use crate::style::Styles;
use crate::text::measure::TextCache;
use crate::text::system::TextSystem;

/// One frame to draw: the scene after style and layout, and the device size.
pub struct Frame<'a> {
    pub scene: &'a dyn SceneRead,
    pub styles: &'a Styles,
    pub env: &'a Environment,
    /// Device pixels.
    pub width: u32,
    pub height: u32,
    /// Device pixels per logical pixel.
    pub scale: f32,
    /// The submitted drawings of canvas nodes, drawn inside their rectangles.
    pub canvases: Option<&'a crate::paint::scene::Canvases>,
}

/// Render `f` into `out` (RGBA8, `width * height * 4` bytes, straight alpha). Text is shaped with
/// `text` through the window's paragraph cache, the same entries the layout measured.
pub fn render_frame(f: &Frame<'_>, text: &mut TextSystem, cache: &mut TextCache, out: &mut [u8]) {
    let palette = Palette::for_env(f.env);
    let mut list = DisplayList::new();
    build_with(
        &mut Lower {
            scene: f.scene,
            styles: f.styles,
            palette: &palette,
            text,
            cache,
        },
        f.canvases,
        &mut list,
    );
    let img = raster::render(
        &list,
        &mut text.glyph_source(),
        f.width,
        f.height,
        &raster::RasterOptions {
            scale: f.scale,
            clear: palette.background(),
            tile: raster::TILE,
        },
    );
    let straight = img.to_straight();
    let n = out.len().min(straight.len());
    out[..n].copy_from_slice(&straight[..n]);
}
