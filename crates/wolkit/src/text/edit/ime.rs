// SPDX-License-Identifier: BSD-2-Clause
//! The input-method state machine (text-input v3 semantics, no Wayland).
//!
//! The compositor sends a batch of requests (`preedit_string`, `commit_string`,
//! `delete_surrounding_text`) followed by `done(serial)`. The batch is applied atomically, in the
//! protocol's order:
//!
//! 1. replace the existing preedit with nothing,
//! 2. delete the requested surrounding text,
//! 3. insert the commit string, cursor at its end,
//! 4. compute the surrounding text to send back,
//! 5. insert the new preedit at the cursor,
//! 6. place the cursor inside the preedit.
//!
//! The preedit is *not* part of the document: it never enters the rope or the undo history, it is
//! drawn inline with an underline, and cancelling it leaves the text untouched. A commit is one
//! undo step.

use std::ops::Range;

/// The in-progress composition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    /// Cursor inside `text` as byte offsets (`begin == end` is a bare caret). `None` hides it.
    pub cursor: Option<(usize, usize)>,
}

/// Requests received since the last `done`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pending {
    /// `Some` replaces the preedit (empty text clears it).
    pub preedit: Option<Preedit>,
    pub commit: Option<String>,
    /// Bytes to delete before and after the cursor.
    pub delete: Option<(usize, usize)>,
}

impl Pending {
    pub fn is_empty(&self) -> bool {
        self.preedit.is_none() && self.commit.is_none() && self.delete.is_none()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Ime {
    enabled: bool,
    preedit: Option<Preedit>,
    pending: Pending,
    /// The serial of the last `done` we applied.
    serial: u32,
    /// How many `commit` requests the client has sent (the `done` serial the compositor expects
    /// echoes this count).
    commits: u32,
}

impl Ime {
    pub fn new() -> Ime {
        Ime::default()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    pub fn serial(&self) -> u32 {
        self.serial
    }

    pub fn commits(&self) -> u32 {
        self.commits
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.preedit = None;
            self.pending = Pending::default();
        }
    }

    pub fn pending_mut(&mut self) -> &mut Pending {
        &mut self.pending
    }

    pub fn take_pending(&mut self, serial: u32) -> Pending {
        self.serial = serial;
        std::mem::take(&mut self.pending)
    }

    pub fn set_preedit(&mut self, p: Option<Preedit>) {
        self.preedit = p.filter(|p| !p.text.is_empty());
    }

    pub fn note_commit(&mut self) {
        self.commits = self.commits.wrapping_add(1);
    }
}

/// Trims `text` to at most `limit` bytes around byte `cursor`, on character boundaries.
/// Returns the window and the cursor / anchor offsets inside it.
pub fn surrounding_window(
    text: &str,
    cursor: usize,
    anchor: usize,
    limit: usize,
) -> (String, usize, usize) {
    if text.len() <= limit {
        return (text.to_string(), cursor, anchor);
    }
    let lo_pt = cursor.min(anchor);
    let hi_pt = cursor.max(anchor);
    let span = hi_pt - lo_pt;
    let spare = limit.saturating_sub(span) / 2;
    let mut a = lo_pt.saturating_sub(spare);
    let mut b = (hi_pt + spare).min(text.len());
    while !text.is_char_boundary(a) {
        a += 1;
    }
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    // A selection longer than the limit is cut to its cursor end.
    let a2 = a.min(cursor).min(anchor).max(a);
    let _ = a2;
    let cur = cursor.clamp(a, b) - a;
    let anc = anchor.clamp(a, b) - a;
    (text[a..b].to_string(), cur, anc)
}

/// The display range of the preedit inside a string with the preedit inserted at `at`.
pub fn preedit_range(at: usize, p: &Preedit) -> Range<usize> {
    at..at + p.text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_keeps_cursor_inside_and_boundaries() {
        let text = "日本語のテキスト".repeat(50);
        let cur = 3 * 100;
        let (w, c, a) = surrounding_window(&text, cur, cur, 120);
        assert!(w.len() <= 120);
        assert!(w.is_char_boundary(c) && w.is_char_boundary(a));
        assert_eq!(c, a);
        assert_eq!(&text[cur..cur + 3], &w[c..c + 3]);
    }

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(
            surrounding_window("abc", 1, 2, 4000),
            ("abc".to_string(), 1, 2)
        );
    }

    #[test]
    fn disable_drops_preedit_and_pending() {
        let mut i = Ime::new();
        i.set_enabled(true);
        i.set_preedit(Some(Preedit {
            text: "にほ".into(),
            cursor: Some((0, 0)),
        }));
        i.pending_mut().commit = Some("x".into());
        i.set_enabled(false);
        assert!(i.preedit().is_none() && i.pending_mut().is_empty());
    }

    #[test]
    fn empty_preedit_clears() {
        let mut i = Ime::new();
        i.set_preedit(Some(Preedit {
            text: String::new(),
            cursor: None,
        }));
        assert!(i.preedit().is_none());
    }
}
