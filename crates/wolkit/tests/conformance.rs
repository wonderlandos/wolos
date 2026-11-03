// SPDX-License-Identifier: BSD-2-Clause
//! The conformance suite: one set of scenarios, run on every backend (plan section 1.4). A backend
//! may differ only where `EXPECTED` says so, with the reason; anything else is a bug in the
//! backend. The scene is backend independent, so its dump must be identical everywhere.
//!
//! Backends run here: `headless` and `kit` (against a fake Kit host). `native` joins when WK-W4
//! lands it, by adding a `Fixture` variant and removing its rows from `EXPECTED`.

mod common;

use common::fakes::FakeKit;
use shared::view::{FromHost, Node, ToHost};
use wolkit::abi::{DISPATCH_EVENTS, ENGINE_HEADLESS};
use wolkit::engine::Engine;
use wolkit::event::{self, kind as ev};
use wolkit::scene::patch::{APPEND, Writer};
use wolkit::scene::prop::id;
use wolkit::scene::{Kind, state};
use wolkit::window::{POINTER_DOWN, POINTER_MOVE, POINTER_UP, PointerInput};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    Headless,
    Kit,
}

/// Where a backend is allowed to differ, and why.
const EXPECTED: &[(&str, Backend, &str)] = &[
    ("many_nodes", Backend::Kit, "the Kit wire carries at most 200 parts"),
    ("close_request", Backend::Headless, "only a display can close a window under the person"),
];

fn expected(scenario: &str, b: Backend) -> Option<&'static str> {
    EXPECTED.iter().find(|(s, be, _)| *s == scenario && *be == b).map(|r| r.2)
}

struct Fixture {
    backend: Backend,
    engine: Arc<Engine>,
    win: u32,
    kit: Option<FakeKit>,
    _dir: std::path::PathBuf,
}

impl Fixture {
    fn new(backend: Backend, tag: &str) -> Fixture {
        let dir = common::temp(&format!("conf-{tag}-{backend:?}"));
        let (engine, kit) = match backend {
            Backend::Headless => (Engine::new("org.example.conf", ENGINE_HEADLESS).unwrap(), None),
            Backend::Kit => {
                let kit = FakeKit::start(&dir);
                let e = Engine::new("org.example.conf", wolkit::abi::ENGINE_FORCE_KIT).unwrap();
                e.set_kit_socket(&kit.path);
                (e, Some(kit))
            }
        };
        let win = engine.create_window("Conformance", 400, 300, 0).unwrap();
        Fixture { backend, engine, win, kit, _dir: dir }
    }

    fn show(&self) {
        self.engine.show(self.win).unwrap();
    }

    fn apply(&self, w: &mut Writer, seq: u32) -> wolkit::error::Result<()> {
        let r = self.engine.apply(self.win, w.finish(seq));
        w.clear();
        r
    }

    fn dump(&self) -> String {
        self.engine.dump(self.win, wolkit::window::dump::SCENE).unwrap()
    }

    /// Wait for the engine to have `n` events pending (the Kit reader thread is asynchronous).
    fn events(&self, n: usize) -> Vec<(event::Header, Vec<u8>)> {
        let mut out = Vec::new();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let flags = self.engine.dispatch(50).unwrap();
            let mut buf = vec![0u8; 65536];
            let got = self.engine.read_events(&mut buf);
            out.extend(event::parse(&buf[..got]).into_iter().map(|(h, p)| (h, p.to_vec())));
            if out.len() >= n || std::time::Instant::now() > end {
                let _ = flags & DISPATCH_EVENTS;
                return out;
            }
        }
    }

    /// The person clicks a part.
    fn click(&self, node: u32) {
        match self.backend {
            Backend::Headless => {
                let r = self.engine.node_rect(self.win, node).unwrap();
                let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                for kind in [POINTER_MOVE, POINTER_DOWN, POINTER_UP] {
                    self.engine.inject_pointer(self.win, PointerInput { kind, x, y, button: 1, mods: 0 }).unwrap();
                }
            }
            Backend::Kit => self.kit.as_ref().unwrap().send(FromHost::Clicked { window: self.win, widget: node }),
        }
    }

    /// The person types a whole text into a field.
    fn type_text(&self, node: u32, text: &str) {
        match self.backend {
            Backend::Headless => {
                self.engine.set_focus(self.win, node).unwrap();
                self.engine.inject_text(self.win, text).unwrap();
            }
            Backend::Kit => self.kit.as_ref().unwrap().send(FromHost::Edited { window: self.win, widget: node, text: text.into() }),
        }
    }

    fn toggle(&self, node: u32, on: bool) {
        match self.backend {
            Backend::Headless => {
                let r = self.engine.node_rect(self.win, node).unwrap();
                let (x, y) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                for kind in [POINTER_MOVE, POINTER_DOWN, POINTER_UP] {
                    self.engine.inject_pointer(self.win, PointerInput { kind, x, y, button: 1, mods: 0 }).unwrap();
                }
            }
            Backend::Kit => self.kit.as_ref().unwrap().send(FromHost::Toggled { window: self.win, widget: node, on }),
        }
    }
}

/// A small app: column 1 holds a label 2, a field 3, a button 4, a switch 5.
fn app(w: &mut Writer) {
    w.create(1, Kind::Box).create(2, Kind::Text).create(3, Kind::TextInput).create(4, Kind::Button).create(5, Kind::Toggle);
    w.set_prop(1, id::FLEX_DIRECTION, &[1]).set_f32(1, id::GAP, 8.0);
    w.insert(0, 1, APPEND);
    for n in 2..=5 {
        w.insert(1, n, APPEND);
    }
    w.set_text(2, "Hello").set_text(4, "Add").set_text(5, "Notify me");
    let p = w.string("What needs doing?");
    w.set_prop(3, id::PLACEHOLDER, &[p]);
    w.bind_event(3, event::bit(ev::TEXT_CHANGED)).bind_event(4, event::bit(ev::CLICK)).bind_event(5, event::bit(ev::TOGGLE));
    w.set_a11y(4, 3, "Add", "").set_a11y(3, 7, "New item", "");
}

fn each_backend(scenario: &str, mut f: impl FnMut(&Fixture)) {
    for b in [Backend::Headless, Backend::Kit] {
        if let Some(why) = expected(scenario, b) {
            eprintln!("{scenario} on {b:?}: expected to differ ({why})");
            continue;
        }
        f(&Fixture::new(b, scenario));
    }
}

#[test]
fn the_same_scene_everywhere() {
    let mut dumps = Vec::new();
    each_backend("scene", |f| {
        let mut w = Writer::new();
        app(&mut w);
        f.apply(&mut w, 1).unwrap();
        f.show();
        dumps.push(f.dump());
    });
    assert_eq!(dumps.len(), 2);
    assert_eq!(dumps[0], dumps[1], "the scene is backend independent");
    assert!(dumps[0].contains("\"Notify me\""), "{}", dumps[0]);
}

#[test]
fn a_click_reaches_only_the_part_that_asked() {
    each_backend("click", |f| {
        let mut w = Writer::new();
        app(&mut w);
        f.apply(&mut w, 7).unwrap();
        f.show();
        f.click(2); // the label did not ask for clicks
        f.click(4);
        let evs: Vec<_> = f.events(1).into_iter().filter(|(h, _)| h.kind == ev::CLICK).collect();
        assert_eq!(evs.len(), 1, "{:?}", f.backend);
        assert_eq!((evs[0].0.node, evs[0].0.window, evs[0].0.seq), (4, f.win, 7), "{:?}", f.backend);
    });
}

#[test]
fn typing_makes_a_text_event_and_the_text_is_readable() {
    each_backend("typing", |f| {
        let mut w = Writer::new();
        app(&mut w);
        f.apply(&mut w, 1).unwrap();
        f.show();
        f.type_text(3, "milk");
        let evs: Vec<_> = f.events(1).into_iter().filter(|(h, _)| h.kind == ev::TEXT_CHANGED).collect();
        assert_eq!(evs.len(), 1, "{:?}", f.backend);
        let p = &evs[0].1;
        assert_eq!(&p[12..16], b"milk", "{:?}", f.backend);
        let mut buf = [0u8; 16];
        let n = f.engine.read_text(f.win, 3, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"milk");
    });
}

#[test]
fn a_switch_reports_its_new_state() {
    each_backend("toggle", |f| {
        let mut w = Writer::new();
        app(&mut w);
        f.apply(&mut w, 1).unwrap();
        f.show();
        f.toggle(5, true);
        let evs: Vec<_> = f.events(1).into_iter().filter(|(h, _)| h.kind == ev::TOGGLE).collect();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].1[0], 1, "{:?}", f.backend);
        assert_ne!(f.engine.dump(f.win, wolkit::window::dump::A11Y).unwrap().find("state=checked"), None);
    });
}

#[test]
fn a_bad_patch_changes_nothing_and_says_why() {
    each_backend("atomic", |f| {
        let mut w = Writer::new();
        app(&mut w);
        f.apply(&mut w, 1).unwrap();
        f.show();
        let before = f.dump();
        // Valid ops first, then a part that does not exist: all of it is undone.
        w.create(9, Kind::Text).insert(1, 9, 0).set_text(9, "ghost").set_text(2, "changed").set_state(77, state::DISABLED);
        let e = f.apply(&mut w, 2).unwrap_err();
        assert!(e.message.contains("Part 77"), "{}", e.message);
        assert_eq!(f.dump(), before, "{:?}", f.backend);
        // The sequence did not advance and the window still works.
        f.click(4);
        let evs: Vec<_> = f.events(1).into_iter().filter(|(h, _)| h.kind == ev::CLICK).collect();
        assert_eq!(evs[0].0.seq, 1);
    });
}

#[test]
fn freed_parts_can_be_made_again() {
    each_backend("free", |f| {
        let mut w = Writer::new();
        app(&mut w);
        f.apply(&mut w, 1).unwrap();
        f.show();
        w.free(4);
        f.apply(&mut w, 2).unwrap();
        assert!(!f.dump().contains("button"));
        // The same slot with a new generation.
        w.create(0x0100_0004, Kind::Button).insert(1, 0x0100_0004, APPEND).set_text(0x0100_0004, "Again");
        f.apply(&mut w, 3).unwrap();
        assert!(f.dump().contains("\"Again\""));
        // The old id is stale now.
        w.set_text(4, "stale");
        assert!(f.apply(&mut w, 4).is_err());
    });
}

#[test]
fn a_window_has_a_limit_on_parts() {
    each_backend("many_nodes", |f| {
        let mut w = Writer::new();
        w.create(1, Kind::Box).insert(0, 1, APPEND);
        f.apply(&mut w, 1).unwrap();
        let mut seq = 2;
        // Fill to the limit in patches of 1000, then one more.
        let mut next = 2u32;
        while next < 65_536 {
            let end = (next + 1000).min(65_536);
            for n in next..end {
                w.create(n, Kind::Spacer).insert(1, n, APPEND);
            }
            f.apply(&mut w, seq).unwrap();
            seq += 1;
            next = end;
        }
        w.create(65_536, Kind::Spacer);
        let e = f.apply(&mut w, seq).unwrap_err();
        assert_eq!(e.status, wolkit::error::Status::Limit);
        assert!(e.message.contains("65536"), "{}", e.message);
    });
}

#[test]
fn the_person_can_close_the_window() {
    each_backend("close_request", |f| {
        let mut w = Writer::new();
        app(&mut w);
        f.apply(&mut w, 1).unwrap();
        f.show();
        f.kit.as_ref().unwrap().send(FromHost::Closed { window: f.win });
        let evs = f.events(1);
        assert!(evs.iter().any(|(h, _)| h.kind == ev::CLOSE_REQUEST));
    });
}

#[test]
fn the_kit_backend_lowers_to_the_expected_tree() {
    let f = Fixture::new(Backend::Kit, "lower");
    let mut w = Writer::new();
    app(&mut w);
    f.apply(&mut w, 1).unwrap();
    f.show();
    let seen = f.kit.as_ref().unwrap().wait(2);
    let ToHost::Hello { app } = &seen[0] else { panic!("{seen:?}") };
    assert_eq!(app, "org.example.conf");
    let ToHost::Show { title, content, .. } = &seen[1] else { panic!("{seen:?}") };
    assert_eq!(title, "Conformance");
    let got = serde_json::to_string_pretty(content).unwrap();
    let golden_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/kit_app.json");
    if std::env::var_os("WOLKIT_BLESS").is_some() {
        std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        std::fs::write(&golden_path, got.clone() + "\n").unwrap();
    }
    let want = std::fs::read_to_string(&golden_path).unwrap_or_default();
    assert_eq!(got.trim(), want.trim(), "the lowering changed; rerun with WOLKIT_BLESS=1 and review the diff");
    assert!(matches!(content, Node::Stack { .. }));
}
