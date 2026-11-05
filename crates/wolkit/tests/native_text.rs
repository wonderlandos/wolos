// SPDX-License-Identifier: BSD-2-Clause
//! The focused `text_input` through `wolkit::input` (WK-W4) over WK-W3's editor: typing, editing
//! keys, undo, the clipboard requests, `max-length`, read-only and secure fields, and input-method
//! batches with text-input-v3 semantics (preedit never reaches the node; commit and
//! delete-surrounding apply at `done`). The Wayland side of the same batches is in
//! `native_wayland.rs`.

use wolkit::env::Environment;
use wolkit::event::{self, EventQueue, kind as ev};
use wolkit::input::edit::{self, Ime};
use wolkit::input::keys::{self, KeyPress, sym};
use wolkit::input::{InputState, Request, focus};
use wolkit::scene::patch::{APPEND, Patch, Writer};
use wolkit::scene::prop::id;
use wolkit::scene::{Kind, SceneRead};
use wolkit::window::{MOD_CTRL, MOD_SHIFT, Sink, WindowCore};

struct Win {
    core: WindowCore,
    events: EventQueue,
    st: InputState,
    seq: u32,
}

impl Win {
    /// A window with one focused text field (node 1) that reports its text.
    fn field(extra: impl FnOnce(&mut Writer)) -> Win {
        let mut w = Win {
            core: WindowCore::new(1, "Test", 400, 300, 0, Environment::default()),
            events: EventQueue::new(),
            st: InputState::new(),
            seq: 0,
        };
        w.apply(|p| {
            p.create(1, Kind::TextInput).insert(0, 1, APPEND);
            p.bind_event(
                1,
                event::bit(ev::TEXT_CHANGED) | event::bit(ev::TEXT_SUBMIT),
            );
            extra(p);
            p.focus(1);
        });
        assert_eq!(
            edit::focus_changed(&mut w.core, &mut w.st),
            Request::Ime(w.st.field.editor.ime_enabled())
        );
        w
    }

    fn apply(&mut self, f: impl FnOnce(&mut Writer)) {
        let mut p = Writer::new();
        f(&mut p);
        self.seq += 1;
        let bytes = p.finish(self.seq).to_vec();
        let c = &mut self.core;
        let patch = Patch::parse(&bytes, &mut c.scratch).unwrap();
        c.scene.try_apply(&patch, &mut c.journal).unwrap();
        c.scene.commit(&mut c.journal);
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: self.seq,
        };
        focus::sync(c, &mut s, &mut self.st);
    }

    fn key_text(&mut self, keysym: u32, mods: u32, text: &str) -> Request {
        let k = KeyPress {
            keysym,
            pressed: true,
            mods,
            repeat: false,
            text,
        };
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: 0,
        };
        keys::key(&mut self.core, &mut s, &mut self.st, k)
    }

    fn key(&mut self, keysym: u32, mods: u32) -> Request {
        self.key_text(keysym, mods, "")
    }

    fn typed(&mut self, t: &str) {
        for c in t.chars() {
            let mut b = [0u8; 4];
            self.key_text(u32::from(c), 0, c.encode_utf8(&mut b));
        }
    }

    fn text(&self) -> &str {
        self.core.scene.text(1)
    }

    /// `(kind, caret, anchor, text)` of each text event since the last call.
    fn changes(&mut self) -> Vec<(u16, u32, u32, String)> {
        let mut buf = vec![0u8; 1 << 20];
        let n = self.events.read(&mut buf);
        event::parse(&buf[..n])
            .into_iter()
            .filter(|(h, _)| h.kind == ev::TEXT_CHANGED || h.kind == ev::TEXT_SUBMIT)
            .map(|(h, p)| {
                let w = |i: usize| u32::from_le_bytes(p[i..i + 4].try_into().unwrap());
                let len = usize::try_from(w(8)).unwrap();
                (
                    h.kind,
                    w(0),
                    w(4),
                    String::from_utf8(p[12..12 + len].to_vec()).unwrap(),
                )
            })
            .collect()
    }
}

#[test]
fn typing_edits_and_reports_caret_and_text() {
    let mut w = Win::field(|_| {});
    w.typed("héllo");
    assert_eq!(w.text(), "héllo");
    let last = w.changes().pop().unwrap();
    assert_eq!(
        last,
        (ev::TEXT_CHANGED, 6, 6, "héllo".into()),
        "byte offsets"
    );
    w.key(sym::LEFT, 0);
    w.key(sym::LEFT, MOD_SHIFT);
    w.key(sym::BACKSPACE, 0);
    assert_eq!(w.text(), "hélo");
    w.key(sym::BACKSPACE, MOD_CTRL);
    assert_eq!(w.text(), "o", "control+backspace deletes the word");
    w.key(sym::Z, MOD_CTRL);
    assert_eq!(w.text(), "hélo", "undo");
    w.key(sym::Z_UPPER, MOD_CTRL | MOD_SHIFT);
    assert_eq!(w.text(), "o", "redo");
    w.changes();
    w.key(sym::RETURN, 0);
    assert_eq!(w.changes(), vec![(ev::TEXT_SUBMIT, 0, 0, "o".into())]);
}

#[test]
fn clipboard_keys_ask_the_backend_and_paste_inserts() {
    let mut w = Win::field(|_| {});
    w.typed("copy me");
    w.key(sym::A, MOD_CTRL);
    assert_eq!(w.key(sym::C, MOD_CTRL), Request::Copy("copy me".into()));
    assert_eq!(w.key(sym::X, MOD_CTRL), Request::Copy("copy me".into()));
    assert_eq!(w.text(), "");
    assert_eq!(w.key(sym::V, MOD_CTRL), Request::Paste);
    let (core, st) = (&mut w.core, &mut w.st);
    let mut s = Sink {
        events: &mut w.events,
        window: 1,
        seq: 0,
    };
    assert!(edit::paste(core, &mut s, st, "line one\nline two"));
    assert_eq!(
        w.text(),
        "line one line two",
        "a single-line field turns breaks into spaces"
    );
}

#[test]
fn max_length_counts_characters_and_cuts_typing_paste_and_commits() {
    let mut w = Win::field(|p| {
        p.set_prop(1, id::MAX_LENGTH, &[4]);
    });
    w.typed("abcdef");
    assert_eq!(w.text(), "abcd");
    w.key(sym::BACKSPACE, 0);
    let (core, st) = (&mut w.core, &mut w.st);
    edit::ime(core, st, Ime::Commit("ééé".into()));
    let mut s = Sink {
        events: &mut w.events,
        window: 1,
        seq: 0,
    };
    edit::ime_done(core, &mut s, st, 1);
    assert_eq!(w.text(), "abcé");
}

#[test]
fn read_only_fields_move_and_copy_but_never_change() {
    let mut w = Win::field(|p| {
        p.set_text(1, "fixed");
        p.set_prop(1, id::READONLY, &[1]);
    });
    assert!(
        !w.st.field.editor.ime_enabled(),
        "no input method for a read-only field"
    );
    w.typed("xyz");
    w.key(sym::BACKSPACE, 0);
    assert_eq!(w.text(), "fixed");
    w.key(sym::HOME, MOD_SHIFT);
    assert_eq!(w.key(sym::C, MOD_CTRL), Request::Copy("fixed".into()));
    assert_eq!(w.key(sym::X, MOD_CTRL), Request::None);
    assert_eq!(w.key(sym::V, MOD_CTRL), Request::None);
    assert!(w.changes().is_empty());
}

#[test]
fn secure_fields_never_hand_out_their_text() {
    let mut w = Win::field(|p| {
        p.set_prop(1, id::SECURE, &[1]);
    });
    w.typed("hunter2");
    w.key(sym::A, MOD_CTRL);
    assert_eq!(w.key(sym::C, MOD_CTRL), Request::None);
    assert_eq!(w.key(sym::X, MOD_CTRL), Request::None);
    assert_eq!(w.text(), "hunter2");
    assert!(
        edit::surrounding(&w.st).is_none(),
        "nothing for the input method either"
    );
}

#[test]
fn preedit_stays_out_of_the_node_until_the_commit() {
    let mut w = Win::field(|_| {});
    w.typed("ni");
    w.changes();
    let (core, st) = (&mut w.core, &mut w.st);
    edit::ime(
        core,
        st,
        Ime::Preedit {
            text: "你".into(),
            cursor: Some((3, 3)),
        },
    );
    let mut s = Sink {
        events: &mut w.events,
        window: 1,
        seq: 0,
    };
    edit::ime_done(core, &mut s, st, 1);
    assert_eq!(w.text(), "ni", "the preedit is drawn, not written");
    let d = w.st.field.display().unwrap();
    assert_eq!(d.text, "ni你");
    assert_eq!(d.preedit, Some(2..5));
    assert_eq!(d.caret, 5);
    assert!(w.changes().is_empty(), "no TEXT_CHANGED for a preedit");
    // The input method replaces "ni" with the character: delete 2 before, commit, no preedit.
    let (core, st) = (&mut w.core, &mut w.st);
    edit::ime(
        core,
        st,
        Ime::DeleteSurrounding {
            before: 2,
            after: 0,
        },
    );
    edit::ime(core, st, Ime::Commit("你".into()));
    let mut s = Sink {
        events: &mut w.events,
        window: 1,
        seq: 0,
    };
    edit::ime_done(core, &mut s, st, 2);
    assert_eq!(w.text(), "你");
    assert_eq!(w.changes(), vec![(ev::TEXT_CHANGED, 3, 3, "你".into())]);
    assert_eq!(edit::surrounding(&w.st), Some(("你".into(), 3, 3)));
}

#[test]
fn escape_cancels_the_preedit_before_anything_else() {
    let mut w = Win::field(|p| {
        p.bind_event(0, event::bit(ev::CLOSE_REQUEST));
    });
    let (core, st) = (&mut w.core, &mut w.st);
    edit::ime(
        core,
        st,
        Ime::Preedit {
            text: "ka".into(),
            cursor: None,
        },
    );
    let mut s = Sink {
        events: &mut w.events,
        window: 1,
        seq: 0,
    };
    edit::ime_done(core, &mut s, st, 1);
    w.key(sym::ESCAPE, 0);
    assert!(w.st.field.editor.preedit().is_none());
    assert_eq!(w.text(), "");
}

#[test]
fn app_text_wins_over_the_editor_while_focused() {
    let mut w = Win::field(|_| {});
    w.typed("draft");
    w.apply(|p| {
        p.set_text(1, "reset");
    });
    w.typed("!");
    assert_eq!(
        w.text(),
        "reset!",
        "the editor took the app's value, caret at the end"
    );
}
