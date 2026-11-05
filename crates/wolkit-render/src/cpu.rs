// SPDX-License-Identifier: BSD-2-Clause
//! The CPU reference rasteriser as a [`Renderer`]: the oracle the GPU targets are compared with.

use crate::{FrameIn, Image, RenderError, Renderer};
use wolkit::cpu::raster::{RasterOptions, TILE, render};
use wolkit::paint::glyphs::{GlyphSource, NoGlyphs};
use wolkit::text::glyphcache::Glyphs;

#[derive(Default)]
pub struct CpuRenderer;

impl Renderer for CpuRenderer {
    fn name(&self) -> String {
        "cpu".into()
    }

    fn render_image(&mut self, frame: FrameIn<'_>) -> Result<Image, RenderError> {
        let mut none = NoGlyphs;
        let mut with;
        let src: &mut dyn GlyphSource = match frame.text {
            Some((db, cache)) => {
                with = Glyphs { db, cache };
                &mut with
            }
            None => &mut none,
        };
        Ok(render(
            frame.list,
            src,
            frame.width,
            frame.height,
            &RasterOptions {
                scale: frame.scale,
                clear: frame.clear,
                tile: TILE,
            },
        ))
    }
}
