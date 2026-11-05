// SPDX-License-Identifier: BSD-2-Clause
//! Geometry of a multi-line text field: where the caret is on the laid-out lines, where a click
//! lands, what Up, Down, Home, End and the page keys do, and how far the text scrolls so the caret
//! stays in view. Pure functions over a [`TextLayout`] and the string it was laid out from (the
//! editor's display text); the editing itself is `text::edit`, the keys and the pointer are
//! `input::edit`, and `paint::scene` draws the result.
//!
//! Movement here is by position on screen for Up and Down and for the line ends, and logical for
//! Left and Right (`text::edit`): bidirectional text moves in logical order and visual caret
//! movement is deferred (see `docs/modules/wolkit.md`, "Limits").
//!
//! Soft wraps: the caret stored at the offset where a wrapped line ends is drawn at the start of
//! the next line (that is where the text continues), so a click past the end of a wrapped line,
//! and End, stop before the white space that the wrap swallowed.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::text::layout::TextLayout;

/// Where the caret is drawn: the line, the x position and the line's vertical extent, all in the
/// layout's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretPos {
    pub line: usize,
    pub x: f32,
    pub top: f32,
    pub height: f32,
}

/// The caret placed before byte `idx` of `text`.
pub fn caret_pos(layout: &TextLayout, text: &str, idx: usize) -> CaretPos {
    let (line, x) = layout.caret(text, idx);
    let l = layout.lines.get(line).or_else(|| layout.lines.last());
    CaretPos {
        line,
        x,
        top: l.map_or(0.0, |l| l.top),
        height: l.map_or(0.0, |l| l.height),
    }
}

/// The line a layout y position falls on (the last line below the text, the first above it).
pub fn line_at(layout: &TextLayout, y: f32) -> usize {
    layout
        .lines
        .iter()
        .position(|l| y < l.top + l.height)
        .unwrap_or(layout.lines.len().saturating_sub(1))
}

/// Whether `line` ends because the text wrapped there, not because a paragraph ended: the next
/// line then continues at the very byte this one ends at.
fn soft_wrapped(layout: &TextLayout, line: usize) -> bool {
    match (layout.lines.get(line), layout.lines.get(line + 1)) {
        (Some(a), Some(b)) => a.range.end == b.range.start && a.range.end > a.range.start,
        _ => false,
    }
}

/// The end of `line` as a caret position: before the white space a soft wrap swallowed.
fn visual_end(layout: &TextLayout, text: &str, line: usize) -> usize {
    let r = layout.lines[line].range.clone();
    if !soft_wrapped(layout, line) {
        return r.end;
    }
    match text[r.clone()].grapheme_indices(true).next_back() {
        Some((i, g)) if g.chars().all(char::is_whitespace) => r.start + i,
        _ => r.end,
    }
}

/// The byte offset nearest to a point of the layout. Unlike `TextLayout::hit_test`, a click past
/// the end of a wrapped line lands before the space that wrapped, so the caret stays on that line.
pub fn hit(layout: &TextLayout, text: &str, x: f32, y: f32) -> usize {
    let at = layout.hit_test(text, x, y);
    let li = line_at(layout, y);
    if li < layout.lines.len() && at == layout.lines[li].range.end && soft_wrapped(layout, li) {
        return visual_end(layout, text, li);
    }
    at
}

/// The start and end of the line the caret at `idx` is drawn on (Home and End).
pub fn line_edges(layout: &TextLayout, text: &str, idx: usize) -> (usize, usize) {
    let li = caret_pos(layout, text, idx).line;
    match layout.lines.get(li) {
        Some(l) => (l.range.start, visual_end(layout, text, li)),
        None => (0, text.len()),
    }
}

/// Moves the caret `lines` lines down (negative: up), keeping the x position it started from.
/// `goal_x` is that x position when a run of vertical moves is under way (`None` on the first
/// move: it is read from the caret). Returns the new offset and the goal to hand back next time.
/// Moving up from the first line goes to the start of the text, down from the last line to the end.
pub fn move_vertical(
    layout: &TextLayout,
    text: &str,
    idx: usize,
    goal_x: Option<f32>,
    lines: isize,
) -> (usize, f32) {
    let from = caret_pos(layout, text, idx);
    let goal = goal_x.unwrap_or(from.x);
    let last = layout.lines.len().saturating_sub(1);
    let target = from.line.cast_signed().saturating_add(lines);
    if target < 0 {
        return (0, goal);
    }
    let target = target.cast_unsigned();
    if target > last {
        return (text.len(), goal);
    }
    let l = &layout.lines[target];
    (hit(layout, text, goal, l.top + l.height / 2.0), goal)
}

/// How many lines fit in a view `view_h` tall, at least one (the page keys move by this less one).
pub fn lines_per_page(layout: &TextLayout, view_h: f32) -> usize {
    let h = layout.lines.first().map_or(0.0, |l| l.height);
    if h <= 0.0 {
        return 1;
    }
    ((view_h / h).floor().max(1.0)) as usize
}

/// The scroll position that shows the caret: unchanged when it is already inside the view.
/// `view` is the visible size. A multi-line field never scrolls sideways (`wrap` true), a
/// single-line one only sideways.
pub fn scroll_to_caret(
    scroll: (f32, f32),
    view: (f32, f32),
    c: &CaretPos,
    wrap: bool,
) -> (f32, f32) {
    let fit = |s: f32, lo: f32, hi: f32, len: f32| -> f32 {
        if lo < s {
            lo
        } else if hi > s + len {
            hi - len
        } else {
            s
        }
    };
    if wrap {
        (0.0, fit(scroll.1, c.top, c.top + c.height, view.1).max(0.0))
    } else {
        // One pixel for the caret itself.
        (fit(scroll.0, c.x, c.x + 1.0, view.0).max(0.0), 0.0)
    }
}

/// `scroll` limited to the text: it never scrolls past the end of the content.
pub fn clamp_scroll(scroll: (f32, f32), content: (f32, f32), view: (f32, f32)) -> (f32, f32) {
    (
        scroll.0.clamp(0.0, (content.0 - view.0).max(0.0)),
        scroll.1.clamp(0.0, (content.1 - view.1).max(0.0)),
    )
}

/// The word (or the run of white space or punctuation) around byte `idx`, for a double click.
pub fn word_at(text: &str, idx: usize) -> Range<usize> {
    let idx = idx.min(text.len());
    for (i, w) in text.split_word_bound_indices() {
        if idx < i + w.len() || i + w.len() == text.len() {
            return i..i + w.len();
        }
    }
    idx..idx
}

/// The paragraph (between line breaks) around byte `idx`, for a triple click; the break at its end
/// is not part of it.
pub fn paragraph_at(text: &str, idx: usize) -> Range<usize> {
    let idx = idx.min(text.len());
    let is_break = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    let start = text[..idx].rfind(is_break).map_or(0, |i| {
        i + text[i..].chars().next().map_or(1, char::len_utf8)
    });
    let end = text[idx..].find(is_break).map_or(text.len(), |i| idx + i);
    start..end
}
