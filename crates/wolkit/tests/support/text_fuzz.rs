// SPDX-License-Identifier: BSD-2-Clause
//! The property of the `wolkit_text` fuzz target (`fuzz/fuzz_targets/wolkit_text.rs`), shared with
//! `tests/text_area.rs` so the same code runs under libFuzzer and, on stable, over the committed
//! seeds and a deterministic mutation of them.
//!
//! The input is a program for the text buffer: a flag byte (one-line or multi-line editor, wrap
//! width), then operations, each a byte and, where it needs one, a payload byte: insert a piece of
//! text, paste, the delete and move commands, set the selection, undo, redo, cut, the input
//! method's batches. After every operation the buffer must be sound: the selection inside the
//! text on character boundaries, the size within the limit, no carriage return, a one-line editor
//! without a line break, the preedit inside the displayed text. Every few operations the text is
//! laid out (without fonts: every glyph is the missing box, which still wraps) and the geometry
//! (`text::area`) is asked for the caret, a vertical move, the line ends and a hit test; each
//! answer is an offset inside the text on a character boundary. At the end, redoing what is left,
//! then undoing everything restores the empty text and the starting selection exactly, and redoing
//! everything restores the text at the top of the history.
#![allow(dead_code)] // each user of this file uses some of it

use wolkit::env::Environment;
use wolkit::event::EventQueue;
use wolkit::input::keys::{KeyPress, sym};
use wolkit::input::{BUTTON_LEFT, InputState, edit, focus, keys, pointer};
use wolkit::scene::patch::{APPEND, Patch, Writer};
use wolkit::scene::prop::{DIM_PX, id};
use wolkit::scene::{Kind, SceneRead};
use wolkit::text::area;
use wolkit::text::cache::ShapeCache;
use wolkit::text::edit::{Editor, Limits};
use wolkit::text::fontdb::{FontDb, FontId};
use wolkit::text::layout::{LayoutOptions, TextLayout, TextStyle, layout_text};
use wolkit::window::{
    MOD_CTRL, MOD_SHIFT, POINTER_DOWN, POINTER_MOVE, POINTER_SCROLL, POINTER_UP, PointerInput,
    Sink, WindowCore,
};

const PIECES: &[&str] = &[
    "a",
    "b c",
    "hello",
    " ",
    "\n",
    "\n\n",
    "x\ny",
    "\r\n",
    "\r",
    "é",
    "e\u{301}",
    "日本\n",
    "🧑‍🚀",
    "🇯🇵",
    "שלום",
    "مرحبا",
    "\t",
    "\u{2029}",
    "\u{0}",
    "\u{301}",
    "a long word that wraps when the field is narrow enough to need it",
];

/// What a run covered, for the test that mutates seeds to check the mutants get somewhere.
#[derive(Default, Debug, Clone, Copy)]
pub struct Stats {
    pub ops: usize,
    pub edits: usize,
    pub geometry: usize,
    pub undone: usize,
}

struct Bytes<'a>(&'a [u8], usize);

impl Bytes<'_> {
    fn next(&mut self) -> Option<u8> {
        let b = self.0.get(self.1).copied();
        self.1 += 1;
        b
    }
}

fn check_sound(ed: &Editor, ctx: &str) {
    let t = ed.text();
    let s = ed.selection();
    assert_eq!(ed.len(), t.len(), "{ctx}: len");
    assert!(t.len() <= ed.limits.max_bytes, "{ctx}: over the size limit");
    assert!(
        s.anchor <= t.len() && s.head <= t.len(),
        "{ctx}: selection outside the text"
    );
    assert!(
        t.is_char_boundary(s.anchor) && t.is_char_boundary(s.head),
        "{ctx}: selection inside a character"
    );
    assert!(!t.contains('\r'), "{ctx}: carriage return kept");
    if ed.is_single_line() {
        assert!(
            !t.contains(['\n', '\u{2028}', '\u{2029}']),
            "{ctx}: break in a one-line editor"
        );
    }
    let shown = ed.display_text();
    if let Some(r) = ed.display_preedit_range() {
        assert!(
            r.end <= shown.len()
                && shown.is_char_boundary(r.start)
                && shown.is_char_boundary(r.end),
            "{ctx}: preedit outside the displayed text"
        );
    }
    assert!(
        shown.is_char_boundary(ed.display_caret().min(shown.len()))
            && ed.display_caret() <= shown.len(),
        "{ctx}: display caret"
    );
}

fn lay(text: &str, width: f32, cache: &mut ShapeCache, db: &FontDb) -> TextLayout {
    let st = TextStyle::new(FontId(0), 16.0);
    let opts = LayoutOptions {
        max_width: width,
        ..LayoutOptions::default()
    };
    layout_text(db, cache, text, &st, &opts)
}

fn check_geometry(
    text: &str,
    caret: usize,
    width: f32,
    probe: (f32, f32),
    cache: &mut ShapeCache,
    db: &FontDb,
) {
    let l = lay(text, width, cache, db);
    // Lines tile the paragraphs and stay inside the text.
    for line in &l.lines {
        assert!(line.range.start <= line.range.end && line.range.end <= text.len());
        assert!(text.is_char_boundary(line.range.start) && text.is_char_boundary(line.range.end));
    }
    let ok = |at: usize, what: &str| {
        assert!(
            at <= text.len() && text.is_char_boundary(at),
            "{what}: {at} in a text of {}",
            text.len()
        );
    };
    let caret = caret.min(text.len());
    let cp = area::caret_pos(&l, text, caret);
    assert!(cp.line < l.lines.len().max(1) && cp.x.is_finite());
    let mut goal = None;
    let mut at = caret;
    for step in [1isize, 1, -1, -3, 5, -9] {
        let (to, g) = area::move_vertical(&l, text, at, goal, step);
        ok(to, "move_vertical");
        assert!(g.is_finite());
        goal = Some(g);
        at = to;
    }
    let (a, b) = area::line_edges(&l, text, caret);
    ok(a, "line start");
    ok(b, "line end");
    assert!(a <= b, "line edges out of order: {a} {b}");
    ok(area::hit(&l, text, probe.0, probe.1), "hit");
    let w = area::word_at(text, caret);
    let p = area::paragraph_at(text, caret);
    assert!(
        w.start <= w.end
            && w.end <= text.len()
            && text.is_char_boundary(w.start)
            && text.is_char_boundary(w.end)
    );
    assert!(
        p.start <= p.end
            && p.end <= text.len()
            && text.is_char_boundary(p.start)
            && text.is_char_boundary(p.end)
    );
    let c = area::scroll_to_caret((0.0, 0.0), (width.min(400.0), 40.0), &cp, true);
    assert!(c.0 == 0.0 && c.1 >= 0.0);
    let _ = area::lines_per_page(&l, 100.0);
}

pub fn run(data: &[u8]) -> Stats {
    let mut st = Stats::default();
    let mut by = Bytes(data, 0);
    let Some(flags) = by.next() else {
        return st;
    };
    if flags & 0x80 != 0 {
        return run_engine(&mut by, flags);
    }
    let mut ed = Editor::new(flags & 1 == 1);
    ed.limits = Limits {
        max_bytes: 2048,
        ..Limits::default()
    };
    let width = if flags & 2 == 0 {
        f32::INFINITY
    } else {
        24.0 + f32::from(flags >> 2) * 3.0
    };
    let db = FontDb::new();
    let mut cache = ShapeCache::new(64);
    let start = ed.selection();
    // Beyond this many operations the history limit could drop the oldest step.
    while st.ops < 900 {
        let Some(op) = by.next() else {
            break;
        };
        st.ops += 1;
        let before = ed.text();
        let arg = by.next().unwrap_or(0);
        let extend = arg & 1 == 1;
        match op % 24 {
            0..=4 => {
                ed.insert(PIECES[usize::from(arg) % PIECES.len()]);
            }
            5 => {
                // A raw run of bytes, taken as text the way a clipboard would give it.
                let n = usize::from(arg % 12);
                let raw: Vec<u8> = (0..n).filter_map(|_| by.next()).collect();
                ed.paste(&String::from_utf8_lossy(&raw));
            }
            6 => {
                ed.backspace();
            }
            7 => {
                ed.delete_forward();
            }
            8 => {
                ed.delete_word_back();
            }
            9 => {
                ed.delete_word_forward();
            }
            10 => ed.move_left(extend),
            11 => ed.move_right(extend),
            12 => ed.move_word_left(extend),
            13 => ed.move_word_right(extend),
            14 => ed.move_home(extend),
            15 => ed.move_end(extend),
            16 => {
                let n = ed.len() + 1;
                ed.move_to(usize::from(arg >> 1) * 7 % n, extend);
            }
            17 => {
                let n = ed.len() + 1;
                let b = by.next().unwrap_or(0);
                ed.set_selection(usize::from(arg) * 5 % n, usize::from(b) * 3 % n);
            }
            18 => {
                ed.undo();
            }
            19 => {
                ed.redo();
            }
            20 => {
                if let Some(t) = ed.cut() {
                    ed.paste(&t);
                }
            }
            21 => ed.select_all(),
            22 => {
                // An input-method batch: a preedit, then a commit, or a delete around the caret.
                ed.ime_enable();
                let serial = ed.ime_serial().wrapping_add(1);
                match arg % 3 {
                    0 => ed.ime_preedit_string(
                        PIECES[usize::from(arg >> 2) % PIECES.len()],
                        Some((0, 1)),
                    ),
                    1 => ed.ime_commit_string(PIECES[usize::from(arg >> 2) % PIECES.len()]),
                    _ => ed.ime_delete_surrounding(usize::from(arg >> 2), usize::from(arg >> 4)),
                }
                ed.ime_done(serial);
            }
            _ => ed.break_undo_group(),
        }
        if ed.text() != before {
            st.edits += 1;
        }
        check_sound(&ed, &format!("op {} ({op})", st.ops));
        if st.ops % 6 == 0 {
            let probe = (f32::from(arg) * 1.5, f32::from(op) * 0.9);
            check_geometry(
                &ed.display_text(),
                ed.display_caret(),
                width,
                probe,
                &mut cache,
                &db,
            );
            st.geometry += 1;
        }
    }
    ed.ime_cancel();
    // Everything the program undid and did not redo is still on the redo stack: redo it first, so
    // the text at the top of the history is the one the walk back has to return to.
    while ed.redo() {}
    let top = ed.text();
    check_sound(&ed, "redo to the top");
    while ed.undo() {
        st.undone += 1;
        check_sound(&ed, "undo");
    }
    assert_eq!(ed.text(), "", "undoing everything restores the empty text");
    assert_eq!(ed.selection(), start, "and the starting selection");
    while ed.redo() {
        check_sound(&ed, "redo");
    }
    assert_eq!(
        ed.text(),
        top,
        "redoing everything restores the text at the top of the history"
    );
    st
}

// ---- the editor inside a window --------------------------------------------------------------

/// The text field of `run_engine`: node 1 on a window core that has no fonts (the process's shared
/// font set is empty unless something installs one, so every glyph is the missing box: the layout
/// is the same on every machine, which a fuzz input needs to reproduce).
struct Rig {
    core: WindowCore,
    events: EventQueue,
    st: InputState,
    seq: u32,
}

impl Rig {
    fn apply(&mut self, f: impl FnOnce(&mut Writer)) {
        let mut w = Writer::new();
        f(&mut w);
        self.seq += 1;
        let bytes = w.finish(self.seq).to_vec();
        let c = &mut self.core;
        let Ok(p) = Patch::parse(&bytes, &mut c.scratch) else {
            return;
        };
        if c.scene.try_apply(&p, &mut c.journal).is_err() {
            return;
        }
        c.scene.commit(&mut c.journal);
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: self.seq,
        };
        wolkit::window::pipeline::ensure_layout(c);
        focus::sync(c, &mut s, &mut self.st);
        edit::focus_changed(c, &mut self.st);
        edit::refresh(c, &mut self.st);
    }

    fn sink(&mut self) -> (&mut WindowCore, Sink<'_>, &mut InputState) {
        (
            &mut self.core,
            Sink {
                events: &mut self.events,
                window: 1,
                seq: 0,
            },
            &mut self.st,
        )
    }

    fn drain(&mut self) {
        let mut buf = vec![0u8; 1 << 16];
        while self.events.read(&mut buf) == buf.len() {}
    }
}

fn px(v: f32) -> [u32; 2] {
    [DIM_PX, v.to_bits()]
}

const KEYS: &[(u32, u32)] = &[
    (sym::LEFT, 0),
    (sym::RIGHT, 0),
    (sym::UP, 0),
    (sym::DOWN, 0),
    (sym::HOME, 0),
    (sym::END, 0),
    (sym::PAGE_UP, 0),
    (sym::PAGE_DOWN, 0),
    (sym::LEFT, MOD_SHIFT),
    (sym::UP, MOD_SHIFT),
    (sym::DOWN, MOD_SHIFT),
    (sym::END, MOD_SHIFT),
    (sym::HOME, MOD_CTRL),
    (sym::END, MOD_CTRL),
    (sym::LEFT, MOD_CTRL),
    (sym::RIGHT, MOD_CTRL | MOD_SHIFT),
    (sym::RETURN, 0),
    (sym::RETURN, MOD_CTRL),
    (sym::BACKSPACE, 0),
    (sym::DELETE, 0),
    (sym::BACKSPACE, MOD_CTRL),
    (sym::A, MOD_CTRL),
    (sym::Z, MOD_CTRL),
    (sym::Z_UPPER, MOD_CTRL | MOD_SHIFT),
    (sym::X, MOD_CTRL),
    (sym::TAB, 0),
];

/// The window-level invariants after every step.
fn check_rig(r: &mut Rig, ctx: &str) {
    let Some(v) = r.core.scene.edit_view(1) else {
        return;
    };
    assert!(v.scroll.0.is_finite() && v.scroll.1.is_finite());
    assert!(
        v.scroll.0 >= 0.0 && v.scroll.1 >= 0.0,
        "{ctx}: negative scroll {:?}",
        v.scroll
    );
    if !v.active {
        return;
    }
    let t = &v.text;
    assert!(
        v.caret <= t.len() && t.is_char_boundary(v.caret),
        "{ctx}: view caret"
    );
    assert!(
        v.selection.end <= t.len()
            && v.selection.start <= v.selection.end
            && t.is_char_boundary(v.selection.start)
            && t.is_char_boundary(v.selection.end),
        "{ctx}: view selection"
    );
    if let Some(p) = &v.preedit {
        assert!(p.end <= t.len() && t.is_char_boundary(p.start) && t.is_char_boundary(p.end));
    }
    // The editor and the node agree once the input layer has caught up with the app.
    if v.base == r.core.scene.text(1) {
        let ed = &r.st.field.editor;
        let e = ed.selection();
        assert!(
            e.head <= ed.len() && e.anchor <= ed.len(),
            "{ctx}: editor selection"
        );
        let node = r.core.scene.text(1);
        assert!(
            node == ed.text() || ed.normalised(node) == ed.text(),
            "{ctx}: node text and editor text differ"
        );
    }
}

fn run_engine(by: &mut Bytes<'_>, flags: u8) -> Stats {
    let mut st = Stats::default();
    let (area, secure) = (flags & 1 == 1, flags & 2 == 2);
    let w = 60.0 + f32::from(by.next().unwrap_or(0) % 120);
    let h = 24.0 + f32::from(by.next().unwrap_or(0) % 100);
    let mut core = WindowCore::new(1, "Fuzz", 240, 200, 0, Environment::default());
    core.scale = 1.0;
    let mut r = Rig {
        core,
        events: EventQueue::new(),
        st: InputState::new(),
        seq: 0,
    };
    r.apply(|p| {
        p.create(1, Kind::TextInput).insert(0, 1, APPEND);
        p.set_prop(1, id::POSITION, &[1]);
        p.set_prop(1, id::INSET_LEFT, &px(10.0));
        p.set_prop(1, id::INSET_TOP, &px(10.0));
        p.set_prop(1, id::WIDTH, &px(w));
        p.set_prop(1, id::HEIGHT, &px(h));
        if area {
            p.set_prop(1, id::WHITE_SPACE, &[2]);
        }
        if secure {
            p.set_prop(1, id::SECURE, &[1]);
        }
        p.create(2, Kind::Button).insert(0, 2, APPEND);
        p.set_prop(2, id::POSITION, &[1]);
        p.set_prop(2, id::INSET_LEFT, &px(10.0));
        p.set_prop(2, id::INSET_TOP, &px(150.0));
        p.set_prop(2, id::WIDTH, &px(40.0));
        p.set_prop(2, id::HEIGHT, &px(20.0));
        p.focus(1);
    });
    let (mut mx, mut my) = (20.0f32, 20.0f32);
    while st.ops < 600 {
        let Some(op) = by.next() else {
            break;
        };
        let arg = by.next().unwrap_or(0);
        st.ops += 1;
        match op % 14 {
            0..=3 => {
                // Typing.
                let ch = PIECES[usize::from(arg) % PIECES.len()];
                let (c, mut s, ist) = r.sink();
                edit::type_text(c, &mut s, ist, ch);
            }
            4..=6 => {
                let (k, m) = KEYS[usize::from(arg) % KEYS.len()];
                r.sink_key(k, m);
            }
            7 => {
                let (c, mut s, ist) = r.sink();
                edit::paste(c, &mut s, ist, PIECES[usize::from(arg) % PIECES.len()]);
            }
            8..=9 => {
                // The pointer: press, drag, release, with or without Shift.
                mx = f32::from(arg) % 200.0;
                my = f32::from(by.next().unwrap_or(0)) % 170.0;
                let mods = if arg & 0x80 != 0 { MOD_SHIFT } else { 0 };
                r.sink_ptr(POINTER_DOWN, mx, my, mods);
                r.sink_ptr(POINTER_MOVE, 200.0 - mx, 170.0 - my, 0);
                r.sink_ptr(POINTER_UP, 200.0 - mx, 170.0 - my, 0);
            }
            10 => {
                r.sink_ptr(POINTER_MOVE, mx, my, 0);
                let dy = (f32::from(arg) - 100.0) * 2.0;
                r.sink_ptr(POINTER_SCROLL, 0.0, dy, 0);
            }
            11 => {
                // The app replaces the text, with whatever selection the person has.
                let t = PIECES[usize::from(arg) % PIECES.len()].repeat(usize::from(arg >> 5) + 1);
                r.apply(|p| {
                    p.set_text(1, &t);
                });
            }
            12 => {
                // Focus leaves and comes back: the scroll must not outlive the text.
                r.sink_ptr(POINTER_DOWN, 20.0, 160.0, 0);
                r.sink_ptr(POINTER_UP, 20.0, 160.0, 0);
                if arg & 1 == 1 {
                    r.apply(|p| {
                        p.set_text(1, "");
                    });
                }
                r.sink_ptr(POINTER_DOWN, 20.0, 20.0, 0);
                r.sink_ptr(POINTER_UP, 20.0, 20.0, 0);
            }
            _ => {
                // A frame: paint must cope with whatever state the field is in.
                let (cw, ch) = (r.core.width, r.core.height);
                let mut out = vec![0u8; (cw * ch * 4) as usize];
                wolkit::window::pipeline::render_rgba(&mut r.core, cw, ch, 1.0, &mut out);
                st.geometry += 1;
            }
        }
        r.drain();
        check_rig(&mut r, &format!("op {} ({op})", st.ops));
        st.edits += 1;
    }
    st
}

impl Rig {
    fn sink_key(&mut self, keysym: u32, mods: u32) {
        let (c, mut s, ist) = self.sink();
        let k = KeyPress {
            keysym,
            pressed: true,
            mods,
            repeat: false,
            text: "",
        };
        keys::key(c, &mut s, ist, k);
    }

    fn sink_ptr(&mut self, kind: u32, x: f32, y: f32, mods: u32) {
        let (c, mut s, ist) = self.sink();
        let p = PointerInput {
            kind,
            x,
            y,
            button: BUTTON_LEFT,
            mods,
        };
        let _ = pointer::pointer(c, &mut s, ist, p);
    }
}
