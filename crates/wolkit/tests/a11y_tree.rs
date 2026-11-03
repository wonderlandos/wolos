// SPDX-License-Identifier: BSD-2-Clause
//! The accessibility tree (`wolkit::a11y`, WK-W4) of a small Todos window against a golden dump
//! (`tests/a11y_golden/todos.txt`; rerun with `WOLKIT_BLESS=1` and review the diff), plus the
//! rules behind it: names from content, hidden parts left out, secure text masked, the lint for
//! nameless controls, and state changes showing up.

use wolkit::a11y::{self, roles};
use wolkit::event::{self, kind as ev};
use wolkit::scene::prop::id;
use wolkit::scene::{Kind, state};

#[path = "a11y_golden/todos.rs"]
mod todos;

use todos::{Win, place, todos};

#[test]
fn todos_tree_matches_the_golden_dump() {
    let w = todos();
    let got = a11y::dump(&w.tree());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/a11y_golden/todos.txt");
    if std::env::var_os("WOLKIT_BLESS").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        got, want,
        "a11y dump changed; rerun with WOLKIT_BLESS=1 and review the diff"
    );
}

#[test]
fn button_label_is_its_name_and_is_not_read_twice() {
    let t = todos().tree();
    let add = t.nodes.iter().find(|n| n.id == 4).unwrap();
    assert_eq!(add.name, "Add");
    assert!(
        add.children.is_empty(),
        "the label child is folded into the button"
    );
    assert_eq!(add.wire_role, roles::role::BUTTON);
    assert_eq!(add.action, Some("click"));
    assert!(
        !t.nodes.iter().any(|n| n.id == 5 || n.id == 10),
        "label and hidden text left out"
    );
}

#[test]
fn states_follow_the_scene() {
    let mut w = todos();
    let has = |t: &a11y::Tree, id: u32, b: u32| t.nodes.iter().find(|n| n.id == id).unwrap().has(b);
    let t = w.tree();
    assert!(has(&t, 3, roles::st::FOCUSED));
    assert!(has(&t, 3, roles::st::EDITABLE));
    assert!(has(&t, 7, roles::st::CHECKED) && has(&t, 7, roles::st::CHECKABLE));
    assert!(!has(&t, 8, roles::st::CHECKED));
    assert!(!has(&t, 11, roles::st::ENABLED) && !has(&t, 11, roles::st::SENSITIVE));
    w.apply(|p| {
        p.set_state(8, state::CHECKED | state::BUSY);
        p.focus(4);
    });
    let t = w.tree();
    assert!(has(&t, 8, roles::st::CHECKED) && has(&t, 8, roles::st::BUSY));
    assert!(has(&t, 4, roles::st::FOCUSED) && !has(&t, 3, roles::st::FOCUSED));
}

#[test]
fn secure_text_is_masked_and_caret_counts_characters() {
    let w = Win::new(|p| {
        place(p, 1, Kind::TextInput, 0, [0.0, 0.0, 100.0, 20.0]);
        p.set_a11y(1, 0, "Password", "");
        p.set_text(1, "pässwörd");
        p.set_prop(1, id::SECURE, &[1]);
        place(p, 2, Kind::TextInput, 0, [0.0, 30.0, 100.0, 20.0]);
        p.set_a11y(2, 0, "Name", "");
        p.set_text(2, "Zoë");
        p.focus(2);
    });
    let t = w.tree();
    let pw = t.nodes.iter().find(|n| n.id == 1).unwrap();
    assert_eq!(pw.wire_role, roles::role::PASSWORD_TEXT);
    assert_eq!(pw.text.as_ref().unwrap().text, "\u{2022}".repeat(8));
    let name = t
        .nodes
        .iter()
        .find(|n| n.id == 2)
        .unwrap()
        .text
        .clone()
        .unwrap();
    assert_eq!(
        (name.caret, name.selection),
        (3, (3, 3)),
        "characters, not bytes"
    );
}

#[test]
fn nameless_controls_are_linted() {
    let w = Win::new(|p| {
        place(p, 1, Kind::Button, 0, [0.0, 0.0, 20.0, 20.0]);
        place(p, 2, Kind::Button, 0, [30.0, 0.0, 20.0, 20.0]);
        p.set_a11y(2, 0, "Delete", "");
        place(p, 3, Kind::Box, 0, [60.0, 0.0, 20.0, 20.0]);
        p.bind_event(3, event::bit(ev::CLICK));
    });
    let l = a11y::lint(&w.tree());
    assert_eq!(l.len(), 2, "{l:?}");
    assert_eq!((l[0].0, l[1].0), (1, 3));
    assert!(l[0].1.starts_with("Part 1 (button) has no accessible name"));
    assert!(l[1].1.starts_with("Part 3 (panel)"));
}
