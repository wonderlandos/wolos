// SPDX-License-Identifier: BSD-2-Clause
//! The focused `text_input`: one [`Editor`] (WK-W3's rope, selection, undo and input-method state
//! machine) bound to the node while it has focus. Typed text, editing keys, clipboard text and
//! input-method batches go through it; every change of the committed text is written back to the
//! node and reported as `TEXT_CHANGED` (`u32 caret`, `u32 anchor`, `u32 length`, the text; byte
//! offsets), so the app's controlled value follows. The preedit is never written to the node:
//! it lives in the editor until the input method commits it ([`Field::display`] is what to draw).
//!
//! A patch that sets the node's text while it is focused wins: the editor takes the new text
//! (caret at the end) the next time input reaches it.

use super::{InputState, Request};
use crate::event::kind as ev;
use crate::scene::ids::{Ix, to_u32};
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, NodeId, SceneRead, SceneWrite, dirty};
use crate::text::edit::Editor;
use crate::window::route::{disabled, receiver};
use crate::window::{MOD_CTRL, MOD_SHIFT, Sink, WindowCore};

/// The editor of the focused field.
pub struct Field {
    node: Idx,
    id: NodeId,
    pub editor: Editor,
}

impl Default for Field {
    fn default() -> Self {
        Field {
            node: NIL,
            id: NodeId::ROOT,
            editor: Editor::new(true),
        }
    }
}

/// What to draw for the focused field: the text with the preedit inside, the preedit's range (to
/// underline) and the caret, as byte offsets into `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Display {
    pub node: Idx,
    pub text: String,
    pub preedit: Option<std::ops::Range<usize>>,
    pub caret: usize,
    pub selection: std::ops::Range<usize>,
}

impl Field {
    /// The node being edited, or NIL.
    pub fn node(&self) -> Idx {
        self.node
    }

    pub fn display(&self) -> Option<Display> {
        (self.node != NIL).then(|| Display {
            node: self.node,
            text: self.editor.display_text(),
            preedit: self.editor.display_preedit_range(),
            caret: self.editor.display_caret(),
            selection: self.editor.selection().range(),
        })
    }
}

fn flag(scene: &dyn SceneRead, n: Idx, p: u16) -> bool {
    scene.prop(n, p).is_some_and(|v| v.w[0] == 1)
}

/// Whether the person may change the field's text.
fn writable(scene: &dyn SceneRead, n: Idx) -> bool {
    !flag(scene, n, id::READONLY) && !disabled(scene, n)
}

/// Attach the editor to the focused node if it is a `text_input`, or detach it. Returns the
/// input-method request the backend passes on (enable for a field, disable when leaving one).
pub fn focus_changed(w: &WindowCore, st: &mut InputState) -> Request {
    let f = st.focused;
    let is_field = f != NIL && w.scene.kind(f) == Kind::TextInput;
    let same = is_field && st.field.node == f && st.field.id == w.scene.id(f);
    if same {
        return Request::None;
    }
    let was = st.field.node != NIL;
    if was {
        st.field.editor.ime_cancel();
        st.field.editor.ime_disable();
    }
    if !is_field {
        st.field.node = NIL;
        return if was {
            Request::Ime(false)
        } else {
            Request::None
        };
    }
    let mut e = Editor::new(true);
    e.set_text(w.scene.text(f));
    // A field the person can type into gets the input method; a read-only one only a caret.
    if writable(&w.scene, f) && !flag(&w.scene, f, id::SECURE) {
        e.ime_enable();
    }
    st.field = Field {
        node: f,
        id: w.scene.id(f),
        editor: e,
    };
    Request::Ime(st.field.editor.ime_enabled())
}

/// The field, re-read from the node when the app changed its text since the last edit.
fn field<'a>(w: &WindowCore, st: &'a mut InputState) -> Option<&'a mut Field> {
    let f = &mut st.field;
    if f.node == NIL || !w.scene.is_live(f.node) || w.scene.id(f.node) != f.id {
        return None;
    }
    let text = w.scene.text(f.node);
    if f.editor.len() != text.len() || f.editor.text() != text {
        f.editor.set_text(text);
    }
    Some(f)
}

/// Characters the field may still take, if it has a `max-length`, given what replaces the
/// selection.
fn room(w: &WindowCore, f: &Field) -> Option<usize> {
    let max = w
        .scene
        .prop(f.node, id::MAX_LENGTH)
        .map_or(0, |v| v.w[0].cast_signed());
    if max <= 0 {
        return None;
    }
    let text = f.editor.text();
    let sel = f.editor.selection().range();
    let kept = text[..sel.start].chars().count() + text[sel.end..].chars().count();
    Some(usize::try_from(max).unwrap_or(0).saturating_sub(kept))
}

fn clip(s: &str, chars: Option<usize>) -> &str {
    match chars {
        Some(n) => s.char_indices().nth(n).map_or(s, |(at, _)| &s[..at]),
        None => s,
    }
}

/// Write the editor's text to the node and report it, if it changed.
fn write_back(w: &mut WindowCore, s: &mut Sink, st: &mut InputState) {
    let f = &st.field;
    let node = f.node;
    let text = f.editor.text();
    let sel = f.editor.selection();
    let mut cur = w.scene.text[node.ix()];
    match w.scene.set_str(&mut cur, &text) {
        Some(true) => {
            let old = w.scene.text[node.ix()];
            w.scene.text[node.ix()] = cur;
            w.scene.strings.release(old);
            w.scene
                .mark_dirty(node, dirty::LAYOUT | dirty::PAINT | dirty::A11Y);
            w.scene.version += 1;
        }
        // Same text: only the caret or the preedit moved, which is drawn, not reported.
        Some(false) => {
            w.scene.mark_dirty(node, dirty::PAINT | dirty::A11Y);
            return;
        }
        None => return,
    }
    if let Some(r) = receiver(&w.scene, node, ev::TEXT_CHANGED) {
        let head = words([to_u32(sel.head), to_u32(sel.anchor), to_u32(text.len())]);
        s.emit(ev::TEXT_CHANGED, w.scene.id(r).0, &head, text.as_bytes());
    }
}

fn words<const N: usize>(v: [u32; N]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Typed text (a key's text or an injected string) at the caret, replacing the selection.
pub fn type_text(w: &mut WindowCore, s: &mut Sink, st: &mut InputState, t: &str) -> bool {
    let room = match field(w, st) {
        Some(f) if writable(&w.scene, f.node) => room(w, f),
        _ => return false,
    };
    let t = clip(t, room);
    if t.is_empty() || !st.field.editor.insert(t) {
        return false;
    }
    write_back(w, s, st);
    true
}

/// Clipboard text arrived for a paste.
pub fn paste(w: &mut WindowCore, s: &mut Sink, st: &mut InputState, t: &str) -> bool {
    let room = match field(w, st) {
        Some(f) if writable(&w.scene, f.node) => room(w, f),
        _ => return false,
    };
    if !st.field.editor.paste(clip(t, room)) {
        return false;
    }
    write_back(w, s, st);
    true
}

/// An editing key in the focused field. `None` when the key is not the field's (Tab, Escape
/// without a preedit, a plain shortcut): the caller routes it on.
pub fn key(
    w: &mut WindowCore,
    s: &mut Sink,
    st: &mut InputState,
    keysym: u32,
    mods: u32,
) -> Option<Request> {
    use super::keys::sym;
    let node = field(w, st)?.node;
    let can_write = writable(&w.scene, node);
    let secure = flag(&w.scene, node, id::SECURE);
    let (shift, ctrl) = (mods & MOD_SHIFT != 0, mods & MOD_CTRL != 0);
    let e = &mut st.field.editor;
    let changed = match (keysym, ctrl) {
        (sym::LEFT, false) => {
            e.move_left(shift);
            false
        }
        (sym::RIGHT, false) => {
            e.move_right(shift);
            false
        }
        (sym::LEFT, true) => {
            e.move_word_left(shift);
            false
        }
        (sym::RIGHT, true) => {
            e.move_word_right(shift);
            false
        }
        (sym::HOME, _) => {
            e.move_home(shift);
            false
        }
        (sym::END, _) => {
            e.move_end(shift);
            false
        }
        (sym::BACKSPACE, false) if can_write => e.backspace(),
        (sym::BACKSPACE, true) if can_write => e.delete_word_back(),
        (sym::DELETE, false) if can_write => e.delete_forward(),
        (sym::DELETE, true) if can_write => e.delete_word_forward(),
        (sym::BACKSPACE | sym::DELETE, _) => false,
        (sym::A | sym::A_UPPER, true) => {
            e.select_all();
            false
        }
        (sym::Z, true) if can_write && !shift => e.undo(),
        (sym::Z | sym::Z_UPPER | sym::Y, true) if can_write => e.redo(),
        (sym::C, true) => {
            let t = e.selected_text();
            return Some(if secure || t.is_empty() {
                Request::None
            } else {
                Request::Copy(t)
            });
        }
        (sym::X, true) => {
            if secure || !can_write {
                return Some(Request::None);
            }
            let Some(t) = e.cut() else {
                return Some(Request::None);
            };
            write_back(w, s, st);
            return Some(Request::Copy(t));
        }
        (sym::V, true) => {
            return Some(if can_write {
                Request::Paste
            } else {
                Request::None
            });
        }
        (sym::ESCAPE, _) if e.preedit().is_some() => {
            e.ime_cancel();
            w.scene.mark_dirty(node, dirty::PAINT);
            return Some(Request::None);
        }
        _ => return None,
    };
    if changed {
        write_back(w, s, st);
    } else {
        // The caret or the selection moved: redraw it, and tell the input method.
        w.scene.mark_dirty(node, dirty::PAINT);
    }
    Some(Request::Ime(st.field.editor.ime_enabled()))
}

/// Enter in the field: `TEXT_SUBMIT` with the committed text.
pub fn submit(w: &mut WindowCore, s: &mut Sink, st: &mut InputState) -> bool {
    let Some(f) = field(w, st) else {
        return false;
    };
    let (node, sel) = (f.node, f.editor.selection());
    if let Some(r) = receiver(&w.scene, node, ev::TEXT_SUBMIT) {
        let text = w.scene.text(node).to_owned();
        let head = words([to_u32(sel.head), to_u32(sel.anchor), to_u32(text.len())]);
        s.emit(ev::TEXT_SUBMIT, w.scene.id(r).0, &head, text.as_bytes());
    }
    true
}

/// One input-method request, queued until [`ime_done`] (text-input-v3 semantics).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ime {
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    Commit(String),
    DeleteSurrounding {
        before: usize,
        after: usize,
    },
}

/// Queue a request of the current batch.
pub fn ime(w: &WindowCore, st: &mut InputState, r: Ime) {
    let Some(f) = field(w, st) else {
        return;
    };
    let room = room(w, f);
    let e = &mut st.field.editor;
    match r {
        Ime::Preedit { text, cursor } => e.ime_preedit_string(&text, cursor),
        Ime::Commit(t) => e.ime_commit_string(clip(&t, room)),
        Ime::DeleteSurrounding { before, after } => e.ime_delete_surrounding(before, after),
    }
}

/// The batch ends (`done(serial)`): apply it atomically and report the new text.
pub fn ime_done(w: &mut WindowCore, s: &mut Sink, st: &mut InputState, serial: u32) {
    let Some(f) = field(w, st) else {
        return;
    };
    if !writable(&w.scene, f.node) {
        f.editor.ime_cancel();
        return;
    }
    f.editor.ime_done(serial);
    write_back(w, s, st);
}

/// The text, cursor and anchor to send to the input method, if a field has it.
pub fn surrounding(st: &InputState) -> Option<(String, usize, usize)> {
    (st.field.node != NIL && st.field.editor.ime_enabled())
        .then(|| st.field.editor.surrounding_text())
}

/// Where the input method should put its candidate window: the field, in window coordinates.
/// (The caret's own x comes with the text layout of the field; until the pipeline keeps one per
/// node, the field's rectangle is the hint.)
pub fn cursor_rect(w: &WindowCore, st: &InputState) -> Option<crate::scene::Rect> {
    let n = st.field.node;
    (n != NIL && w.scene.is_live(n)).then(|| super::hit::drawn_rect(&w.scene, n))
}
