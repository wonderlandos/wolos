// SPDX-License-Identifier: BSD-2-Clause
//! Turning a laid-out paragraph into display-list glyph runs.

use crate::paint::geom::{Color, Rect};
use crate::paint::list::{DisplayList, PosGlyph};
use crate::text::layout::TextLayout;

/// Appends the glyphs of `layout` with its origin at `(ox, oy)`.
pub fn push_layout(list: &mut DisplayList, layout: &TextLayout, ox: f32, oy: f32, color: Color) {
    for line in &layout.lines {
        for frag in &line.frags {
            let run = &frag.run;
            let mut pen = ox + frag.x;
            let base = oy + line.baseline;
            let glyphs = run.glyphs.iter().map(|g| {
                let p = PosGlyph { id: g.id, x: pen + g.x_offset, y: base - g.y_offset };
                pen += g.x_advance;
                p
            });
            list.glyph_run(run.font, run.size, color, glyphs);
        }
    }
}

/// Selection highlight rectangles translated to the layout origin.
pub fn push_selection(list: &mut DisplayList, rects: &[Rect], ox: f32, oy: f32, color: Color) {
    for r in rects {
        list.solid(r.translate(ox, oy), color);
    }
}
