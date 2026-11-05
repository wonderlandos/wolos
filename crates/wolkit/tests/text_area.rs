// SPDX-License-Identifier: BSD-2-Clause
//! The multi-line text field (E2E-WK-TEXT): the geometry of a laid-out text (`text::area`: caret
//! position, Up and Down with a sticky column, line ends, hit tests at wrapped line ends, scroll
//! to caret, word and paragraph at a point), the editor on multi-line text against a reference
//! model, and the `text_input` with `white-space: pre-wrap` driven through the engine's input
//! routing: Enter, vertical movement, clicks, drags, the wheel, paste, undo, and what paint
//! draws (`tests/text_area_golden/*.png`, bit for bit on the bundled fonts; WOLKIT_BLESS=1 writes
//! them).

#[path = "golden/png.rs"]
mod png;
#[path = "support/text_fuzz.rs"]
mod text_fuzz;
#[path = "support/text.rs"]
mod text_support;

use std::sync::{Arc, Mutex};

use text_support::fonts;
use wolkit::env::Environment;
use wolkit::event::{self, EventQueue, kind as ev};
use wolkit::input::keys::{KeyPress, sym};
use wolkit::input::{BUTTON_LEFT, InputState, edit, focus, keys, pointer};
use wolkit::scene::patch::{APPEND, Patch, Writer};
use wolkit::scene::prop::{DIM_PX, id};
use wolkit::scene::{Kind, SceneRead};
use wolkit::style::{Table, compile_sources};
use wolkit::text::area;
use wolkit::text::cache::ShapeCache;
use wolkit::text::edit::Editor;
use wolkit::text::layout::{LayoutOptions, TextLayout, TextStyle, layout_text};
use wolkit::text::system::TextSystem;
use wolkit::window::{
    MOD_CTRL, MOD_SHIFT, POINTER_DOWN, POINTER_MOVE, POINTER_SCROLL, POINTER_UP, PointerInput,
    Sink, WindowCore,
};

// ---- geometry --------------------------------------------------------------------------------

fn lay(text: &str, width: f32) -> TextLayout {
    let f = fonts();
    let mut c = ShapeCache::new(64);
    let st = TextStyle::new(f.latin, 16.0);
    let opts = LayoutOptions {
        max_width: width,
        ..LayoutOptions::default()
    };
    layout_text(&f.db, &mut c, text, &st, &opts)
}

const WRAP: &str = "The quick brown fox jumps over the lazy dog";

#[test]
fn the_caret_follows_lines_wraps_and_empty_paragraphs() {
    let text = "ab\n\ncd";
    let l = lay(text, f32::INFINITY);
    assert_eq!(l.lines.len(), 3);
    let tops: Vec<f32> = (0..=text.len())
        .filter(|&i| text.is_char_boundary(i))
        .map(|i| area::caret_pos(&l, text, i).line)
        .map(|li| l.lines[li].top)
        .collect();
    assert_eq!(
        tops,
        vec![
            l.lines[0].top,
            l.lines[0].top,
            l.lines[0].top, // "ab|" is still line 0
            l.lines[1].top, // the empty paragraph
            l.lines[2].top,
            l.lines[2].top,
            l.lines[2].top
        ]
    );
    // After the final newline there is one more, empty, line to put the caret on.
    let t2 = "ab\n";
    let l2 = lay(t2, f32::INFINITY);
    assert_eq!(l2.lines.len(), 2);
    assert_eq!(area::caret_pos(&l2, t2, 3).line, 1);
    // A wrapped line: the offset where it wraps is drawn at the start of the next line.
    let w = lay(WRAP, 80.0);
    assert!(w.lines.len() >= 3);
    let at = w.lines[1].range.start;
    assert_eq!(area::caret_pos(&w, WRAP, at).line, 1);
    assert!(area::caret_pos(&w, WRAP, at).x <= w.lines[1].x + 0.01);
}

#[test]
fn up_and_down_keep_their_column_over_short_lines() {
    let text = "hello world\nhi\nhello world";
    let l = lay(text, f32::INFINITY);
    let from = 8; // "hello wo|rld"
    let x0 = area::caret_pos(&l, text, from).x;
    let (a, goal) = area::move_vertical(&l, text, from, None, 1);
    assert_eq!(
        a, 14,
        "the short line has only two characters: the caret goes to its end"
    );
    assert!((goal - x0).abs() < 0.01, "the goal is where the run began");
    let (b, goal2) = area::move_vertical(&l, text, a, Some(goal), 1);
    assert_eq!(goal2, goal);
    assert_eq!(
        b,
        15 + 8,
        "back on a long line the caret returns to the original column"
    );
    let (up, _) = area::move_vertical(&l, text, b, Some(goal), -2);
    assert_eq!(up, from);
    // Without the goal the column would have been lost on the short line.
    let (c, _) = area::move_vertical(&l, text, a, None, 1);
    assert!(
        (16..=17).contains(&c) && c < b,
        "the column of the short line, not the goal: {c}"
    );
    // Past the first and last line the caret goes to the ends of the text.
    assert_eq!(area::move_vertical(&l, text, 3, None, -1).0, 0);
    assert_eq!(area::move_vertical(&l, text, 20, None, 1).0, text.len());
}

#[test]
fn line_ends_are_the_ends_on_screen_and_a_wrap_keeps_its_space_out() {
    let l = lay(WRAP, 80.0);
    for (i, line) in l.lines.iter().enumerate() {
        let mid = line.range.start + (line.range.end - line.range.start) / 2;
        let (s, e) = area::line_edges(&l, WRAP, mid);
        assert_eq!(s, line.range.start);
        if i + 1 < l.lines.len() {
            assert!(
                e < line.range.end && WRAP[e..line.range.end].chars().all(char::is_whitespace),
                "End stops before the space that wrapped: {:?}",
                &WRAP[s..e]
            );
            assert_eq!(
                area::caret_pos(&l, WRAP, e).line,
                i,
                "and stays on the line"
            );
        } else {
            assert_eq!(e, line.range.end);
        }
    }
    // A hard break: End is just before the newline.
    let t = "ab\ncd";
    let l = lay(t, f32::INFINITY);
    assert_eq!(area::line_edges(&l, t, 1), (0, 2));
    assert_eq!(area::line_edges(&l, t, 3), (3, 5));
}

#[test]
fn a_click_past_a_wrapped_line_lands_on_that_line() {
    let l = lay(WRAP, 80.0);
    let y = l.lines[0].top + l.lines[0].height / 2.0;
    let at = area::hit(&l, WRAP, 10_000.0, y);
    assert_eq!(area::caret_pos(&l, WRAP, at).line, 0, "offset {at}");
    // The plain hit test would have chosen the start of the next line.
    assert_eq!(l.hit_test(WRAP, 10_000.0, y), l.lines[0].range.end);
    // Below the text: the end; above it: the start.
    assert_eq!(
        area::hit(&l, WRAP, 5.0, 10_000.0),
        area::hit(&l, WRAP, 5.0, l.lines.last().unwrap().top + 1.0)
    );
    assert_eq!(area::hit(&l, WRAP, 0.0, -50.0), 0);
}

#[test]
fn the_scroll_keeps_the_caret_in_view_and_never_past_the_text() {
    let c = |top: f32| area::CaretPos {
        line: 0,
        x: 0.0,
        top,
        height: 20.0,
    };
    let view = (100.0, 60.0);
    assert_eq!(
        area::scroll_to_caret((0.0, 0.0), view, &c(10.0), true),
        (0.0, 0.0)
    );
    assert_eq!(
        area::scroll_to_caret((0.0, 0.0), view, &c(80.0), true),
        (0.0, 40.0)
    );
    assert_eq!(
        area::scroll_to_caret((0.0, 90.0), view, &c(10.0), true),
        (0.0, 10.0)
    );
    // A one-line field scrolls sideways only.
    let x = area::CaretPos {
        line: 0,
        x: 250.0,
        top: 0.0,
        height: 20.0,
    };
    assert_eq!(
        area::scroll_to_caret((0.0, 0.0), view, &x, false),
        (151.0, 0.0)
    );
    assert_eq!(
        area::clamp_scroll((0.0, 500.0), (100.0, 200.0), view),
        (0.0, 140.0)
    );
    assert_eq!(
        area::clamp_scroll((9.0, -5.0), (50.0, 20.0), view),
        (0.0, 0.0)
    );
}

#[test]
fn word_and_paragraph_at_a_point() {
    let t = "one two,  three\nsecond line\r\nlast";
    assert_eq!(&t[area::word_at(t, 5)], "two");
    assert_eq!(&t[area::word_at(t, 3)], " ", "a space is its own stop");
    assert_eq!(&t[area::word_at(t, 0)], "one");
    assert_eq!(area::word_at("", 0), 0..0);
    assert_eq!(&t[area::paragraph_at(t, 2)], "one two,  three");
    assert_eq!(&t[area::paragraph_at(t, 20)], "second line");
    assert_eq!(&t[area::paragraph_at(t, t.len())], "last");
    assert_eq!(
        &t[area::paragraph_at(t, 15)],
        "one two,  three",
        "the break belongs to the line it ends"
    );
}

// ---- the editor on multi-line text, against a model -------------------------------------------

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

const LINES: &[&str] = &[
    "a",
    "b c",
    "\n",
    "\n\n",
    "x\ny",
    "é",
    "日本\n",
    "🧑‍🚀",
    "\r\n",
    "\r",
    "\u{2029}",
    "tab\there",
    " ",
];

/// What the model says `s` becomes: every break is one `\n`.
fn norm(s: &str) -> String {
    s.replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{2029}', '\u{85}'], "\n")
}

#[test]
fn multi_line_editing_matches_a_string_model_and_undoes_exactly() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut ed = Editor::new(false);
        let mut model;
        let mut states = vec![String::new()];
        for step in 0..120 {
            let before = ed.text();
            let sel = ed.selection();
            match rng.below(9) {
                0..=3 => {
                    let piece = LINES[rng.below(LINES.len())];
                    ed.insert(piece);
                    let mut m = before.clone();
                    m.replace_range(sel.range(), &norm(piece));
                    model = m;
                }
                4 => {
                    ed.paste("p\r\nq\r\nr");
                    let mut m = before.clone();
                    m.replace_range(sel.range(), "p\nq\nr");
                    model = m;
                }
                5 => {
                    ed.backspace();
                    model = ed.text();
                }
                6 => {
                    let n = ed.len();
                    ed.set_selection(rng.below(n + 1), rng.below(n + 1));
                    model = before.clone();
                }
                7 => {
                    ed.move_to(rng.below(ed.len() + 1), rng.below(2) == 0);
                    model = before.clone();
                }
                _ => {
                    ed.delete_forward();
                    model = ed.text();
                }
            }
            assert_eq!(ed.text(), model, "seed {seed} step {step}");
            assert!(!ed.text().contains('\r'), "no carriage return survives");
            assert!(ed.text().is_char_boundary(ed.caret()));
            if ed.text() != before {
                states.push(ed.text());
            }
        }
        // Undo walks back through texts that were all seen on the way up (steps merge, so not
        // every state is a stop), ends at the empty text, and redo returns to the final text.
        let last = ed.text();
        let mut undone = 0;
        while ed.undo() {
            undone += 1;
            assert!(
                states.contains(&ed.text()),
                "seed {seed}: undo reached an unseen text"
            );
        }
        assert!(undone > 0 || last.is_empty());
        assert_eq!(ed.text(), "", "seed {seed}");
        while ed.redo() {}
        assert_eq!(ed.text(), last, "seed {seed}");
    }
}

#[test]
fn twenty_typed_words_are_twenty_undo_steps_and_a_paste_is_one() {
    let mut ed = Editor::new(false);
    for i in 0..20 {
        ed.insert(&format!("w{i}"));
        ed.insert(if i % 5 == 4 { "\n" } else { " " });
    }
    let full = ed.text();
    let mut steps = 0;
    while ed.undo() {
        steps += 1;
    }
    assert_eq!(
        steps, 20,
        "one step per word, the space or break that ends it included"
    );
    assert_eq!(ed.text(), "");
    while ed.redo() {}
    assert_eq!(ed.text(), full);
    ed.paste("l1\nl2\nl3\nl4");
    assert!(ed.undo());
    assert_eq!(ed.text(), full, "a multi-line paste is one step");
}

// ---- the engine: a text_input with white-space: pre-wrap ---------------------------------------

const SHEET: &str = "
.area { background: control; border: 1px line; border-radius: s; padding: sm; white-space: pre-wrap; }
.line { background: control; border: 1px line; border-radius: s; padding: sm; }
";

struct Win {
    core: WindowCore,
    events: EventQueue,
    st: InputState,
    seq: u32,
}

type Ev = (u16, u32, Vec<u8>);

fn px(v: f32) -> [u32; 2] {
    [DIM_PX, v.to_bits()]
}

impl Win {
    fn new() -> Win {
        let mut core = WindowCore::new(1, "Test", 360, 240, 0, Environment::default());
        // The bundled fonts, not whatever the machine has: the pixels are the golden's.
        core.text = Arc::new(Mutex::new(TextSystem::from_db(fonts().db)));
        let c = compile_sources(&[("t.wolcss", SHEET)])
            .unwrap_or_else(|d| panic!("{}", wolkit::style::render_all(&d)));
        core.style = Some(Arc::new(Table::new(c.table)));
        core.style_version = 1;
        Win {
            core,
            events: EventQueue::new(),
            st: InputState::new(),
            seq: 0,
        }
    }

    fn class(&self, name: &str) -> u32 {
        self.core
            .style
            .as_ref()
            .unwrap()
            .table
            .class_id(name)
            .unwrap()
    }

    fn apply(&mut self, f: impl FnOnce(&mut Writer)) {
        let mut w = Writer::new();
        f(&mut w);
        self.seq += 1;
        let bytes = w.finish(self.seq).to_vec();
        let c = &mut self.core;
        let p = Patch::parse(&bytes, &mut c.scratch).unwrap();
        c.scene.try_apply(&p, &mut c.journal).unwrap();
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

    /// A patch that reaches the scene and nothing else: the input layer has not heard of it yet.
    fn apply_raw(&mut self, f: impl FnOnce(&mut Writer)) {
        let mut w = Writer::new();
        f(&mut w);
        self.seq += 1;
        let bytes = w.finish(self.seq).to_vec();
        let c = &mut self.core;
        let p = Patch::parse(&bytes, &mut c.scratch).unwrap();
        c.scene.try_apply(&p, &mut c.journal).unwrap();
        c.scene.commit(&mut c.journal);
        wolkit::window::pipeline::ensure_layout(c);
    }

    /// A text area (node 1) or, with `area` false, a one-line field, at (10, 10), `w` by `h`.
    fn field(&mut self, area: bool, w: f32, h: f32) {
        let class = self.class(if area { "area" } else { "line" });
        self.apply(|p| {
            p.create(1, Kind::TextInput).insert(0, 1, APPEND);
            p.set_style(1, &[class]);
            p.set_prop(1, id::POSITION, &[1]);
            p.set_prop(1, id::INSET_LEFT, &px(10.0));
            p.set_prop(1, id::INSET_TOP, &px(10.0));
            p.set_prop(1, id::WIDTH, &px(w));
            p.set_prop(1, id::HEIGHT, &px(h));
            p.bind_event(
                1,
                event::bit(ev::TEXT_CHANGED) | event::bit(ev::TEXT_SUBMIT),
            );
            p.focus(1);
        });
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

    fn key(&mut self, keysym: u32, mods: u32, text: &str) {
        let (c, mut s, st) = self.sink();
        let k = KeyPress {
            keysym,
            pressed: true,
            mods,
            repeat: false,
            text,
        };
        keys::key(c, &mut s, st, k);
    }

    fn type_str(&mut self, t: &str) {
        for ch in t.chars() {
            if ch == '\n' {
                self.key(sym::RETURN, 0, "");
            } else {
                let mut b = [0u8; 4];
                let s = ch.encode_utf8(&mut b);
                self.key(u32::from(ch), 0, s);
            }
        }
    }

    fn ptr(&mut self, kind: u32, x: f32, y: f32, mods: u32) {
        let (c, mut s, st) = self.sink();
        let p = PointerInput {
            kind,
            x,
            y,
            button: BUTTON_LEFT,
            mods,
        };
        pointer::pointer(c, &mut s, st, p).unwrap();
    }

    fn click(&mut self, x: f32, y: f32) {
        self.ptr(POINTER_DOWN, x, y, 0);
        self.ptr(POINTER_UP, x, y, 0);
    }

    fn text(&self) -> String {
        self.core.scene.text(1).to_owned()
    }

    fn sel(&self) -> (usize, usize) {
        let s = self.st.field.editor.selection();
        (s.anchor, s.head)
    }

    fn events(&mut self) -> Vec<Ev> {
        let mut buf = vec![0u8; 1 << 20];
        let n = self.events.read(&mut buf);
        event::parse(&buf[..n])
            .into_iter()
            .map(|(h, p)| (h.kind, h.node, p.to_vec()))
            .collect()
    }

    /// The caret's window position (what the input method is told).
    fn caret_xy(&self) -> (f32, f32) {
        let r = edit::cursor_rect(&self.core, &self.st).unwrap();
        (r.x, r.y)
    }

    fn pixels(&mut self) -> Vec<u8> {
        let (w, h) = (self.core.width, self.core.height);
        let mut out = vec![0u8; (w * h * 4) as usize];
        wolkit::window::pipeline::render_rgba(&mut self.core, w, h, 1.0, &mut out);
        out
    }
}

const THREE: &str = "The first paragraph is long enough to wrap onto a second line of the field.\n\
Short one.\n\
A third paragraph, also long, so that the whole text runs past the bottom of the box and scrolls.";

#[test]
fn enter_breaks_the_line_in_an_area_and_submits_in_a_line_field() {
    let mut w = Win::new();
    w.field(true, 300.0, 120.0);
    w.events();
    w.type_str("ab");
    w.key(sym::RETURN, 0, "");
    w.type_str("cd");
    assert_eq!(w.text(), "ab\ncd");
    let evs = w.events();
    assert!(
        evs.iter().all(|e| e.0 == ev::TEXT_CHANGED),
        "no submit: {evs:?}"
    );
    // The event carries caret, anchor, length and the text, as for a one-line field.
    let last = evs.last().unwrap();
    assert_eq!(u32::from_le_bytes(last.2[0..4].try_into().unwrap()), 5);
    assert_eq!(&last.2[12..17], b"ab\ncd");
    // Ctrl+Enter is the submit.
    w.key(sym::RETURN, MOD_CTRL, "");
    let evs = w.events();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].0, ev::TEXT_SUBMIT);
    assert_eq!(w.text(), "ab\ncd");

    let mut one = Win::new();
    one.field(false, 200.0, 32.0);
    one.events();
    one.type_str("ab");
    one.key(sym::RETURN, 0, "");
    assert_eq!(one.text(), "ab");
    assert_eq!(one.events().last().unwrap().0, ev::TEXT_SUBMIT);
}

#[test]
fn up_and_down_move_between_lines_and_remember_the_column() {
    let mut w = Win::new();
    w.field(true, 300.0, 140.0);
    w.type_str("hello world\nhi\nhello world");
    assert_eq!(w.sel(), (26, 26));
    w.key(sym::LEFT, 0, "");
    w.key(sym::LEFT, 0, "");
    w.key(sym::LEFT, 0, "");
    assert_eq!(w.sel().1, 23);
    w.key(sym::UP, 0, "");
    assert_eq!(w.sel().1, 14, "the short line: its end");
    w.key(sym::UP, 0, "");
    assert_eq!(
        w.sel().1,
        8,
        "back at the first line, at the column the run began at"
    );
    w.key(sym::DOWN, 0, "");
    w.key(sym::DOWN, 0, "");
    assert_eq!(w.sel().1, 23);
    // Any other key forgets the column.
    w.key(sym::UP, 0, "");
    w.key(sym::LEFT, 0, "");
    w.key(sym::DOWN, 0, "");
    assert_eq!(
        w.sel().1,
        16,
        "from column 1 on the short line: after the h"
    );
    // Up on the first line goes to the start, Down on the last to the end; Shift extends.
    w.key(sym::UP, 0, "");
    w.key(sym::UP, 0, "");
    assert_eq!(w.sel().1, 1);
    w.key(sym::UP, MOD_SHIFT, "");
    assert_eq!(w.sel(), (1, 0));
    w.key(sym::END, MOD_CTRL, "");
    assert_eq!(w.sel(), (26, 26));
    w.key(sym::HOME, 0, "");
    assert_eq!(
        w.sel().1,
        15,
        "Home: the start of this line, not of the text"
    );
    w.key(sym::HOME, MOD_CTRL, "");
    assert_eq!(w.sel().1, 0);
    w.key(sym::END, 0, "");
    assert_eq!(w.sel().1, 11, "End: the end of this line");
    w.key(sym::DOWN, MOD_SHIFT, "");
    w.key(sym::DOWN, MOD_SHIFT, "");
    w.key(sym::DOWN, MOD_SHIFT, "");
    assert_eq!(w.sel(), (11, 26));
}

#[test]
fn home_and_end_follow_the_wrap_and_the_page_keys_move_a_page() {
    let mut w = Win::new();
    w.field(true, 220.0, 100.0);
    w.type_str(&"word ".repeat(40));
    assert!(
        w.core.scene.edit_view(1).unwrap().scroll.1 > 0.0,
        "typing scrolled to the caret"
    );
    let end = w.text().len();
    w.key(sym::HOME, 0, "");
    let line_start = w.sel().1;
    assert!(
        line_start > 0 && line_start < end,
        "Home is the start of the last wrapped line"
    );
    w.key(sym::PAGE_UP, 0, "");
    let up = w.sel().1;
    assert!(up < line_start);
    w.key(sym::PAGE_DOWN, 0, "");
    assert!(w.sel().1 > up);
    w.key(sym::HOME, MOD_CTRL, "");
    assert_eq!(
        w.core.scene.edit_view(1).unwrap().scroll.1,
        0.0,
        "back at the top"
    );
    w.key(sym::END, 0, "");
    let first_end = w.sel().1;
    assert!(first_end < w.text()[..first_end + 1].len() && first_end > 0 && first_end < 30);
    assert!(
        w.text().is_char_boundary(first_end) && w.text()[..first_end].ends_with("word"),
        "End stops before the space the wrap swallowed: {:?}",
        &w.text()[..first_end]
    );
}

#[test]
fn a_click_puts_the_caret_a_drag_selects_and_more_clicks_select_more() {
    let mut w = Win::new();
    w.field(true, 300.0, 140.0);
    w.type_str("alpha beta\ngamma delta");
    // Find where the caret lands for known offsets by asking the engine, then click there.
    w.key(sym::HOME, MOD_CTRL, "");
    let (x0, y0) = w.caret_xy();
    w.key(sym::DOWN, 0, "");
    let (_, y1) = w.caret_xy();
    assert!(y1 > y0, "the second line is lower");
    w.click(x0 + 2.0, y1 + 4.0);
    assert_eq!(
        w.sel(),
        (11, 11),
        "a click on the left of line 2 is its start"
    );
    w.click(300.0, y0 + 4.0);
    assert_eq!(
        w.sel(),
        (10, 10),
        "a click past the end of line 1 is its end"
    );
    // A drag from line 1 to line 2.
    std::thread::sleep(std::time::Duration::from_millis(520));
    w.ptr(POINTER_DOWN, x0 + 2.0, y0 + 4.0, 0);
    w.ptr(POINTER_MOVE, 300.0, y1 + 4.0, 0);
    w.ptr(POINTER_UP, 300.0, y1 + 4.0, 0);
    assert_eq!(
        w.sel(),
        (0, 22),
        "the selection runs from the press to the pointer"
    );
    assert_eq!(w.st.field.editor.selected_text(), "alpha beta\ngamma delta");
    // Shift-click extends from the anchor.
    std::thread::sleep(std::time::Duration::from_millis(520));
    w.click(x0 + 2.0, y0 + 4.0);
    w.ptr(POINTER_DOWN, 300.0, y0 + 4.0, MOD_SHIFT);
    w.ptr(POINTER_UP, 300.0, y0 + 4.0, MOD_SHIFT);
    assert_eq!(w.sel(), (0, 10));
    // Double click: a word. Triple: the paragraph.
    std::thread::sleep(std::time::Duration::from_millis(520));
    let (xw, _) = {
        w.key(sym::HOME, MOD_CTRL, "");
        for _ in 0..7 {
            w.key(sym::RIGHT, 0, "");
        }
        w.caret_xy()
    };
    w.click(xw, y0 + 4.0);
    w.click(xw, y0 + 4.0);
    assert_eq!(w.st.field.editor.selected_text(), "beta");
    w.click(xw, y0 + 4.0);
    assert_eq!(w.st.field.editor.selected_text(), "alpha beta");
}

#[test]
fn the_wheel_scrolls_a_text_area_that_overflows_and_hands_the_rest_on() {
    let mut w = Win::new();
    w.field(true, 220.0, 80.0);
    w.type_str(&"line\n".repeat(12));
    w.key(sym::HOME, MOD_CTRL, "");
    let scroll = |w: &Win| w.core.scene.edit_view(1).map_or(0.0, |v| v.scroll.1);
    assert_eq!(scroll(&w), 0.0);
    w.ptr(POINTER_MOVE, 100.0, 40.0, 0);
    w.ptr(POINTER_SCROLL, 0.0, 30.0, 0);
    assert_eq!(scroll(&w), 30.0);
    w.ptr(POINTER_SCROLL, 0.0, -100.0, 0);
    assert_eq!(scroll(&w), 0.0, "never above the first line");
    w.ptr(POINTER_SCROLL, 0.0, 10_000.0, 0);
    let max = scroll(&w);
    assert!(
        max > 100.0 && max < 400.0,
        "stops at the end of the text: {max}"
    );
    // The caret scrolls back into view when the person types.
    w.type_str("x");
    assert!(scroll(&w) < max || w.sel().1 > 0);
}

#[test]
fn a_one_line_field_scrolls_sideways_to_the_caret_and_keeps_its_keys() {
    let mut w = Win::new();
    w.field(false, 120.0, 32.0);
    w.type_str("a long line that is wider than the field");
    let v = w.core.scene.edit_view(1).unwrap();
    assert!(
        v.scroll.0 > 20.0 && v.scroll.1 == 0.0,
        "scrolled sideways: {:?}",
        v.scroll
    );
    w.key(sym::HOME, 0, "");
    assert_eq!(w.core.scene.edit_view(1).unwrap().scroll.0, 0.0);
    // Up and Down are not the field's: they are not consumed.
    w.key(sym::DOWN, 0, "");
    assert_eq!(w.sel().1, 0);
}

#[test]
fn paste_of_several_lines_is_one_undo_step_and_undo_walks_the_words() {
    let mut w = Win::new();
    w.field(true, 300.0, 140.0);
    w.type_str("one two");
    {
        let (c, mut s, st) = w.sink();
        assert!(edit::paste(c, &mut s, st, "l1\r\nl2\rl3"));
    }
    assert_eq!(w.text(), "one twol1\nl2\nl3");
    w.key(u32::from('z'), MOD_CTRL, "z");
    assert_eq!(w.text(), "one two", "the paste was one step");
    w.key(u32::from('z'), MOD_CTRL, "z");
    assert_eq!(w.text(), "one ", "then the last word");
    w.key(u32::from('z'), MOD_CTRL, "z");
    assert_eq!(w.text(), "");
    w.key(u32::from('z'), MOD_CTRL | MOD_SHIFT, "Z");
    assert_eq!(w.text(), "one ");
}

#[test]
fn the_app_replacing_the_text_wins_over_what_paint_was_drawing() {
    let mut w = Win::new();
    w.field(true, 300.0, 100.0);
    w.type_str("typed");
    assert_eq!(w.core.scene.edit_view(1).unwrap().text, "typed");
    // The app clears the field (as after a send): paint must not keep the old text.
    w.apply(|p| {
        p.set_text(1, "");
    });
    assert_eq!(w.text(), "");
    let v = w.core.scene.edit_view(1).unwrap();
    assert!(v.text.is_empty() && v.caret == 0, "{v:?}");
    w.type_str("ab");
    assert_eq!(w.text(), "ab");
}

/// A text area (node 1) with a placeholder and `text`, and a button (node 2) that has the focus.
fn chat_box(text: &str) -> Win {
    let mut w = Win::new();
    w.field(true, 300.0, 80.0);
    w.apply(|p| {
        let ph = p.string("Say something");
        p.set_prop(1, id::PLACEHOLDER, &[ph]);
        p.set_text(1, text);
        p.create(2, Kind::Button).insert(0, 2, APPEND);
        p.set_prop(2, id::POSITION, &[1]);
        p.set_prop(2, id::INSET_LEFT, &px(10.0));
        p.set_prop(2, id::INSET_TOP, &px(150.0));
        p.set_prop(2, id::WIDTH, &px(60.0));
        p.set_prop(2, id::HEIGHT, &px(24.0));
    });
    w.click(20.0, 160.0);
    w
}

#[test]
fn the_scroll_of_a_blurred_field_does_not_outlive_its_text() {
    let long = "line\n".repeat(14);
    let mut a = chat_box(&long);
    a.ptr(POINTER_MOVE, 100.0, 40.0, 0);
    a.ptr(POINTER_SCROLL, 0.0, 10_000.0, 0);
    assert!(
        a.core.scene.edit_view(1).unwrap().scroll.1 > 0.0,
        "the unfocused field scrolled"
    );
    assert_ne!(a.pixels(), chat_box(&long).pixels(), "and it shows");
    // The chat box clears after Send: the placeholder is back at the top, not a blank page.
    a.apply(|p| {
        p.set_text(1, "");
    });
    assert_eq!(a.pixels(), chat_box("").pixels());
    // Other text of any length starts at the top too.
    let mut b = chat_box(&long);
    b.ptr(POINTER_MOVE, 100.0, 40.0, 0);
    b.ptr(POINTER_SCROLL, 0.0, 10_000.0, 0);
    let other = "other\n".repeat(14);
    b.apply(|p| {
        p.set_text(1, &other);
    });
    assert_eq!(b.pixels(), chat_box(&other).pixels());
}

#[test]
fn a_focused_scrolled_field_whose_text_the_app_replaced_is_laid_out_fresh() {
    let mut w = Win::new();
    w.field(true, 300.0, 80.0);
    w.type_str(&"line\n".repeat(14));
    assert!(w.core.scene.edit_view(1).unwrap().scroll.1 > 0.0);
    // The wheel arrives before anything told the input layer about the patch.
    w.apply_raw(|p| {
        p.set_text(1, "short");
    });
    w.ptr(POINTER_MOVE, 100.0, 40.0, 0);
    w.ptr(POINTER_SCROLL, 0.0, 30.0, 0);
    let v = w.core.scene.edit_view(1).unwrap();
    assert_eq!((v.text.as_str(), v.scroll.1), ("short", 0.0), "{v:?}");
}

#[test]
fn up_and_down_in_a_secure_area_follow_the_bullets_that_are_painted() {
    let mut w = Win::new();
    w.field(true, 120.0, 140.0);
    w.apply(|p| {
        p.set_prop(1, id::SECURE, &[1]);
    });
    w.type_str(&"WM".repeat(30));
    assert_eq!(w.sel(), (60, 60));
    let (x_end, y_end) = w.caret_xy();
    w.key(sym::HOME, 0, "");
    let (x_home, y_home) = w.caret_xy();
    let home = w.sel().1;
    assert!(
        home > 0 && home < 60 && y_home == y_end,
        "Home: the start of the last painted line, {home}"
    );
    let bullet = (x_end - x_home) / (60 - home) as f32;
    w.key(sym::END, 0, "");
    assert_eq!(w.sel().1, 60);
    w.key(sym::UP, 0, "");
    let (x_up, y_up) = w.caret_xy();
    assert!(y_up < y_end, "Up went to the line above: {y_up} < {y_end}");
    assert!(
        (x_up - x_end).abs() < bullet / 2.0 + 0.01,
        "the column is kept in bullets, not in the letters under them: {x_up} vs {x_end}"
    );
    let up = w.sel().1;
    assert!(
        up < home && up > 0,
        "a character offset, not a bullet's bytes: {up}"
    );
    w.key(sym::DOWN, 0, "");
    assert_eq!((w.caret_xy().1, w.sel().1), (y_end, 60));
}

#[test]
fn a_text_that_differs_only_by_its_line_endings_is_not_a_replacement() {
    let mut w = Win::new();
    w.field(true, 300.0, 100.0);
    w.apply(|p| {
        p.set_text(1, "a\r\nb\r\nc");
    });
    w.key(sym::HOME, MOD_CTRL, "");
    w.key(sym::RIGHT, 0, "");
    assert_eq!(w.sel(), (1, 1));
    // An unrelated patch: the caret stays where it is.
    w.apply(|p| {
        p.set_prop(1, id::MAX_LENGTH, &[100]);
    });
    assert_eq!(w.sel(), (1, 1), "the editor was not reset by the same text");
    w.key(sym::DELETE, 0, "");
    assert_eq!(w.text(), "ab\nc", "and it edits the normalised text");
}

#[test]
fn a_view_made_for_a_reused_slot_is_refused() {
    let mut w = Win::new();
    w.field(true, 200.0, 80.0);
    let live = w.core.scene.id(1);
    let stale = wolkit::scene::NodeId::new(1, live.generation().wrapping_add(1));
    let view = |id| wolkit::scene::EditView {
        node: 1,
        id,
        active: false,
        text: String::new(),
        base: String::new(),
        caret: 0,
        selection: 0..0,
        preedit: None,
        scroll: (0.0, 40.0),
    };
    assert!(
        !w.core.scene.set_edit_view(view(stale)),
        "another generation of the slot"
    );
    assert!(w.core.scene.edit_view(1).is_none_or(|v| v.scroll.1 == 0.0));
    assert!(w.core.scene.set_edit_view(view(live)));
    // Clearing by the stale id leaves the live node's view alone.
    w.core.scene.clear_edit_view(stale);
    assert_eq!(w.core.scene.edit_view(1).unwrap().scroll.1, 40.0);
    w.core.scene.clear_edit_view(live);
    assert!(w.core.scene.edit_view(1).is_none());
}

#[test]
fn the_headless_engine_breaks_a_line_in_a_text_area_for_the_shard_specs() {
    // The Crystal shard's specs run on the headless backend: Enter in a field whose white space is
    // pre-wrap appends a line break and reports it as a change (no submit), Ctrl+Enter submits.
    let engine =
        wolkit::engine::Engine::new("org.example.area", wolkit::abi::ENGINE_HEADLESS).unwrap();
    let win = engine.create_window("Area", 300, 200, 0).unwrap();
    let mut p = Writer::new();
    p.create(1, Kind::TextInput).insert(0, 1, APPEND);
    p.set_prop(1, id::WHITE_SPACE, &[2]);
    p.bind_event(
        1,
        event::bit(ev::TEXT_CHANGED) | event::bit(ev::TEXT_SUBMIT),
    );
    p.focus(1);
    engine.apply(win, p.finish(1)).unwrap();
    engine.inject_text(win, "first").unwrap();
    let key = |keysym, mods| {
        engine
            .inject_key(
                win,
                wolkit::window::KeyInput {
                    keysym,
                    pressed: true,
                    mods,
                },
            )
            .unwrap();
    };
    key(0xff0d, 0);
    let mut buf = vec![0u8; 4096];
    let n = engine.read_events(&mut buf);
    let evs = event::parse(&buf[..n]);
    let last = evs.last().expect("an event");
    assert_eq!(last.0.kind, ev::TEXT_CHANGED);
    assert_eq!(&last.1[12..18], b"first\n");
    key(0xff0d, wolkit::window::MOD_CTRL);
    let n = engine.read_events(&mut buf);
    assert_eq!(
        event::parse(&buf[..n]).last().unwrap().0.kind,
        ev::TEXT_SUBMIT
    );
}

#[test]
fn a_secure_field_maps_its_bullets_back_to_characters() {
    let mut w = Win::new();
    w.field(false, 200.0, 32.0);
    w.apply(|p| {
        p.set_prop(1, id::SECURE, &[1]);
    });
    w.type_str("abc");
    assert_eq!(w.sel(), (3, 3));
    let (x0, y0) = w.caret_xy();
    assert!(x0 > 20.0, "three bullets are drawn before the caret: {x0}");
    w.click(300.0, y0 + 4.0);
    assert_eq!(
        w.sel(),
        (3, 3),
        "a click past the bullets is the end, in characters not bytes"
    );
    w.click(12.0, y0 + 4.0);
    assert_eq!(w.sel(), (0, 0));
    w.key(sym::LEFT, 0, "");
    w.key(sym::END, 0, "");
    assert_eq!(w.sel(), (3, 3));
    assert!(w.pixels().iter().any(|&b| b != 0), "and it paints");
}

// ---- what paint draws -----------------------------------------------------------------------

fn golden_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/text_area_golden")
}

fn check_golden(name: &str, w: &mut Win) {
    let rgba = w.pixels();
    let (width, height) = (w.core.width, w.core.height);
    let path = golden_dir().join(format!("{name}.png"));
    if std::env::var_os("WOLKIT_BLESS").is_some() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, png::encode(width, height, &rgba)).unwrap();
        return;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{name}: no golden at {} ({e}); run with WOLKIT_BLESS=1",
            path.display()
        )
    });
    let (gw, gh, want) = png::decode(&bytes).unwrap();
    assert_eq!((gw, gh), (width, height), "{name}: size");
    if rgba != want {
        let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        std::fs::write(&out, png::encode(width, height, &rgba)).unwrap();
        let diff = rgba
            .chunks(4)
            .zip(want.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        panic!(
            "{name}: {diff} pixels differ; actual written to {}",
            out.display()
        );
    }
}

#[test]
fn golden_three_paragraphs_wrapped() {
    let mut w = Win::new();
    w.field(true, 340.0, 220.0);
    {
        let (c, mut s, st) = w.sink();
        edit::paste(c, &mut s, st, THREE);
    }
    w.key(sym::HOME, MOD_CTRL, "");
    // Wrapped: more than three lines for three paragraphs.
    let lines = {
        let (c, _, st) = w.sink();
        let _ = (c, st);
        w.st.field.editor.text().lines().count()
    };
    assert_eq!(lines, 3);
    check_golden("three_paragraphs_wrapped", &mut w);
}

#[test]
fn golden_selection_across_lines() {
    let mut w = Win::new();
    w.field(true, 340.0, 220.0);
    {
        let (c, mut s, st) = w.sink();
        edit::paste(c, &mut s, st, THREE);
    }
    w.key(sym::HOME, MOD_CTRL, "");
    for _ in 0..30 {
        w.key(sym::RIGHT, 0, "");
    }
    w.key(sym::DOWN, MOD_SHIFT, "");
    w.key(sym::DOWN, MOD_SHIFT, "");
    w.key(sym::DOWN, MOD_SHIFT, "");
    w.key(sym::END, MOD_SHIFT, "");
    let (a, h) = w.sel();
    assert!(a < h && w.st.field.editor.selected_text().contains('\n'));
    check_golden("selection_across_lines", &mut w);
}

#[test]
fn golden_undo_of_twenty_edits() {
    let mut w = Win::new();
    w.field(true, 340.0, 220.0);
    let words: Vec<String> = (1..=20).map(|i| format!("edit{i}")).collect();
    for (i, word) in words.iter().enumerate() {
        w.type_str(word);
        w.type_str(if i % 4 == 3 { "\n" } else { " " });
    }
    let full = w.text();
    assert_eq!(full.lines().count(), 5);
    for _ in 0..12 {
        w.key(u32::from('z'), MOD_CTRL, "z");
    }
    let kept: String = words[..8]
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{s}{}", if i % 4 == 3 { "\n" } else { " " }))
        .collect();
    assert_eq!(w.text(), kept, "twelve of the twenty edits undone");
    check_golden("undo_of_twenty_edits", &mut w);
    for _ in 0..8 {
        w.key(u32::from('z'), MOD_CTRL, "z");
    }
    assert_eq!(w.text(), "");
    for _ in 0..20 {
        w.key(u32::from('z'), MOD_CTRL | MOD_SHIFT, "Z");
    }
    assert_eq!(w.text(), full, "and all twenty redone");
}

// ---- the fuzz target's property, on stable ------------------------------------------------------

fn seeds() -> Vec<Vec<u8>> {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/regressions/wolkit_text");
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    v.iter().map(|p| std::fs::read(p).unwrap()).collect()
}

#[test]
fn the_text_buffer_seeds_hold_the_property() {
    let s = seeds();
    assert!(s.len() >= 8);
    let mut ran = 0;
    for d in &s {
        ran += text_fuzz::run(d).ops;
    }
    assert!(ran > 80, "the seeds run operations: {ran}");
    // The window programs (top bit of the first byte) reach the engine's routing and paint frames.
    let frames: usize = s
        .iter()
        .filter(|d| d.first().is_some_and(|f| f & 0x80 != 0))
        .map(|d| text_fuzz::run(d).geometry)
        .sum();
    assert!(frames >= 5, "the engine seeds paint: {frames}");
}

#[test]
fn mutated_text_buffer_programs_hold_the_property() {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let seeds = seeds();
    let mut total = text_fuzz::Stats::default();
    for _ in 0..500u32 {
        let mut d = seeds[next() as usize % seeds.len()].clone();
        for _ in 0..1 + next() % 6 {
            let at = if d.is_empty() {
                0
            } else {
                next() as usize % d.len()
            };
            match next() % 5 {
                0 if !d.is_empty() => d[at] = next() as u8,
                1 if !d.is_empty() => d[at] ^= 1 << (next() % 8),
                2 => d.truncate(at),
                3 => d.insert(at, next() as u8),
                _ => {
                    let extra = d.clone();
                    d.extend_from_slice(&extra[..extra.len().min(40)]);
                }
            }
        }
        let st = text_fuzz::run(&d);
        total.ops += st.ops;
        total.edits += st.edits;
        total.geometry += st.geometry;
        total.undone += st.undone;
    }
    assert!(
        total.ops > 3_000 && total.edits > 800 && total.geometry > 400 && total.undone > 300,
        "the mutants must get somewhere: {total:?}"
    );
}
