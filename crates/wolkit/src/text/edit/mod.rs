// SPDX-License-Identifier: BSD-2-Clause
//! The text-editing core of `text_input`: a rope, a selection, undo/redo and the IME state
//! machine. No Wayland and no layout: callers feed it key actions and text, and read back the
//! text, the selection and the preedit. Visual (bidi-aware) caret movement is the layout's job;
//! the movements here are logical (by grapheme, word, line edge).
//!
//! Invariants, checked by `tests/text_editor.rs`:
//! * every offset the editor stores is on a character boundary, and selection ends are moved to
//!   grapheme boundaries by the movement and delete commands;
//! * `undo` followed by `redo` is the identity, and undoing everything restores the starting text
//!   and selection exactly;
//! * the preedit never changes `text()` and never enters the undo history;
//! * the text never exceeds `limits.max_bytes`.

pub mod ime;
pub mod rope;

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::text::edit::ime::{Ime, Pending, Preedit, surrounding_window};
use crate::text::edit::rope::Rope;

/// Largest surrounding text sent to an input method (the protocol's limit).
pub const SURROUNDING_LIMIT: usize = 4000;
/// Bytes of context around the caret examined for grapheme boundaries.
const GRAPHEME_WINDOW: usize = 256;
/// Bytes of context examined for word boundaries.
const WORD_WINDOW: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    /// The fixed end.
    pub anchor: usize,
    /// The moving end, where the caret is drawn.
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Selection {
        Selection {
            anchor: at,
            head: at,
        }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_undo: usize,
    /// Total bytes the undo and redo stacks may hold.
    pub max_history_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_bytes: 64 * 1024,
            max_undo: 1000,
            max_history_bytes: 1 << 20,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Typing,
    Delete,
    Other,
}

#[derive(Clone, Debug)]
struct Edit {
    pos: usize,
    removed: String,
    inserted: String,
    before: Selection,
    after: Selection,
    group: Group,
    /// A closed edit is never merged into.
    closed: bool,
}

impl Edit {
    fn bytes(&self) -> usize {
        self.removed.len() + self.inserted.len() + 64
    }
}

pub struct Editor {
    rope: Rope,
    sel: Selection,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    history_bytes: usize,
    ime: Ime,
    pub limits: Limits,
    single_line: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new(true)
    }
}

impl Editor {
    /// A new empty editor; `single_line` turns line breaks in inserted text into spaces.
    pub fn new(single_line: bool) -> Editor {
        Editor {
            rope: Rope::new(),
            sel: Selection::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            history_bytes: 0,
            ime: Ime::new(),
            limits: Limits::default(),
            single_line,
        }
    }

    pub fn text(&self) -> String {
        self.rope.to_text()
    }

    pub fn len(&self) -> usize {
        self.rope.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.is_empty()
    }

    pub fn selection(&self) -> Selection {
        self.sel
    }

    pub fn caret(&self) -> usize {
        self.sel.head
    }

    pub fn selected_text(&self) -> String {
        self.rope.slice(self.sel.range())
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Replaces the whole text programmatically: the history is dropped and the caret goes to
    /// the end. The preedit is cancelled.
    pub fn set_text(&mut self, s: &str) {
        let s = self.sanitize(s);
        let s = truncate_bytes(&s, self.limits.max_bytes);
        self.rope = Rope::from_text(s);
        self.sel = Selection::caret(self.rope.len());
        self.undo.clear();
        self.redo.clear();
        self.history_bytes = 0;
        self.ime.set_preedit(None);
    }

    fn sanitize(&self, s: &str) -> String {
        s.chars()
            .filter_map(|c| match c {
                '\n' | '\r' | '\u{2028}' | '\u{2029}' if self.single_line => Some(' '),
                '\n' => Some('\n'),
                '\t' => Some('\t'),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect()
    }

    // ---- boundaries -----------------------------------------------------------------------

    fn window(&self, at: usize, radius: usize) -> (usize, String) {
        let mut a = at.saturating_sub(radius);
        let mut b = (at + radius).min(self.rope.len());
        while !self.rope.is_char_boundary(a) {
            a += 1;
        }
        while !self.rope.is_char_boundary(b) {
            b -= 1;
        }
        (a, self.rope.slice(a..b))
    }

    /// The grapheme boundary before `at` (or 0).
    pub fn prev_grapheme(&self, at: usize) -> usize {
        if at == 0 {
            return 0;
        }
        let (base, w) = self.window(at, GRAPHEME_WINDOW);
        let rel = at - base;
        w.grapheme_indices(true)
            .map(|(i, _)| i)
            .take_while(|&i| i < rel)
            .last()
            .map_or(base, |i| base + i)
    }

    /// The grapheme boundary after `at` (or the end).
    pub fn next_grapheme(&self, at: usize) -> usize {
        if at >= self.rope.len() {
            return self.rope.len();
        }
        let (base, w) = self.window(at, GRAPHEME_WINDOW);
        let rel = at - base;
        for (i, g) in w.grapheme_indices(true) {
            if i + g.len() > rel {
                return base + i + g.len();
            }
        }
        self.rope.len()
    }

    fn word_start_before(&self, at: usize) -> usize {
        let (base, w) = self.window(at, WORD_WINDOW);
        let rel = at - base;
        let mut best = base;
        for (i, seg) in w.split_word_bound_indices() {
            if i >= rel {
                break;
            }
            if is_wordish(seg) {
                best = base + i;
            }
        }
        best
    }

    fn word_end_after(&self, at: usize) -> usize {
        let (base, w) = self.window(at, WORD_WINDOW);
        let rel = at - base;
        for (i, seg) in w.split_word_bound_indices() {
            if i + seg.len() > rel && is_wordish(seg) {
                return base + i + seg.len();
            }
        }
        base + w.len()
    }

    // ---- selection and movement ----------------------------------------------------------

    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        let a = self.snap(anchor);
        let h = self.snap(head);
        self.sel = Selection { anchor: a, head: h };
    }

    /// Clamps to the text and moves to the nearest grapheme boundary at or before.
    fn snap(&self, at: usize) -> usize {
        let at = at.min(self.rope.len());
        if at == 0 || at == self.rope.len() {
            return at;
        }
        let mut a = at;
        while !self.rope.is_char_boundary(a) {
            a -= 1;
        }
        let (base, w) = self.window(a, GRAPHEME_WINDOW);
        w.grapheme_indices(true)
            .map(|(i, _)| base + i)
            .take_while(|&i| i <= a)
            .last()
            .unwrap_or(base)
    }

    fn set_head(&mut self, head: usize, extend: bool) {
        self.sel.head = head;
        if !extend {
            self.sel.anchor = head;
        }
    }

    pub fn select_all(&mut self) {
        self.sel = Selection {
            anchor: 0,
            head: self.rope.len(),
        };
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend && !self.sel.is_empty() {
            let s = self.sel.start();
            self.sel = Selection::caret(s);
            return;
        }
        let h = self.prev_grapheme(self.sel.head);
        self.set_head(h, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend && !self.sel.is_empty() {
            let e = self.sel.end();
            self.sel = Selection::caret(e);
            return;
        }
        let h = self.next_grapheme(self.sel.head);
        self.set_head(h, extend);
    }

    pub fn move_word_left(&mut self, extend: bool) {
        let h = self.word_start_before(self.sel.head);
        self.set_head(h, extend);
    }

    pub fn move_word_right(&mut self, extend: bool) {
        let h = self.word_end_after(self.sel.head);
        self.set_head(h, extend);
    }

    pub fn move_home(&mut self, extend: bool) {
        self.set_head(0, extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        let n = self.rope.len();
        self.set_head(n, extend);
    }

    // ---- editing --------------------------------------------------------------------------

    /// Replaces `range` with `with` and records the step. Returns false when nothing changed.
    fn replace(
        &mut self,
        range: Range<usize>,
        with: &str,
        after_head: Option<usize>,
        group: Group,
    ) -> bool {
        if range.start == range.end && with.is_empty() {
            return false;
        }
        let removed = self.rope.slice(range.clone());
        // Respect the size limit: shorten the insertion, never refuse a pure deletion.
        let room = self
            .limits
            .max_bytes
            .saturating_sub(self.rope.len() - removed.len());
        let with = truncate_bytes(with, room);
        if removed.is_empty() && with.is_empty() {
            return false;
        }
        let before = self.sel;
        if !self.rope.remove(range.clone()) || !self.rope.insert(range.start, with) {
            return false;
        }
        let head = after_head.unwrap_or(range.start + with.len());
        self.sel = Selection::caret(head);
        let edit = Edit {
            pos: range.start,
            removed,
            inserted: with.to_string(),
            before,
            after: self.sel,
            group,
            closed: false,
        };
        self.push_edit(edit);
        true
    }

    fn push_edit(&mut self, mut e: Edit) {
        self.clear_redo();
        // Merge into the previous step when it is the same kind of run and adjacent.
        if let Some(last) = self.undo.last_mut() {
            let mergeable = !last.closed
                && last.group == e.group
                && e.group != Group::Other
                && match e.group {
                    Group::Typing => {
                        e.removed.is_empty()
                            && last.removed.is_empty()
                            && last.pos + last.inserted.len() == e.pos
                            && last.after.head == e.before.head
                            && e.before.is_empty()
                    }
                    Group::Delete => {
                        e.inserted.is_empty()
                            && last.inserted.is_empty()
                            && (e.pos + e.removed.len() == last.pos || e.pos == last.pos)
                    }
                    Group::Other => false,
                };
            if mergeable {
                let old = last.bytes();
                match e.group {
                    Group::Typing => last.inserted.push_str(&e.inserted),
                    Group::Delete => {
                        if e.pos == last.pos {
                            // Forward delete: more text removed after the same point.
                            last.removed.push_str(&e.removed);
                        } else {
                            // Backspace: the new text came before.
                            e.removed.push_str(&last.removed);
                            last.removed = std::mem::take(&mut e.removed);
                            last.pos = e.pos;
                        }
                    }
                    Group::Other => {}
                }
                last.after = e.after;
                if e.group == Group::Typing
                    && e.inserted.chars().last().is_some_and(char::is_whitespace)
                {
                    last.closed = true;
                }
                let new = last.bytes();
                self.history_bytes = self.history_bytes + new - old;
                return;
            }
        }
        if e.group == Group::Typing && e.inserted.chars().last().is_some_and(char::is_whitespace) {
            e.closed = true;
        }
        self.history_bytes += e.bytes();
        self.undo.push(e);
        self.trim_history();
    }

    fn clear_redo(&mut self) {
        for e in self.redo.drain(..) {
            self.history_bytes -= e.bytes();
        }
    }

    fn trim_history(&mut self) {
        while self.undo.len() > self.limits.max_undo
            || self.history_bytes > self.limits.max_history_bytes
        {
            if self.undo.is_empty() {
                break;
            }
            let e = self.undo.remove(0);
            self.history_bytes -= e.bytes();
        }
    }

    /// Inserts text at the caret, replacing the selection. Control characters are dropped and, in
    /// a single-line editor, line breaks become spaces.
    pub fn insert(&mut self, s: &str) -> bool {
        let s = self.sanitize(s);
        if s.is_empty() && self.sel.is_empty() {
            return false;
        }
        let r = self.sel.range();
        let group = if r.is_empty() {
            Group::Typing
        } else {
            Group::Other
        };
        self.replace(r, &s, None, group)
    }

    pub fn backspace(&mut self) -> bool {
        if !self.sel.is_empty() {
            let r = self.sel.range();
            return self.replace(r, "", None, Group::Other);
        }
        let h = self.sel.head;
        let p = self.prev_grapheme(h);
        if p == h {
            return false;
        }
        self.replace(p..h, "", Some(p), Group::Delete)
    }

    pub fn delete_forward(&mut self) -> bool {
        if !self.sel.is_empty() {
            let r = self.sel.range();
            return self.replace(r, "", None, Group::Other);
        }
        let h = self.sel.head;
        let n = self.next_grapheme(h);
        if n == h {
            return false;
        }
        self.replace(h..n, "", Some(h), Group::Delete)
    }

    pub fn delete_word_back(&mut self) -> bool {
        if !self.sel.is_empty() {
            let r = self.sel.range();
            return self.replace(r, "", None, Group::Other);
        }
        let h = self.sel.head;
        let p = self.word_start_before(h);
        if p == h {
            return false;
        }
        self.replace(p..h, "", Some(p), Group::Other)
    }

    pub fn delete_word_forward(&mut self) -> bool {
        if !self.sel.is_empty() {
            let r = self.sel.range();
            return self.replace(r, "", None, Group::Other);
        }
        let h = self.sel.head;
        let n = self.word_end_after(h);
        if n == h {
            return false;
        }
        self.replace(h..n, "", Some(h), Group::Other)
    }

    /// Removes and returns the selection (clipboard cut).
    pub fn cut(&mut self) -> Option<String> {
        if self.sel.is_empty() {
            return None;
        }
        let s = self.selected_text();
        let r = self.sel.range();
        self.replace(r, "", None, Group::Other);
        Some(s)
    }

    pub fn paste(&mut self, s: &str) -> bool {
        let s = self.sanitize(s);
        let r = self.sel.range();
        self.replace(r, &s, None, Group::Other)
    }

    pub fn undo(&mut self) -> bool {
        let Some(e) = self.undo.pop() else {
            return false;
        };
        let end = e.pos + e.inserted.len();
        self.rope.remove(e.pos..end);
        self.rope.insert(e.pos, &e.removed);
        self.sel = e.before;
        self.redo.push(e);
        self.ime.set_preedit(None);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(e) = self.redo.pop() else {
            return false;
        };
        let end = e.pos + e.removed.len();
        self.rope.remove(e.pos..end);
        self.rope.insert(e.pos, &e.inserted);
        self.sel = e.after;
        self.undo.push(e);
        self.ime.set_preedit(None);
        true
    }

    /// Ends the current typing run, so the next typed character starts a new undo step.
    pub fn break_undo_group(&mut self) {
        if let Some(l) = self.undo.last_mut() {
            l.closed = true;
        }
    }

    // ---- input method ---------------------------------------------------------------------

    pub fn ime_enable(&mut self) {
        self.ime.set_enabled(true);
    }

    pub fn ime_disable(&mut self) {
        self.ime.set_enabled(false);
    }

    pub fn ime_enabled(&self) -> bool {
        self.ime.enabled()
    }

    /// Queues `preedit_string`.
    pub fn ime_preedit_string(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        let text = self.sanitize(text);
        let cursor = cursor.filter(|(a, b)| {
            *a <= text.len()
                && *b <= text.len()
                && text.is_char_boundary(*a)
                && text.is_char_boundary(*b)
        });
        self.ime.pending_mut().preedit = Some(Preedit { text, cursor });
    }

    /// Queues `commit_string`.
    pub fn ime_commit_string(&mut self, text: &str) {
        let t = self.sanitize(text);
        self.ime.pending_mut().commit = Some(t);
    }

    /// Queues `delete_surrounding_text` (bytes before and after the cursor).
    pub fn ime_delete_surrounding(&mut self, before: usize, after: usize) {
        self.ime.pending_mut().delete = Some((before, after));
    }

    /// Applies everything queued since the last `done`, in the protocol's order. Deleting the
    /// surrounding text and committing happen as one undo step.
    pub fn ime_done(&mut self, serial: u32) {
        let Pending {
            preedit,
            commit,
            delete,
        } = self.ime.take_pending(serial);
        if !self.ime.enabled() {
            return;
        }
        // 1. The old preedit goes away (it was never in the text).
        if preedit.is_some() || commit.is_some() || delete.is_some() {
            self.ime.set_preedit(None);
        }
        // 2. and 3. Delete around the selection, then commit in its place.
        let sel = self.sel.range();
        let (mut lo, mut hi) = (sel.start, sel.end);
        if let Some((before, after)) = delete {
            lo = sel.start.saturating_sub(before);
            hi = (sel.end + after).min(self.rope.len());
            while !self.rope.is_char_boundary(lo) {
                lo += 1;
            }
            while !self.rope.is_char_boundary(hi) {
                hi -= 1;
            }
        }
        match commit {
            Some(c) => {
                self.ime.note_commit();
                self.replace(lo..hi, &c, None, Group::Other);
                self.break_undo_group();
            }
            None if delete.is_some() && (lo, hi) != (sel.start, sel.end) => {
                let mid = self.rope.slice(sel.clone());
                let anchor = self.sel.anchor <= self.sel.head;
                if self.replace(lo..hi, &mid, Some(lo + mid.len()), Group::Other) && !mid.is_empty()
                {
                    self.sel = if anchor {
                        Selection {
                            anchor: lo,
                            head: lo + mid.len(),
                        }
                    } else {
                        Selection {
                            anchor: lo + mid.len(),
                            head: lo,
                        }
                    };
                }
                self.break_undo_group();
            }
            None => {}
        }
        // 5. and 6. The new preedit sits at the cursor.
        if let Some(p) = preedit {
            self.ime.set_preedit(Some(p));
        }
    }

    /// Drops the composition without committing it (focus lost, Escape).
    pub fn ime_cancel(&mut self) {
        self.ime.set_preedit(None);
        self.ime.pending_mut().preedit = None;
    }

    pub fn preedit(&self) -> Option<&Preedit> {
        self.ime.preedit()
    }

    /// Where the preedit is shown: at the start of the selection.
    pub fn preedit_anchor(&self) -> usize {
        self.sel.start()
    }

    /// The text as displayed: the document with the preedit inserted at the caret.
    pub fn display_text(&self) -> String {
        match self.ime.preedit() {
            None => self.text(),
            Some(p) => {
                // The preedit stands in for the selection while it lasts.
                let r = self.sel.range();
                let mut s = self.rope.slice(0..r.start);
                s.push_str(&p.text);
                s.push_str(&self.rope.slice(r.end..self.rope.len()));
                s
            }
        }
    }

    /// The preedit's byte range inside [`Editor::display_text`].
    pub fn display_preedit_range(&self) -> Option<Range<usize>> {
        let p = self.ime.preedit()?;
        let at = self.preedit_anchor();
        Some(at..at + p.text.len())
    }

    /// The caret inside [`Editor::display_text`]: inside the preedit when it has a cursor.
    pub fn display_caret(&self) -> usize {
        match self.ime.preedit() {
            None => self.sel.head,
            Some(p) => {
                let at = self.preedit_anchor();
                match p.cursor {
                    Some((_, end)) => at + end,
                    None => at + p.text.len(),
                }
            }
        }
    }

    /// The text, cursor and anchor to send to the input method (preedit excluded, at most 4000
    /// bytes around the cursor).
    pub fn surrounding_text(&self) -> (String, usize, usize) {
        surrounding_window(
            &self.text(),
            self.sel.head,
            self.sel.anchor,
            SURROUNDING_LIMIT,
        )
    }

    pub fn ime_serial(&self) -> u32 {
        self.ime.serial()
    }
}

fn is_wordish(seg: &str) -> bool {
    seg.chars().any(|c| {
        c.is_alphanumeric()
            || c == '_'
            || (!c.is_whitespace()
                && !c.is_ascii_punctuation()
                && !c.is_control()
                && !is_general_punct(c))
    })
}

fn is_general_punct(c: char) -> bool {
    matches!(c, '\u{2000}'..='\u{206F}' | '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FF0F}' | '\u{FF1A}'..='\u{FF20}')
}

/// The longest prefix of `s` that fits in `max` bytes on a character boundary.
pub fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut n = max;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}
