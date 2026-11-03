// SPDX-License-Identifier: BSD-2-Clause
//! Shaping one run: a piece of text in one font, one direction and one script.
//!
//! The engine is `harfrust` (a safe-Rust port of HarfBuzz). Glyph positions come back in font
//! units and are scaled to pixels here. Cluster values are UTF-8 byte offsets *relative to the
//! start of the run's text*, so a run can be cached by content and reused anywhere in a document.

use harfrust::{Buffer, Direction, Feature, Script, ShapeOptions, ShaperFont, Tag};

use crate::text::fontdb::{FontDb, FontId};

/// Direction of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Ltr,
    Rtl,
}

/// One OpenType feature setting, e.g. `liga=0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFeature {
    pub tag: [u8; 4],
    pub value: u32,
}

impl FontFeature {
    pub const fn new(tag: &[u8; 4], value: u32) -> FontFeature {
        FontFeature { tag: *tag, value }
    }
}

/// One positioned glyph, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    /// Byte offset of the cluster's first character inside the run text.
    pub cluster: u32,
    pub x_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// The result of shaping a run. Glyphs are in visual order (left to right on screen), so the
/// clusters of an RTL run decrease along the vector.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedRun {
    pub font: FontId,
    pub size: f32,
    pub dir: Dir,
    /// Length in bytes of the text that was shaped.
    pub text_len: u32,
    pub glyphs: Vec<Glyph>,
    pub advance: f32,
    pub ascent: f32,
    /// Positive distance below the baseline.
    pub descent: f32,
    pub line_gap: f32,
}

/// Everything besides the text that decides how a run is shaped.
#[derive(Clone, Copy, Debug)]
pub struct ShapeParams<'a> {
    pub font: FontId,
    pub size: f32,
    pub dir: Dir,
    pub script: Script,
    pub features: &'a [FontFeature],
}

/// Line metrics of a face at a size: (ascent, descent, line gap), all positive, in pixels.
pub fn line_metrics(db: &FontDb, font: FontId, size: f32) -> (f32, f32, f32) {
    let Some(f) = db.face(font).and_then(|f| f.font()) else {
        return (size * 0.8, size * 0.2, 0.0);
    };
    let m = f.metrics();
    let upem = f32::from(m.units_per_em).max(1.0);
    match m.h_line() {
        Some(l) => {
            let k = size / upem;
            (l.ascender.to_f32() * k, -l.descender.to_f32() * k, l.line_gap.to_f32().max(0.0) * k)
        }
        None => (size * 0.8, size * 0.2, 0.0),
    }
}

/// Shapes `text`. A face that cannot be parsed or a shaping error yields a run of `.notdef`
/// glyphs, one per character, so layout always has something to measure and draw.
pub fn shape_run(db: &FontDb, text: &str, p: &ShapeParams<'_>) -> ShapedRun {
    let (ascent, descent, line_gap) = line_metrics(db, p.font, p.size);
    let mut run = ShapedRun {
        font: p.font,
        size: p.size,
        dir: p.dir,
        text_len: text.len() as u32,
        glyphs: Vec::with_capacity(text.chars().count()),
        advance: 0.0,
        ascent,
        descent,
        line_gap,
    };
    let shaped = db.face(p.font).and_then(|f| f.font()).and_then(|font| shape_with(font, text, p, &mut run));
    if shaped.is_none() {
        run.glyphs.clear();
        run.advance = 0.0;
        let adv = p.size * 0.6;
        for (i, _) in text.char_indices() {
            run.glyphs.push(Glyph { id: 0, cluster: i as u32, x_advance: adv, x_offset: 0.0, y_offset: 0.0 });
            run.advance += adv;
        }
        if p.dir == Dir::Rtl {
            run.glyphs.reverse();
        }
    }
    run
}

fn shape_with(font: &skrifa::Font, text: &str, p: &ShapeParams<'_>, run: &mut ShapedRun) -> Option<()> {
    let upem = f32::from(font.units_per_em()).max(1.0);
    let k = p.size / upem;
    let sf = ShaperFont::new(font);
    let mut buf = Buffer::new();
    buf.push_str(text);
    buf.set_direction(match p.dir {
        Dir::Ltr => Direction::LeftToRight,
        Dir::Rtl => Direction::RightToLeft,
    });
    buf.set_script(Some(p.script));
    let mut feats: [Option<Feature>; 8] = [None; 8];
    let mut n = 0;
    for f in p.features.iter().take(feats.len()) {
        feats[n] = Some(Feature::new(Tag::new(&f.tag), f.value, ..));
        n += 1;
    }
    let list: Vec<Feature> = feats[..n].iter().flatten().copied().collect();
    harfrust::shape(&sf, &mut buf, ShapeOptions::new().features(&list)).ok()?;
    let infos = buf.glyph_infos();
    let pos = buf.glyph_positions();
    if infos.len() != pos.len() {
        return None;
    }
    for (i, g) in infos.iter().zip(pos) {
        let adv = g.x_advance as f32 * k;
        run.glyphs.push(Glyph {
            id: i.glyph_id,
            cluster: i.cluster,
            x_advance: adv,
            x_offset: g.x_offset as f32 * k,
            y_offset: g.y_offset as f32 * k,
        });
        run.advance += adv;
    }
    Some(())
}

impl ShapedRun {
    /// Logical cluster boundaries, ascending and de-duplicated, ending with `text_len`.
    pub fn cluster_starts(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.glyphs.iter().map(|g| g.cluster).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The glyph index range (in visual order) whose clusters start inside `[a, b)`.
    ///
    /// Glyphs of one cluster are contiguous and the cluster order is monotonic, so the result is
    /// a single range.
    pub fn glyph_range(&self, a: u32, b: u32) -> std::ops::Range<usize> {
        let inside = |g: &Glyph| g.cluster >= a && g.cluster < b;
        let first = self.glyphs.iter().position(inside);
        let Some(first) = first else { return 0..0 };
        let last = self.glyphs.iter().rposition(inside).unwrap_or(first);
        first..last + 1
    }

    /// A copy holding only the glyphs whose clusters start in `[a, b)`; clusters are rebased to
    /// `a`.
    pub fn slice(&self, a: u32, b: u32) -> ShapedRun {
        let r = self.glyph_range(a, b);
        let glyphs: Vec<Glyph> = self.glyphs[r].iter().map(|g| Glyph { cluster: g.cluster - a, ..*g }).collect();
        let advance = glyphs.iter().map(|g| g.x_advance).sum();
        ShapedRun { glyphs, advance, text_len: b.min(self.text_len).saturating_sub(a), ..self.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_rebases_clusters() {
        let g = |c| Glyph { id: 1, cluster: c, x_advance: 2.0, x_offset: 0.0, y_offset: 0.0 };
        let r = ShapedRun {
            font: FontId(0),
            size: 10.0,
            dir: Dir::Ltr,
            text_len: 4,
            glyphs: vec![g(0), g(1), g(2), g(3)],
            advance: 8.0,
            ascent: 8.0,
            descent: 2.0,
            line_gap: 0.0,
        };
        let s = r.slice(1, 3);
        assert_eq!(s.glyphs.len(), 2);
        assert_eq!(s.glyphs[0].cluster, 0);
        assert_eq!(s.advance, 4.0);
        assert_eq!(s.text_len, 2);
    }
}
