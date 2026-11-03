// SPDX-License-Identifier: BSD-2-Clause
//! Paragraph layout: itemise (bidi level, script, font), shape through the cache, break lines,
//! reorder visually, align, and answer caret / hit-test / selection queries.
//!
//! * Bidi: UAX #9 through `unicode-bidi`, per paragraph, base direction automatic or forced.
//! * Line breaking: UAX #14 opportunities from `unicode-linebreak`; a word longer than the line
//!   breaks between clusters as a last resort.
//! * Clusters: breaks and carets only fall on shaped-cluster boundaries, so a ligature or a
//!   combining sequence is never split.
//! * Positions are logical `f32` pixels; the baseline of each line is rounded to a whole pixel so
//!   text stays crisp.

use std::ops::Range;
use std::sync::Arc;

use harfrust::Script;
use unicode_bidi::{Level, ParagraphBidiInfo};
use unicode_linebreak::linebreaks;
use unicode_segmentation::UnicodeSegmentation;

use crate::paint::geom::Rect;
use crate::text::cache::ShapeCache;
use crate::text::fontdb::{FontDb, FontId};
use crate::text::shape::{line_metrics, Dir, FontFeature, ShapeParams, ShapedRun};

/// Base direction of a paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BaseDir {
    /// From the first strong character (UAX #9 P2/P3).
    #[default]
    Auto,
    Ltr,
    Rtl,
}

/// Horizontal alignment relative to the paragraph direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    /// Left for LTR, right for RTL.
    #[default]
    Start,
    Center,
    End,
}

/// The font and size of a piece of text.
#[derive(Clone, Debug)]
pub struct TextStyle {
    /// The primary face; other faces are chosen per character from the fallback chain.
    pub font: FontId,
    pub size: f32,
    /// Line pitch in pixels; `0.0` uses the primary face's own metrics.
    pub line_height: f32,
    pub features: Vec<FontFeature>,
}

impl TextStyle {
    pub fn new(font: FontId, size: f32) -> TextStyle {
        TextStyle { font, size, line_height: 0.0, features: Vec::new() }
    }
}

/// How a paragraph is laid out.
#[derive(Clone, Copy, Debug)]
pub struct LayoutOptions {
    /// Wrap width in pixels; `f32::INFINITY` disables wrapping.
    pub max_width: f32,
    pub align: Align,
    pub base: BaseDir,
    /// Stop after this many lines (0 = unlimited).
    pub max_lines: usize,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        LayoutOptions { max_width: f32::INFINITY, align: Align::Start, base: BaseDir::Auto, max_lines: 0 }
    }
}

/// A piece of a line: a slice of one shaped run, placed at `x`.
#[derive(Clone, Debug)]
pub struct Frag {
    pub run: Arc<ShapedRun>,
    /// Byte range of the text this fragment covers (absolute offsets in the laid-out string).
    pub range: Range<usize>,
    /// Left edge from the layout origin.
    pub x: f32,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub range: Range<usize>,
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
    /// Left edge of the content (after alignment).
    pub x: f32,
    /// Width of the content, trailing white space excluded.
    pub width: f32,
    pub rtl: bool,
    pub frags: Vec<Frag>,
}

/// One caret stop: a cluster's horizontal extent and the bytes it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterBox {
    pub bytes: (usize, usize),
    pub x0: f32,
    pub x1: f32,
    pub rtl: bool,
    pub line: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TextLayout {
    pub text_len: usize,
    pub lines: Vec<Line>,
    pub width: f32,
    pub height: f32,
}

struct Item {
    range: Range<usize>,
    run: Arc<ShapedRun>,
}

struct Atom {
    start: usize,
    end: usize,
    width: f32,
    space: bool,
}

/// Lays `text` out. Paragraphs are separated by `\n`, `\r\n` or U+2029.
pub fn layout_text(
    db: &FontDb,
    cache: &mut ShapeCache,
    text: &str,
    style: &TextStyle,
    opts: &LayoutOptions,
) -> TextLayout {
    let mut out = TextLayout { text_len: text.len(), ..TextLayout::default() };
    let (asc, desc, gap) = line_metrics(db, style.font, style.size);
    let pitch = if style.line_height > 0.0 { style.line_height } else { asc + desc + gap };
    let mut y = 0.0f32;
    let mut start = 0usize;
    let mut lines_done = 0usize;
    for (para, next) in paragraphs(text) {
        let body = &text[start..para];
        let mut lines = layout_paragraph(db, cache, text, start..para, body, style, opts);
        if lines.is_empty() {
            lines.push(PendingLine::empty(start));
        }
        for pl in lines {
            if opts.max_lines != 0 && lines_done >= opts.max_lines {
                break;
            }
            let baseline = (y + (pitch - (asc + desc)) * 0.5 + asc).round();
            out.lines.push(pl.finish(y, pitch, baseline));
            y += pitch;
            lines_done += 1;
        }
        start = next;
    }
    if out.lines.is_empty() {
        out.lines.push(PendingLine::empty(0).finish(0.0, pitch, (((pitch - (asc + desc)) * 0.5) + asc).round()));
        y = pitch;
    }
    // Width and alignment.
    out.width = out.lines.iter().map(|l| l.width).fold(0.0, f32::max);
    let avail = if opts.max_width.is_finite() { opts.max_width.max(out.width) } else { out.width };
    for l in &mut out.lines {
        let slack = (avail - l.width).max(0.0);
        let off = match (opts.align, l.rtl) {
            (Align::Start, false) | (Align::End, true) => 0.0,
            (Align::Start, true) | (Align::End, false) => slack,
            (Align::Center, _) => (slack * 0.5).floor(),
        };
        l.x = off;
        for f in &mut l.frags {
            f.x += off;
        }
    }
    out.height = y;
    out
}

struct PendingLine {
    range: Range<usize>,
    frags: Vec<Frag>,
    width: f32,
    rtl: bool,
}

impl PendingLine {
    fn empty(at: usize) -> PendingLine {
        PendingLine { range: at..at, frags: Vec::new(), width: 0.0, rtl: false }
    }

    fn finish(self, top: f32, height: f32, baseline: f32) -> Line {
        Line { range: self.range, top, height, baseline, x: 0.0, width: self.width, rtl: self.rtl, frags: self.frags }
    }
}

/// Paragraph spans: `(end_of_text, start_of_next)` pairs. The last paragraph always exists, so
/// text ending in a newline has a final empty paragraph.
fn paragraphs(text: &str) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => {
                v.push((i, i + 1));
                i += 1;
            }
            b'\r' => {
                let n = if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                v.push((i, i + n));
                i += n;
            }
            0xE2 if text.is_char_boundary(i) && text[i..].starts_with('\u{2029}') => {
                v.push((i, i + 3));
                i += 3;
            }
            _ => i += 1,
        }
    }
    v.push((text.len(), text.len()));
    v
}

fn script_of(c: char) -> Script {
    harfrust::unicode::script(c as u32)
}

fn layout_paragraph(
    db: &FontDb,
    cache: &mut ShapeCache,
    _full: &str,
    span: Range<usize>,
    body: &str,
    style: &TextStyle,
    opts: &LayoutOptions,
) -> Vec<PendingLine> {
    if body.is_empty() {
        return Vec::new();
    }
    let base_off = span.start;
    let level = match opts.base {
        BaseDir::Auto => None,
        BaseDir::Ltr => Some(Level::ltr()),
        BaseDir::Rtl => Some(Level::rtl()),
    };
    let bidi = ParagraphBidiInfo::new(body, level);
    let para_rtl = bidi.paragraph_level.is_rtl();

    // --- itemise ---
    let mut items: Vec<Item> = Vec::new();
    let mut run_start = 0usize;
    let mut cur_font = style.font;
    let mut cur_level = bidi.levels.first().copied().unwrap_or(bidi.paragraph_level);
    // The script of leading common characters comes from the first real script in the paragraph.
    let first_real = body
        .chars()
        .map(script_of)
        .find(|s| *s != Script::COMMON && *s != Script::INHERITED)
        .unwrap_or(Script::LATIN);
    let mut cur_script = first_real;
    let mut first = true;
    let flush = |items: &mut Vec<Item>, cache: &mut ShapeCache, from: usize, to: usize, font: FontId, script: Script, lvl: Level| {
        if to <= from {
            return;
        }
        let dir = if lvl.is_rtl() { Dir::Rtl } else { Dir::Ltr };
        let p = ShapeParams { font, size: style.size, dir, script, features: &style.features };
        let run = cache.get(db, &body[from..to], &p);
        items.push(Item { range: base_off + from..base_off + to, run });
    };
    for (i, c) in body.char_indices() {
        let lvl = bidi.levels[i];
        let s = script_of(c);
        let real = s != Script::COMMON && s != Script::INHERITED;
        let new_script = if real { s } else { cur_script };
        let new_font = if !real && db.face(cur_font).is_some_and(|f| f.has_char(c)) {
            cur_font
        } else if c.is_control() || c.is_whitespace() || s == Script::INHERITED {
            cur_font
        } else {
            db.resolve(c, new_script, style.font)
        };
        if !first && (lvl != cur_level || new_script != cur_script || new_font != cur_font) {
            flush(&mut items, cache, run_start, i, cur_font, cur_script, cur_level);
            run_start = i;
        }
        first = false;
        cur_level = lvl;
        cur_script = new_script;
        cur_font = new_font;
    }
    flush(&mut items, cache, run_start, body.len(), cur_font, cur_script, cur_level);

    // --- atoms (logical order) ---
    let mut atoms: Vec<Atom> = Vec::new();
    for it in &items {
        let starts = it.run.cluster_starts();
        for (k, &s) in starts.iter().enumerate() {
            let e = starts.get(k + 1).copied().unwrap_or(it.run.text_len);
            let width: f32 = it.run.glyphs.iter().filter(|g| g.cluster == s).map(|g| g.x_advance).sum();
            let (a, b) = (it.range.start + s as usize, it.range.start + e as usize);
            let space = body[a - base_off..b - base_off].chars().all(|c| c.is_whitespace());
            atoms.push(Atom { start: a, end: b, width, space });
        }
    }
    atoms.sort_by_key(|a| a.start);

    // --- break into lines ---
    let allowed: Vec<usize> = linebreaks(body).map(|(i, _)| base_off + i).collect();
    let mut ranges: Vec<Range<usize>> = Vec::new(); // atom index ranges
    let wrap = opts.max_width.is_finite();
    let mut line_start = 0usize;
    let mut w = 0.0f32;
    let mut trailing = 0.0f32;
    let mut last_break: Option<(usize, f32)> = None; // (atom count included, content width at break)
    let mut i = 0usize;
    while i < atoms.len() {
        let a = &atoms[i];
        w += a.width;
        if a.space {
            trailing += a.width;
        } else {
            trailing = 0.0;
        }
        let content = w - trailing;
        if wrap && content > opts.max_width && i > line_start {
            // Over the limit: break at the last opportunity, else right before this atom.
            let (brk, _) = last_break.unwrap_or((i, 0.0));
            let brk = if brk <= line_start { i } else { brk };
            ranges.push(line_start..brk);
            line_start = brk;
            w = 0.0;
            trailing = 0.0;
            last_break = None;
            i = brk;
            continue;
        }
        if allowed.binary_search(&a.end).is_ok() && a.end < base_off + body.len() {
            last_break = Some((i + 1, content));
        }
        i += 1;
    }
    if line_start < atoms.len() || ranges.is_empty() {
        ranges.push(line_start..atoms.len());
    }

    // --- build each line ---
    let mut out = Vec::new();
    for r in ranges {
        let (lo, hi) = if r.is_empty() {
            (base_off, base_off)
        } else {
            (atoms[r.start].start, atoms[r.end - 1].end)
        };
        let (levels, vruns) = bidi.visual_runs(lo - base_off..hi - base_off);
        let mut frags = Vec::new();
        let mut x = 0.0f32;
        let mut content_end = 0.0f32;
        for vr in &vruns {
            let rtl = levels[vr.start].is_rtl();
            let mut pieces: Vec<(usize, Range<usize>)> = Vec::new();
            for (ii, it) in items.iter().enumerate() {
                let a = it.range.start.max(base_off + vr.start);
                let b = it.range.end.min(base_off + vr.end);
                if a < b {
                    pieces.push((ii, a..b));
                }
            }
            if rtl {
                pieces.reverse();
            }
            for (ii, rg) in pieces {
                let it = &items[ii];
                let run = if rg == it.range {
                    Arc::clone(&it.run)
                } else {
                    Arc::new(it.run.slice((rg.start - it.range.start) as u32, (rg.end - it.range.start) as u32))
                };
                let adv = run.advance;
                frags.push(Frag { run, range: rg, x });
                x += adv;
            }
        }
        // Content width excludes the trailing white space of the line.
        let mut trail = 0.0f32;
        for a in atoms[r.clone()].iter().rev() {
            if a.space {
                trail += a.width;
            } else {
                break;
            }
        }
        content_end += x - trail;
        out.push(PendingLine { range: lo..hi, frags, width: content_end.max(0.0), rtl: para_rtl });
    }
    out
}

impl TextLayout {
    /// Every caret stop of every line, in visual order per line.
    pub fn cluster_boxes(&self) -> Vec<ClusterBox> {
        let mut v = Vec::new();
        for (li, line) in self.lines.iter().enumerate() {
            for f in &line.frags {
                let run = &f.run;
                let starts = run.cluster_starts();
                let mut x = f.x;
                let mut i = 0;
                while i < run.glyphs.len() {
                    let c = run.glyphs[i].cluster;
                    let mut adv = 0.0;
                    let mut j = i;
                    while j < run.glyphs.len() && run.glyphs[j].cluster == c {
                        adv += run.glyphs[j].x_advance;
                        j += 1;
                    }
                    let k = starts.binary_search(&c).unwrap_or(0);
                    let e = starts.get(k + 1).copied().unwrap_or(run.text_len);
                    v.push(ClusterBox {
                        bytes: (f.range.start + c as usize, f.range.start + e as usize),
                        x0: x,
                        x1: x + adv,
                        rtl: run.dir == Dir::Rtl,
                        line: li,
                    });
                    x += adv;
                    i = j;
                }
            }
        }
        v
    }

    /// The x position and line of the caret placed before byte `idx` (clamped to the text).
    /// `text` is the string that was laid out; it gives grapheme boundaries inside a cluster.
    pub fn caret(&self, text: &str, idx: usize) -> (usize, f32) {
        let idx = idx.min(text.len());
        let boxes = self.cluster_boxes();
        for b in &boxes {
            if idx >= b.bytes.0 && idx < b.bytes.1 {
                let total = text[b.bytes.0..b.bytes.1].graphemes(true).count().max(1);
                let before = text[b.bytes.0..idx].graphemes(true).count();
                let w = b.x1 - b.x0;
                let t = before as f32 / total as f32;
                let x = if b.rtl { b.x1 - w * t } else { b.x0 + w * t };
                return (b.line, x);
            }
        }
        // End of a line or of the text: the trailing edge of the last cluster ending at idx.
        if let Some(b) = boxes.iter().rev().find(|b| b.bytes.1 <= idx) {
            let li = self.line_of(idx).unwrap_or(b.line);
            let line = &self.lines[li];
            if b.line == li {
                return (li, if b.rtl { b.x0 } else { b.x1 });
            }
            return (li, line.x);
        }
        let l = self.lines.first();
        (0, if l.is_some_and(|l| l.rtl) { l.map_or(0.0, |l| l.x + l.width) } else { l.map_or(0.0, |l| l.x) })
    }

    fn line_of(&self, idx: usize) -> Option<usize> {
        self.lines.iter().position(|l| idx >= l.range.start && idx <= l.range.end)
    }

    /// The byte offset nearest to a point (the grapheme boundary closest to `x` on the line at `y`).
    pub fn hit_test(&self, text: &str, x: f32, y: f32) -> usize {
        let li = self
            .lines
            .iter()
            .position(|l| y < l.top + l.height)
            .unwrap_or(self.lines.len().saturating_sub(1));
        let Some(line) = self.lines.get(li) else { return 0 };
        let boxes: Vec<ClusterBox> = self.cluster_boxes().into_iter().filter(|b| b.line == li).collect();
        let mut best = (line.range.start, f32::INFINITY);
        for b in &boxes {
            let g: Vec<usize> = text[b.bytes.0..b.bytes.1].grapheme_indices(true).map(|(i, _)| b.bytes.0 + i).collect();
            let total = g.len().max(1);
            let w = b.x1 - b.x0;
            for (k, &bi) in g.iter().enumerate() {
                let t = k as f32 / total as f32;
                let px = if b.rtl { b.x1 - w * t } else { b.x0 + w * t };
                let d = (px - x).abs();
                if d < best.1 {
                    best = (bi, d);
                }
            }
            // The trailing edge belongs to the next cluster, except at the end of the line.
            if b.bytes.1 == line.range.end {
                let px = if b.rtl { b.x0 } else { b.x1 };
                let d = (px - x).abs();
                if d < best.1 {
                    best = (b.bytes.1, d);
                }
            }
        }
        best.0
    }

    /// Highlight rectangles for the byte range `sel`, one per contiguous visual stretch.
    pub fn selection_rects(&self, sel: Range<usize>) -> Vec<Rect> {
        let mut out: Vec<Rect> = Vec::new();
        if sel.start >= sel.end {
            return out;
        }
        for b in self.cluster_boxes() {
            let lo = b.bytes.0.max(sel.start);
            let hi = b.bytes.1.min(sel.end);
            if lo >= hi {
                continue;
            }
            let line = &self.lines[b.line];
            let r = Rect::new(b.x0, line.top, b.x1 - b.x0, line.height);
            if let Some(last) = out.last_mut() {
                if (last.y - r.y).abs() < 0.01 && (last.right() - r.x).abs() < 0.01 {
                    *last = last.union(&r);
                    continue;
                }
            }
            out.push(r);
        }
        out
    }
}
