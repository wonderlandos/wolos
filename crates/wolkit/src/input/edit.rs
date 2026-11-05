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
//!
//! A `text_input` whose `white-space` is `pre-wrap` is a text area: Enter inserts a line break
//! (Ctrl+Enter submits), the text wraps at the field's width, Up and Down keep their column
//! (`text::area`), Home and End go to the ends of the line on screen, a click or a drag places the
//! caret or selects (a double click a word, a triple click a paragraph), the wheel scrolls it and
//! the field scrolls to keep the caret in view. After every change the editor's drawable state is
//! copied into the scene (`scene::EditView`) for paint.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use super::{InputState, Request, hit};
use crate::event::kind as ev;
use crate::scene::ids::{Ix, to_u32};
use crate::scene::prop::id;
use crate::scene::{EditView, Idx, Kind, NIL, NodeId, Rect, SceneRead, SceneWrite, dirty};
use crate::style::value::WhiteSpace;
use crate::text::area;
use crate::text::edit::Editor;
use crate::text::layout::TextLayout;
use crate::window::route::{disabled, receiver};
use crate::window::{MOD_CTRL, MOD_SHIFT, Sink, WindowCore};

/// Two presses closer than this in time and [`MULTI_CLICK_SLOP`] in space are a double click.
const MULTI_CLICK: Duration = Duration::from_millis(500);
const MULTI_CLICK_SLOP: f32 = 4.0;

/// The editor of the focused field.
pub struct Field {
    node: Idx,
    id: NodeId,
    pub editor: Editor,
    /// A text area (`white-space: pre-wrap`): several lines, wrapped.
    multiline: bool,
    /// The x position Up and Down aim for while a run of vertical moves lasts.
    goal_x: Option<f32>,
    /// How far the text is scrolled (logical pixels).
    scroll: (f32, f32),
    /// The caret in window coordinates, for the input method's candidate window.
    caret_rect: Option<Rect>,
    /// The primary button is down in the field: moving the pointer selects.
    dragging: bool,
    last_click: Option<Click>,
}

#[derive(Clone, Copy)]
struct Click {
    at: Instant,
    node: Idx,
    x: f32,
    y: f32,
    count: u8,
}

impl Default for Field {
    fn default() -> Self {
        Field {
            node: NIL,
            id: NodeId::ROOT,
            editor: Editor::new(true),
            multiline: false,
            goal_x: None,
            scroll: (0.0, 0.0),
            caret_rect: None,
            dragging: false,
            last_click: None,
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

/// Whether the `text_input` `n` is a text area: its white space is `pre-wrap`.
fn is_area(w: &WindowCore, n: Idx) -> bool {
    w.styles.get(w.scene.computed(n)).text.white_space == WhiteSpace::PreWrap
}

/// Whether the focused field is a text area (Enter is a line break there).
pub fn is_multiline(st: &InputState) -> bool {
    st.field.node != NIL && st.field.multiline
}

/// Attach the editor to the focused node if it is a `text_input`, or detach it. Returns the
/// input-method request the backend passes on (enable for a field, disable when leaving one).
pub fn focus_changed(w: &mut WindowCore, st: &mut InputState) -> Request {
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
        leave(w, &st.field);
    }
    if !is_field {
        st.field = Field::default();
        return if was {
            Request::Ime(false)
        } else {
            Request::None
        };
    }
    let multiline = is_area(w, f);
    let mut e = Editor::new(!multiline);
    e.set_text(w.scene.text(f));
    // A field the person can type into gets the input method; a read-only one only a caret.
    if writable(&w.scene, f) && !flag(&w.scene, f, id::SECURE) {
        e.ime_enable();
    }
    // A text area scrolled before keeps its place.
    let scroll = w.scene.edit_view(f).map_or((0.0, 0.0), |v| v.scroll);
    st.field = Field {
        node: f,
        id: w.scene.id(f),
        editor: e,
        multiline,
        scroll,
        ..Field::default()
    };
    sync_view(w, st, true);
    Request::Ime(st.field.editor.ime_enabled())
}

/// The field lost focus: it stops drawing a caret, but a scrolled text area keeps its place for
/// as long as its text stays the same.
fn leave(w: &mut WindowCore, f: &Field) {
    if !w.scene.is_live(f.node) || w.scene.id(f.node) != f.id {
        return;
    }
    if f.multiline && f.scroll != (0.0, 0.0) {
        w.scene.set_edit_view(EditView {
            node: f.node,
            id: f.id,
            active: false,
            text: String::new(),
            base: w.scene.text(f.node).to_owned(),
            caret: 0,
            selection: 0..0,
            preedit: None,
            scroll: f.scroll,
        });
    } else {
        w.scene.clear_edit_view(f.id);
    }
}

/// Where a laid-out field draws: its text laid out, the content box in window coordinates and the
/// vertical offset that centres a one-line field in its box.
struct Geo {
    layout: std::sync::Arc<TextLayout>,
    content: Rect,
    dy: f32,
}

/// Lay `shown` out the way `paint::scene` draws the text of `node`: same style, same width, so the
/// paragraph cache answers both.
fn geo(w: &mut WindowCore, node: Idx, shown: &str) -> Option<Geo> {
    if !w.scene.is_live(node) {
        return None;
    }
    let r = hit::drawn_rect(&w.scene, node);
    let props = w.styles.get(w.scene.computed(node));
    let area = props.text.white_space == WhiteSpace::PreWrap;
    let bw = props.layout.border_width;
    let [pt, pr, pb, pl] = props.layout.padding;
    let content = Rect::new(
        r.x + pl + bw,
        r.y + pt + bw,
        (r.w - pl - pr - 2.0 * bw).max(0.0),
        (r.h - pt - pb - 2.0 * bw).max(0.0),
    );
    let text = w.text.clone();
    let mut sys = crate::text::measure::lock(&text);
    let layout = w.text_cache.layout(
        &mut sys,
        shown,
        &props.text,
        area.then_some(content.w + 0.5),
    );
    let dy = if area {
        0.0
    } else {
        ((content.h - layout.height) / 2.0).max(0.0)
    };
    Some(Geo {
        layout,
        content,
        dy,
    })
}

/// What a field draws for `text`: a secure field draws one bullet (three bytes) per character.
fn shown(text: &str, secure: bool) -> Cow<'_, str> {
    if secure {
        Cow::Owned("\u{2022}".repeat(text.chars().count()))
    } else {
        Cow::Borrowed(text)
    }
}

/// A byte offset of `text` as an offset of what [`shown`] draws.
fn shown_at(text: &str, at: usize, secure: bool) -> usize {
    if secure {
        text[..at.min(text.len())].chars().count() * 3
    } else {
        at
    }
}

/// Copy the focused field's drawable state into the scene for paint, and keep the caret in view
/// when `reveal` (after the caret or the text moved; a wheel scroll passes false).
fn sync_view(w: &mut WindowCore, st: &mut InputState, reveal: bool) {
    let f = &st.field;
    if f.node == NIL {
        return;
    }
    let (node, secure) = (f.node, flag(&w.scene, f.node, id::SECURE));
    let text = f.editor.display_text();
    let draw = shown(&text, secure).into_owned();
    let Some(g) = geo(w, node, &draw) else {
        return;
    };
    let caret = f.editor.display_caret();
    let cp = area::caret_pos(&g.layout, &draw, shown_at(&text, caret, secure));
    let view = (g.content.w, g.content.h);
    let mut scroll = f.scroll;
    if reveal {
        scroll = area::scroll_to_caret(scroll, view, &cp, f.multiline);
    }
    // One more pixel of room at the end of a line, for the caret after the last character.
    let content = (g.layout.width + 1.0, g.layout.height);
    scroll = area::clamp_scroll(scroll, content, view);
    if f.multiline {
        scroll.0 = 0.0;
    } else {
        scroll.1 = 0.0;
    }
    let preedit = f.editor.display_preedit_range();
    // While there is a preedit it stands in for the selection.
    let selection = if preedit.is_some() {
        caret..caret
    } else {
        f.editor.selection().range()
    };
    let v = EditView {
        node,
        id: f.id,
        active: true,
        text,
        base: w.scene.text(node).to_owned(),
        caret,
        selection,
        preedit,
        scroll,
    };
    let rect = Rect::new(
        g.content.x + cp.x - scroll.0,
        g.content.y + g.dy + cp.top - scroll.1,
        1.0,
        cp.height,
    );
    let f = &mut st.field;
    f.scroll = scroll;
    f.caret_rect = Some(rect);
    w.scene.set_edit_view(v);
}

/// After the app patched the scene: a focused field whose text the app replaced takes the new text
/// (caret at the end), and what paint draws follows.
pub fn refresh(w: &mut WindowCore, st: &mut InputState) {
    if field(w, st).is_some() {
        sync_view(w, st, false);
    }
}

/// The field, re-read from the node when the app changed its text since the last edit.
fn field<'a>(w: &WindowCore, st: &'a mut InputState) -> Option<&'a mut Field> {
    let f = &mut st.field;
    if f.node == NIL || !w.scene.is_live(f.node) || w.scene.id(f.node) != f.id {
        return None;
    }
    let text = w.scene.text(f.node);
    // The app's text may differ from the editor's only by what the editor normalises (a `\r\n`,
    // the size limit): that is not a replacement, and must not reset the caret and the history.
    let cur = f.editor.text();
    if cur != text && f.editor.normalised(text) != cur {
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

/// Write the editor's text to the node and report it, if it changed; then bring what paint draws
/// (caret, selection, scroll) up to date.
fn write_back(w: &mut WindowCore, s: &mut Sink, st: &mut InputState) {
    write_node(w, s, st);
    sync_view(w, st, true);
}

fn write_node(w: &mut WindowCore, s: &mut Sink, st: &mut InputState) {
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
    st.field.goal_x = None;
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
    st.field.goal_x = None;
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
    if st.field.multiline && area_key(w, st, keysym, mods) {
        return Some(Request::Ime(st.field.editor.ime_enabled()));
    }
    st.field.goal_x = None;
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
            sync_view(w, st, false);
            return Some(Request::None);
        }
        _ => return None,
    };
    if changed {
        write_back(w, s, st);
    } else {
        // The caret or the selection moved: redraw it, and tell the input method.
        sync_view(w, st, true);
    }
    Some(Request::Ime(st.field.editor.ime_enabled()))
}

/// Up, Down, the page keys, Home and End in a text area: they move by what is on screen, so they
/// need the text laid out. Ctrl+Home and Ctrl+End stay the editor's (start and end of the text).
fn area_key(w: &mut WindowCore, st: &mut InputState, keysym: u32, mods: u32) -> bool {
    use super::keys::sym;
    if mods & MOD_CTRL != 0
        || !matches!(
            keysym,
            sym::UP | sym::DOWN | sym::PAGE_UP | sym::PAGE_DOWN | sym::HOME | sym::END
        )
    {
        return false;
    }
    let node = st.field.node;
    let secure = flag(&w.scene, node, id::SECURE);
    let doc = st.field.editor.text();
    // Geometry follows what is painted: a secure field draws bullets.
    let text = shown(&doc, secure).into_owned();
    let Some(g) = geo(w, node, &text) else {
        return false;
    };
    let caret = shown_at(&doc, st.field.editor.caret(), secure);
    let page = isize::try_from(area::lines_per_page(&g.layout, g.content.h))
        .unwrap_or(1)
        .saturating_sub(1)
        .max(1);
    let goal = st.field.goal_x;
    let (drawn, goal) = match keysym {
        sym::UP => {
            let (t, x) = area::move_vertical(&g.layout, &text, caret, goal, -1);
            (t, Some(x))
        }
        sym::DOWN => {
            let (t, x) = area::move_vertical(&g.layout, &text, caret, goal, 1);
            (t, Some(x))
        }
        sym::PAGE_UP => {
            let (t, x) = area::move_vertical(&g.layout, &text, caret, goal, -page);
            (t, Some(x))
        }
        sym::PAGE_DOWN => {
            let (t, x) = area::move_vertical(&g.layout, &text, caret, goal, page);
            (t, Some(x))
        }
        sym::HOME => (area::line_edges(&g.layout, &text, caret).0, None),
        _ => (area::line_edges(&g.layout, &text, caret).1, None),
    };
    let to = text_at(&doc, drawn, secure);
    st.field.editor.move_to(to, mods & MOD_SHIFT != 0);
    st.field.goal_x = goal;
    sync_view(w, st, true);
    true
}

/// A byte offset of what a field draws as an offset of its text (see [`shown_at`]).
fn text_at(text: &str, drawn: usize, secure: bool) -> usize {
    if !secure {
        return drawn;
    }
    text.char_indices()
        .nth(drawn / 3)
        .map_or(text.len(), |(i, _)| i)
}

/// A press of the primary button at window point `(x, y)` in the focused field: places the caret
/// (Shift extends the selection), a second press in the same place selects the word, a third the
/// paragraph. The press starts a drag.
pub fn pointer_down(w: &mut WindowCore, st: &mut InputState, x: f32, y: f32, shift: bool) {
    let node = st.field.node;
    if node == NIL || field(w, st).is_none() {
        return;
    }
    let secure = flag(&w.scene, node, id::SECURE);
    let text = st.field.editor.text();
    let draw = shown(&text, secure).into_owned();
    let Some(g) = geo(w, node, &draw) else {
        return;
    };
    let at = text_at(&text, point_at(&st.field, &g, &draw, x, y), secure);
    let now = Instant::now();
    let count = match st.field.last_click {
        Some(c)
            if c.node == node
                && now.duration_since(c.at) <= MULTI_CLICK
                && (c.x - x).abs() <= MULTI_CLICK_SLOP
                && (c.y - y).abs() <= MULTI_CLICK_SLOP =>
        {
            c.count % 3 + 1
        }
        _ => 1,
    };
    st.field.last_click = Some(Click {
        at: now,
        node,
        x,
        y,
        count,
    });
    let e = &mut st.field.editor;
    match count {
        1 => e.move_to(at, shift),
        2 => {
            let r = area::word_at(&text, at);
            e.set_selection(r.start, r.end);
        }
        _ => {
            let r = area::paragraph_at(&text, at);
            e.set_selection(r.start, r.end);
        }
    }
    st.field.dragging = true;
    st.field.goal_x = None;
    sync_view(w, st, true);
}

/// The byte offset of `draw` under window point `(x, y)`.
fn point_at(f: &Field, g: &Geo, draw: &str, x: f32, y: f32) -> usize {
    area::hit(
        &g.layout,
        draw,
        x - g.content.x + f.scroll.0,
        y - g.content.y - g.dy + f.scroll.1,
    )
}

/// The pointer moved with the primary button held in the focused field: extend the selection.
pub fn pointer_drag(w: &mut WindowCore, st: &mut InputState, x: f32, y: f32) {
    let node = st.field.node;
    if node == NIL || !st.field.dragging || field(w, st).is_none() {
        return;
    }
    let secure = flag(&w.scene, node, id::SECURE);
    let text = st.field.editor.text();
    let draw = shown(&text, secure).into_owned();
    let Some(g) = geo(w, node, &draw) else {
        return;
    };
    let at = text_at(&text, point_at(&st.field, &g, &draw, x, y), secure);
    st.field.editor.move_to(at, true);
    sync_view(w, st, true);
}

/// The primary button was released.
pub fn pointer_up(st: &mut InputState) {
    st.field.dragging = false;
}

/// The wheel over a text area scrolls it by `dy` logical pixels. False when it has nowhere to go
/// (not a text area, or already at that end), so the scroll container around it takes the wheel.
pub fn wheel(w: &mut WindowCore, st: &mut InputState, n: Idx, dy: f32) -> bool {
    if w.scene.kind(n) != Kind::TextInput || !is_area(w, n) {
        return false;
    }
    let nid = w.scene.id(n);
    let focused = st.field.node == n && st.field.id == nid && field(w, st).is_some();
    let text = if focused {
        st.field.editor.display_text()
    } else {
        w.scene.text(n).to_owned()
    };
    let Some(g) = geo(w, n, &text) else {
        return false;
    };
    let cur = if focused {
        st.field.scroll
    } else {
        // The scroll of a view made for other text is no scroll (see `paint::scene`).
        w.scene
            .edit_view(n)
            .filter(|v| v.base == w.scene.text(n))
            .map_or((0.0, 0.0), |v| v.scroll)
    };
    let view = (g.content.w, g.content.h);
    let next = area::clamp_scroll(
        (0.0, cur.1 + dy),
        (g.layout.width + 1.0, g.layout.height),
        view,
    );
    if next == cur {
        return false;
    }
    if focused {
        st.field.scroll = next;
        sync_view(w, st, false);
    } else {
        w.scene.set_edit_view(EditView {
            node: n,
            id: nid,
            active: false,
            text: String::new(),
            base: w.scene.text(n).to_owned(),
            caret: 0,
            selection: 0..0,
            preedit: None,
            scroll: next,
        });
    }
    true
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

/// Where the input method should put its candidate window: the caret in window coordinates (the
/// field's rectangle until the field has been laid out).
pub fn cursor_rect(w: &WindowCore, st: &InputState) -> Option<crate::scene::Rect> {
    let n = st.field.node;
    (n != NIL && w.scene.is_live(n)).then(|| {
        st.field
            .caret_rect
            .unwrap_or_else(|| super::hit::drawn_rect(&w.scene, n))
    })
}
