// SPDX-License-Identifier: BSD-2-Clause
//! Property tests for the `text_input` editing core: a reference model (a `String` and a full
//! grapheme scan on every step) must agree with the rope editor after every operation of a long
//! pseudo-random sequence; undo and redo must walk the same states; the preedit must never touch
//! the text or the history.

use unicode_segmentation::UnicodeSegmentation;
use wolkit::text::edit::{Editor, Selection};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "a",
    "b",
    "hello",
    " ",
    "é",
    "e\u{301}",
    "日本",
    "語",
    "🧑‍🚀",
    "🇯🇵",
    "שלום",
    "مرحبا",
    "x y",
    "\u{301}",
    "\t",
    "ok.",
];

/// The reference: plain string, byte offsets, graphemes recomputed from scratch.
#[derive(Clone, Debug, PartialEq)]
struct Model {
    text: String,
    anchor: usize,
    head: usize,
}

impl Model {
    fn new() -> Model {
        Model {
            text: String::new(),
            anchor: 0,
            head: 0,
        }
    }

    fn sel(&self) -> Selection {
        Selection {
            anchor: self.anchor,
            head: self.head,
        }
    }

    fn bounds(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.text.grapheme_indices(true).map(|(i, _)| i).collect();
        v.push(self.text.len());
        v
    }

    fn prev(&self, at: usize) -> usize {
        self.bounds()
            .into_iter()
            .rev()
            .find(|&b| b < at)
            .unwrap_or(0)
    }

    fn next(&self, at: usize) -> usize {
        self.bounds()
            .into_iter()
            .find(|&b| b > at)
            .unwrap_or(self.text.len())
    }

    fn replace(&mut self, lo: usize, hi: usize, with: &str) {
        self.text.replace_range(lo..hi, with);
        self.head = lo + with.len();
        self.anchor = self.head;
    }

    fn lo(&self) -> usize {
        self.anchor.min(self.head)
    }

    fn hi(&self) -> usize {
        self.anchor.max(self.head)
    }

    fn insert(&mut self, s: &str) {
        let (lo, hi) = (self.lo(), self.hi());
        self.replace(lo, hi, s);
    }

    fn backspace(&mut self) {
        if self.lo() != self.hi() {
            let (lo, hi) = (self.lo(), self.hi());
            self.replace(lo, hi, "");
        } else if self.head > 0 {
            let p = self.prev(self.head);
            let h = self.head;
            self.replace(p, h, "");
        }
    }

    fn delete(&mut self) {
        if self.lo() != self.hi() {
            let (lo, hi) = (self.lo(), self.hi());
            self.replace(lo, hi, "");
        } else if self.head < self.text.len() {
            let n = self.next(self.head);
            let h = self.head;
            self.replace(h, n, "");
        }
    }

    fn left(&mut self, extend: bool) {
        if !extend && self.lo() != self.hi() {
            self.head = self.lo();
            self.anchor = self.head;
            return;
        }
        self.head = self.prev(self.head);
        if !extend {
            self.anchor = self.head;
        }
    }

    fn right(&mut self, extend: bool) {
        if !extend && self.lo() != self.hi() {
            self.head = self.hi();
            self.anchor = self.head;
            return;
        }
        self.head = self.next(self.head);
        if !extend {
            self.anchor = self.head;
        }
    }
}

fn check(ed: &Editor, m: &Model, ctx: &str) {
    assert_eq!(ed.text(), m.text, "{ctx}: text");
    assert_eq!(ed.selection(), m.sel(), "{ctx}: selection in {:?}", m.text);
    assert!(
        ed.text().is_char_boundary(ed.selection().anchor)
            && ed.text().is_char_boundary(ed.selection().head)
    );
}

fn check_sequence(seed: u64, steps: usize, with_ime: bool) {
    let mut rng = Rng(seed | 1);
    let mut ed = Editor::new(false);
    let mut m = Model::new();
    let mut states: Vec<(String, Selection)> = vec![(String::new(), Selection::default())];
    let mut last_edit_sel = Selection::default();
    for step in 0..steps {
        let ctx = format!("seed {seed} step {step}");
        let before_text = ed.text();
        let before_sel = ed.selection();
        let op = rng.below(if with_ime { 15 } else { 11 });
        match op {
            0..=3 => {
                let p = PIECES[rng.below(PIECES.len())];
                ed.insert(p);
                m.insert(p);
            }
            4 => {
                ed.backspace();
                m.backspace();
            }
            5 => {
                ed.delete_forward();
                m.delete();
            }
            6 => {
                let e = rng.below(2) == 0;
                ed.move_left(e);
                m.left(e);
            }
            7 => {
                let e = rng.below(2) == 0;
                ed.move_right(e);
                m.right(e);
            }
            8 => {
                ed.move_home(false);
                m.head = 0;
                m.anchor = 0;
            }
            9 => {
                ed.move_end(rng.below(2) == 0);
                // Extend flag was random: recompute the model the same way.
                m.head = m.text.len();
                if ed.selection().anchor == ed.selection().head {
                    m.anchor = m.head;
                } else {
                    m.anchor = ed.selection().anchor;
                }
            }
            10 => {
                ed.select_all();
                m.anchor = 0;
                m.head = m.text.len();
            }
            // IME batches: preedit only (text must not change), or commit.
            11 | 12 => {
                ed.ime_enable();
                ed.ime_preedit_string("にほ", Some((0, 3)));
                ed.ime_done(step as u32);
                assert_eq!(ed.text(), before_text, "{ctx}: preedit changed the text");
                assert!(ed.preedit().is_some());
            }
            13 => {
                ed.ime_enable();
                ed.ime_commit_string("日本");
                ed.ime_done(step as u32);
                m.insert("日本");
            }
            _ => {
                ed.ime_cancel();
                assert!(ed.preedit().is_none());
            }
        }
        // A real edit moves the model; record the undo state if the text changed.
        if ed.text() != before_text {
            states.push((before_text.clone(), before_sel));
            last_edit_sel = ed.selection();
        }
        check(&ed, &m, &ctx);
        assert!(ed.len() <= ed.limits.max_bytes);
    }
    // Undo walks back through the recorded states in order.
    let final_text = ed.text();
    let final_sel = last_edit_sel;
    ed.ime_cancel();
    // The final text is itself a state the first undo may return to (an edit that replaced text
    // with identical text still makes an undo step).
    states.push((final_text.clone(), final_sel));
    let mut idx = states.len() - 1;
    while ed.undo() {
        let t = ed.text();
        let found = states[..=idx].iter().rposition(|(s, _)| *s == t);
        assert!(
            found.is_some(),
            "seed {seed}: undo produced a text that never existed: {t:?}",
        );
        idx = found.unwrap();
    }
    assert_eq!(ed.text(), "", "seed {seed}: undo all");
    assert_eq!(
        ed.selection(),
        Selection::default(),
        "seed {seed}: undo all restores the selection"
    );
    // Redo returns to the end state exactly.
    while ed.redo() {}
    assert_eq!(ed.text(), final_text, "seed {seed}: redo all");
    assert_eq!(ed.selection(), final_sel, "seed {seed}: redo all selection");
}

#[test]
fn editor_matches_the_model_over_random_sequences() {
    for seed in 1..=40u64 {
        check_sequence(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), 400, false);
    }
}

#[test]
fn editor_with_ime_batches_keeps_the_text_and_history_exact() {
    for seed in 100..=130u64 {
        check_sequence(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), 400, true);
    }
}

#[test]
fn typing_is_grouped_by_word_for_undo() {
    let mut ed = Editor::new(true);
    for c in "hello world".chars() {
        ed.insert(&c.to_string());
    }
    assert!(ed.undo());
    assert_eq!(ed.text(), "hello ");
    assert!(ed.undo());
    assert_eq!(ed.text(), "");
    assert!(!ed.undo());
    assert!(ed.redo() && ed.redo());
    assert_eq!(ed.text(), "hello world");
}

#[test]
fn backspace_runs_undo_together() {
    let mut ed = Editor::new(true);
    ed.insert("abcdef");
    ed.break_undo_group();
    for _ in 0..3 {
        ed.backspace();
    }
    assert_eq!(ed.text(), "abc");
    assert!(ed.undo());
    assert_eq!(ed.text(), "abcdef");
    assert_eq!(ed.selection(), Selection::caret(6));
}

#[test]
fn single_line_turns_breaks_into_spaces_and_drops_controls() {
    let mut ed = Editor::new(true);
    ed.insert("a\nb\r\nc\u{0}d\u{7}");
    assert_eq!(ed.text(), "a b  cd");
    let mut ml = Editor::new(false);
    ml.insert("a\nb");
    assert_eq!(ml.text(), "a\nb");
}

#[test]
fn size_limit_is_enforced_on_character_boundaries() {
    let mut ed = Editor::new(true);
    ed.limits.max_bytes = 10;
    ed.insert("日本語日本語"); // 18 bytes
    assert_eq!(ed.text(), "日本語");
    ed.insert("x");
    assert_eq!(ed.text(), "日本語x");
    ed.insert("yy");
    assert_eq!(ed.text(), "日本語x");
    assert!(ed.len() <= 10);
    // Deleting always works at the limit.
    assert!(ed.backspace());
}

#[test]
fn word_movement_and_deletion() {
    let mut ed = Editor::new(true);
    ed.set_text("hello, brave new world");
    ed.move_home(false);
    ed.move_word_right(false);
    assert_eq!(ed.caret(), 5);
    ed.move_word_right(false);
    assert_eq!(ed.caret(), 12);
    ed.move_word_left(false);
    assert_eq!(ed.caret(), 7);
    ed.move_end(false);
    assert!(ed.delete_word_back());
    assert_eq!(ed.text(), "hello, brave new ");
    assert!(ed.undo());
    assert_eq!(ed.text(), "hello, brave new world");
}

#[test]
fn cut_paste_roundtrip() {
    let mut ed = Editor::new(true);
    ed.set_text("one two three");
    ed.set_selection(4, 7);
    assert_eq!(ed.cut().as_deref(), Some("two"));
    assert_eq!(ed.text(), "one  three");
    assert!(ed.paste("2"));
    assert_eq!(ed.text(), "one 2 three");
    assert!(ed.undo() && ed.undo());
    assert_eq!(ed.text(), "one two three");
}

#[test]
fn selection_snaps_to_grapheme_boundaries() {
    let mut ed = Editor::new(true);
    ed.set_text("ae\u{301}b");
    // Byte 3 is inside "e + combining acute".
    ed.set_selection(3, 3);
    assert_eq!(ed.caret(), 1);
    ed.set_selection(0, 999);
    assert_eq!(
        ed.selection(),
        Selection {
            anchor: 0,
            head: ed.len()
        }
    );
}

// ---- input method ---------------------------------------------------------------------------

#[test]
fn preedit_is_not_text_and_commit_is_one_undo_step() {
    let mut ed = Editor::new(true);
    ed.insert("ab");
    ed.break_undo_group();
    ed.ime_enable();
    ed.ime_preedit_string("に", Some((3, 3)));
    ed.ime_done(1);
    assert_eq!(ed.text(), "ab");
    assert_eq!(ed.display_text(), "abに");
    assert_eq!(ed.display_preedit_range(), Some(2..5));
    assert_eq!(ed.display_caret(), 5);
    ed.ime_preedit_string("にほ", Some((3, 6)));
    ed.ime_done(2);
    assert_eq!(ed.display_text(), "abにほ");
    // Commit replaces the preedit with the final text.
    ed.ime_commit_string("日本");
    ed.ime_preedit_string("", None);
    ed.ime_done(3);
    assert_eq!(ed.text(), "ab日本");
    assert!(ed.preedit().is_none());
    assert_eq!(ed.caret(), 8);
    assert!(ed.undo());
    assert_eq!(ed.text(), "ab", "the commit is a single undo step");
}

#[test]
fn cancelling_a_composition_leaves_text_and_selection_alone() {
    let mut ed = Editor::new(true);
    ed.set_text("hello");
    ed.set_selection(1, 4);
    ed.ime_enable();
    ed.ime_preedit_string("x", None);
    ed.ime_done(1);
    assert_eq!(
        ed.display_text(),
        "hxo",
        "the preedit shows where the selection starts"
    );
    ed.ime_cancel();
    assert_eq!(ed.text(), "hello");
    assert_eq!(ed.selection(), Selection { anchor: 1, head: 4 });
    assert!(!ed.can_undo());
}

#[test]
fn delete_surrounding_and_commit_replace_around_the_cursor() {
    let mut ed = Editor::new(true);
    ed.set_text("abcdef");
    ed.set_selection(3, 3);
    ed.ime_enable();
    ed.ime_delete_surrounding(1, 1);
    ed.ime_commit_string("X");
    ed.ime_done(1);
    assert_eq!(ed.text(), "abXef");
    assert_eq!(ed.caret(), 3);
    assert!(ed.undo());
    assert_eq!(ed.text(), "abcdef");
    assert_eq!(ed.selection(), Selection::caret(3));
}

#[test]
fn delete_surrounding_never_splits_a_character() {
    let mut ed = Editor::new(true);
    ed.set_text("日本語");
    ed.set_selection(3, 3);
    ed.ime_enable();
    ed.ime_delete_surrounding(1, 1); // inside the neighbouring characters
    ed.ime_done(1);
    assert!(ed.text().is_char_boundary(ed.caret()));
    assert!(std::str::from_utf8(ed.text().as_bytes()).is_ok());
}

#[test]
fn surrounding_text_excludes_the_preedit_and_is_bounded() {
    let mut ed = Editor::new(true);
    ed.limits.max_bytes = 20_000;
    ed.set_text(&"あ".repeat(3000));
    ed.set_selection(4500, 4500);
    ed.ime_enable();
    ed.ime_preedit_string("い", None);
    ed.ime_done(1);
    let (t, cur, anchor) = ed.surrounding_text();
    assert!(t.len() <= 4000 && !t.contains('い'));
    assert!(t.is_char_boundary(cur) && cur == anchor);
}

#[test]
fn disabling_the_input_method_drops_the_composition() {
    let mut ed = Editor::new(true);
    ed.ime_enable();
    ed.ime_preedit_string("かな", None);
    ed.ime_done(1);
    assert!(ed.preedit().is_some());
    ed.ime_disable();
    assert!(ed.preedit().is_none());
    // Requests while disabled do nothing.
    ed.ime_commit_string("x");
    ed.ime_done(2);
    assert_eq!(ed.text(), "");
}
