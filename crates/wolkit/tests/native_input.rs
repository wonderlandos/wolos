// SPDX-License-Identifier: BSD-2-Clause
//! Input routing (`wolkit::input`, WK-W4) on a window core with injected input: hit test, focus,
//! pointer capture, bubbling, keyboard navigation, the modal focus trap, shortcuts and pointer
//! coalescing. The native backend feeds the same functions from the seat, so this is its routing
//! test too (`native_wayland.rs` covers the protocol side).

use wolkit::env::Environment;
use wolkit::event::{self, EventQueue, kind as ev};
use wolkit::input::keys::{KeyPress, sym};
use wolkit::input::{
    BUTTON_LEFT, BUTTON_RIGHT, InputState, coalesce::Coalescer, focus, keys, pointer,
};
use wolkit::scene::patch::{APPEND, Patch, Writer};
use wolkit::scene::prop::{DIM_PX, id};
use wolkit::scene::{Kind, NIL, SceneRead, state};
use wolkit::window::{
    MOD_CTRL, MOD_SHIFT, POINTER_DOWN, POINTER_MOVE, POINTER_SCROLL, POINTER_UP, PointerInput,
    Sink, WindowCore,
};

struct Win {
    core: WindowCore,
    events: EventQueue,
    st: InputState,
    seq: u32,
}

type Ev = (u16, u32, Vec<u8>);

impl Win {
    fn new() -> Win {
        Win {
            core: WindowCore::new(1, "Test", 400, 300, 0, Environment::default()),
            events: EventQueue::new(),
            st: InputState::new(),
            seq: 0,
        }
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
    }

    fn events(&mut self) -> Vec<Ev> {
        let mut buf = vec![0u8; 1 << 20];
        let n = self.events.read(&mut buf);
        event::parse(&buf[..n])
            .into_iter()
            .map(|(h, p)| (h.kind, h.node, p.to_vec()))
            .collect()
    }

    fn ptr(&mut self, kind: u32, x: f32, y: f32, button: u32) {
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: 0,
        };
        let p = PointerInput {
            kind,
            x,
            y,
            button,
            mods: 0,
        };
        pointer::pointer(&mut self.core, &mut s, &mut self.st, p).unwrap();
    }

    fn click(&mut self, x: f32, y: f32) {
        self.ptr(POINTER_DOWN, x, y, BUTTON_LEFT);
        self.ptr(POINTER_UP, x, y, BUTTON_LEFT);
    }

    fn key(&mut self, keysym: u32, mods: u32) {
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: 0,
        };
        let k = KeyPress {
            keysym,
            pressed: true,
            mods,
            repeat: false,
            text: "",
        };
        keys::key(&mut self.core, &mut s, &mut self.st, k);
    }

    fn focused(&self) -> u32 {
        self.core.scene.focus()
    }

    fn state(&self, n: u32) -> u32 {
        self.core.scene.state(n)
    }
}

fn px(v: f32) -> [u32; 2] {
    [DIM_PX, v.to_bits()]
}

/// A node of `kind` under `parent`, `w` by `h` pixels, absolutely placed at `x`, `y`.
fn place(p: &mut Writer, node: u32, kind: Kind, parent: u32, r: [f32; 4]) {
    p.create(node, kind).insert(parent, node, APPEND);
    p.set_prop(node, id::POSITION, &[1]);
    p.set_prop(node, id::INSET_LEFT, &px(r[0]));
    p.set_prop(node, id::INSET_TOP, &px(r[1]));
    p.set_prop(node, id::WIDTH, &px(r[2]));
    p.set_prop(node, id::HEIGHT, &px(r[3]));
}

fn kinds(evs: &[Ev]) -> Vec<(u16, u32)> {
    evs.iter().map(|e| (e.0, e.1)).collect()
}

#[test]
fn tab_order_puts_positive_tab_index_first_and_shows_the_ring() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Button, 0, [0.0, 0.0, 50.0, 20.0]);
        place(p, 2, Kind::Button, 0, [60.0, 0.0, 50.0, 20.0]);
        place(p, 3, Kind::Button, 0, [120.0, 0.0, 50.0, 20.0]);
        p.set_prop(3, id::TAB_INDEX, &[1]);
        place(p, 4, Kind::Button, 0, [180.0, 0.0, 50.0, 20.0]);
        p.set_prop(4, id::TAB_INDEX, &[(-1i32).cast_unsigned()]);
    });
    let mut order = Vec::new();
    for _ in 0..4 {
        w.key(sym::TAB, 0);
        order.push(w.focused());
    }
    assert_eq!(order, vec![3, 1, 2, 3], "tab-index 1 first, -1 never");
    assert_ne!(
        w.state(3) & state::FOCUS_VISIBLE,
        0,
        "keyboard focus shows the ring"
    );
    w.key(sym::TAB, MOD_SHIFT);
    assert_eq!(w.focused(), 2, "shift+tab goes back");
    w.key(sym::ISO_LEFT_TAB, MOD_SHIFT);
    assert_eq!(w.focused(), 1);
}

#[test]
fn pointer_press_focuses_without_the_ring_and_the_click_bubbles() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Box, 0, [0.0, 0.0, 200.0, 100.0]);
        p.bind_event(1, event::bit(ev::CLICK) | event::bit(ev::FOCUS));
        place(p, 2, Kind::Button, 1, [10.0, 10.0, 80.0, 30.0]);
    });
    w.events();
    w.click(20.0, 20.0);
    assert_eq!(w.focused(), 2);
    assert_eq!(
        w.state(2) & state::FOCUS_VISIBLE,
        0,
        "no ring after a click"
    );
    let evs = w.events();
    assert_eq!(
        kinds(&evs),
        vec![(ev::FOCUS, 1), (ev::CLICK, 1)],
        "one receiver per event: the nearest ancestor that asked"
    );
    let click = &evs[1].2;
    assert_eq!(
        u32::from_le_bytes(click[0..4].try_into().unwrap()),
        BUTTON_LEFT
    );
}

#[test]
fn press_then_release_elsewhere_is_not_a_click() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Button, 0, [0.0, 0.0, 50.0, 20.0]);
        p.bind_event(1, event::bit(ev::CLICK));
        place(p, 2, Kind::Button, 0, [100.0, 0.0, 50.0, 20.0]);
        p.bind_event(2, event::bit(ev::CLICK));
    });
    w.ptr(POINTER_DOWN, 10.0, 10.0, BUTTON_LEFT);
    assert_ne!(w.state(1) & state::ACTIVE, 0);
    w.ptr(POINTER_MOVE, 120.0, 10.0, 0);
    assert_ne!(
        w.state(1) & state::HOVER,
        0,
        "hover stays with the pressed part"
    );
    assert_eq!(w.state(2) & state::HOVER, 0);
    w.ptr(POINTER_UP, 120.0, 10.0, BUTTON_LEFT);
    assert_eq!(w.state(1) & state::ACTIVE, 0);
    assert!(!kinds(&w.events()).iter().any(|e| e.0 == ev::CLICK));
    assert_ne!(
        w.state(2) & state::HOVER,
        0,
        "hover follows again after release"
    );
}

#[test]
fn slider_drag_is_captured_outside_its_rectangle() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Slider, 0, [100.0, 100.0, 100.0, 20.0]);
        p.set_f32(1, id::SLIDER_MAX, 10.0);
        p.set_f32(1, id::SLIDER_STEP, 1.0);
        p.bind_event(1, event::bit(ev::SLIDER));
    });
    let value = |w: &mut Win| -> Vec<f32> {
        w.events()
            .iter()
            .filter(|e| e.0 == ev::SLIDER)
            .map(|e| f32::from_le_bytes(e.2[0..4].try_into().unwrap()))
            .collect()
    };
    w.ptr(POINTER_DOWN, 150.0, 110.0, BUTTON_LEFT);
    assert_eq!(value(&mut w), vec![5.0]);
    w.ptr(POINTER_MOVE, 380.0, 250.0, 0);
    assert_eq!(
        value(&mut w),
        vec![10.0],
        "past the end: clamped at the maximum"
    );
    w.ptr(POINTER_MOVE, 0.0, 250.0, 0);
    w.ptr(POINTER_UP, 0.0, 250.0, BUTTON_LEFT);
    assert_eq!(value(&mut w), vec![0.0]);
}

#[test]
fn modal_layer_traps_focus_and_gives_it_back() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Button, 0, [0.0, 0.0, 50.0, 20.0]);
        place(p, 2, Kind::Button, 0, [60.0, 0.0, 50.0, 20.0]);
    });
    w.key(sym::TAB, 0);
    w.key(sym::TAB, 0);
    assert_eq!(w.focused(), 2);
    w.apply(|p| {
        place(p, 10, Kind::Overlay, 0, [100.0, 100.0, 200.0, 100.0]);
        p.set_prop(10, id::MODAL, &[1]);
        p.bind_event(10, event::bit(ev::CLICK));
        place(p, 11, Kind::Button, 10, [110.0, 110.0, 50.0, 20.0]);
        place(p, 12, Kind::Button, 10, [170.0, 110.0, 50.0, 20.0]);
    });
    assert_eq!(w.focused(), 11, "an opened modal layer takes focus");
    w.key(sym::TAB, 0);
    w.key(sym::TAB, 0);
    assert_eq!(w.focused(), 11, "tab cycles inside the layer");
    w.events();
    w.click(5.0, 5.0);
    assert_eq!(
        w.focused(),
        11,
        "a click outside cannot move focus behind the layer"
    );
    assert_eq!(
        kinds(&w.events()),
        vec![(ev::CLICK, 10)],
        "the layer absorbs it"
    );
    w.apply(|p| {
        p.free(10);
    });
    assert_eq!(w.focused(), 2, "closing the layer gives focus back");
    assert_ne!(w.state(2) & state::FOCUS, 0);
}

#[test]
fn escape_asks_the_innermost_overlay_to_close() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Overlay, 0, [10.0, 10.0, 100.0, 100.0]);
        p.bind_event(1, event::bit(ev::CLOSE_REQUEST));
        place(p, 2, Kind::Button, 1, [20.0, 20.0, 50.0, 20.0]);
        p.focus(2);
    });
    w.events();
    w.key(sym::ESCAPE, 0);
    assert_eq!(kinds(&w.events()), vec![(ev::CLOSE_REQUEST, 1)]);
}

#[test]
fn arrows_home_and_end_move_between_list_items() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Box, 0, [0.0, 0.0, 200.0, 200.0]);
        p.set_a11y(1, 9, "Fruit", "");
        for (i, n) in [2u32, 3, 4].iter().enumerate() {
            let y = i as f32 * 30.0;
            place(p, *n, Kind::Button, 1, [0.0, y, 100.0, 20.0]);
        }
        p.focus(2);
    });
    w.key(sym::DOWN, 0);
    assert_eq!(w.focused(), 3);
    w.key(sym::DOWN, 0);
    w.key(sym::DOWN, 0);
    assert_eq!(w.focused(), 4, "the last item stays");
    w.key(sym::HOME, 0);
    assert_eq!(w.focused(), 2);
    w.key(sym::END, 0);
    assert_eq!(w.focused(), 4);
    w.key(sym::UP, 0);
    assert_eq!(w.focused(), 3);
    assert_ne!(w.state(3) & state::FOCUS_VISIBLE, 0);
}

#[test]
fn slider_keys_step_page_and_clamp() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Slider, 0, [0.0, 0.0, 100.0, 20.0]);
        p.set_f32(1, id::SLIDER_MAX, 100.0);
        p.set_f32(1, id::SLIDER_STEP, 5.0);
        p.focus(1);
    });
    let v = |w: &Win| {
        w.core
            .scene
            .prop(1, id::SLIDER_VALUE)
            .map_or(0.0, |v| v.f32())
    };
    w.key(sym::RIGHT, 0);
    assert_eq!(v(&w), 5.0);
    w.key(sym::PAGE_UP, 0);
    assert_eq!(v(&w), 15.0);
    w.key(sym::END, 0);
    assert_eq!(v(&w), 100.0);
    w.key(sym::RIGHT, 0);
    assert_eq!(v(&w), 100.0, "clamped");
    w.key(sym::HOME, 0);
    assert_eq!(v(&w), 0.0);
}

#[test]
fn enter_and_space_activate_buttons_and_toggles() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Button, 0, [0.0, 0.0, 50.0, 20.0]);
        p.bind_event(1, event::bit(ev::CLICK));
        place(p, 2, Kind::Toggle, 0, [60.0, 0.0, 50.0, 20.0]);
        p.bind_event(2, event::bit(ev::TOGGLE));
        p.focus(1);
    });
    w.events();
    w.key(sym::RETURN, 0);
    w.key(sym::TAB, 0);
    w.key(sym::SPACE, 0);
    let evs = w.events();
    let k = kinds(&evs);
    assert!(k.contains(&(ev::CLICK, 1)));
    assert!(k.contains(&(ev::TOGGLE, 2)));
    assert_ne!(w.state(2) & state::CHECKED, 0);
}

#[test]
fn shortcuts_use_the_view_shortcut_names() {
    assert_eq!(
        keys::shortcut_name(0x73, MOD_CTRL | MOD_SHIFT).as_deref(),
        Some("cmd+shift+s")
    );
    assert_eq!(
        keys::shortcut_name(0x53, MOD_CTRL | MOD_SHIFT).as_deref(),
        Some("cmd+shift+s")
    );
    assert_eq!(keys::shortcut_name(sym::F1 + 4, 0).as_deref(), Some("f5"));
    assert_eq!(
        keys::shortcut_name(0x73, 0),
        None,
        "a plain letter is typing"
    );
    let mut w = Win::new();
    w.apply(|p| {
        p.bind_event(0, event::bit(ev::SHORTCUT));
    });
    w.key(0x73, MOD_CTRL);
    let evs = w.events();
    let sc: Vec<_> = evs.iter().filter(|e| e.0 == ev::SHORTCUT).collect();
    assert_eq!(sc.len(), 1);
    assert_eq!(&sc[0].2[8..12], &5u32.to_le_bytes());
    assert_eq!(&sc[0].2[12..17], b"cmd+s");
}

#[test]
fn scrolled_content_is_hit_where_it_is_drawn() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Scroll, 0, [0.0, 0.0, 200.0, 100.0]);
        p.set_prop(1, id::OVERFLOW, &[2]);
        p.bind_event(1, event::bit(ev::SCROLL));
        place(p, 2, Kind::Button, 1, [0.0, 150.0, 100.0, 20.0]);
        p.bind_event(2, event::bit(ev::CLICK));
    });
    w.ptr(POINTER_MOVE, 10.0, 50.0, 0);
    w.ptr(POINTER_SCROLL, 0.0, 100.0, 0);
    w.events();
    // The button's layout y is 150; scrolled by 100 it is drawn at 50.
    w.click(10.0, 55.0);
    assert!(kinds(&w.events()).contains(&(ev::CLICK, 2)));
}

#[test]
fn programmatic_focus_gets_its_state_and_event() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Button, 0, [0.0, 0.0, 50.0, 20.0]);
        p.bind_event(1, event::bit(ev::FOCUS));
    });
    w.events();
    w.apply(|p| {
        p.focus(1);
    });
    assert_ne!(w.state(1) & state::FOCUS, 0);
    assert_eq!(kinds(&w.events()), vec![(ev::FOCUS, 1)]);
    w.apply(|p| {
        p.set_state(1, state::DISABLED);
    });
    assert_eq!(w.focused(), NIL, "a disabled part lets go of focus");
}

#[test]
fn secondary_button_opens_the_context_menu() {
    let mut w = Win::new();
    w.apply(|p| {
        place(p, 1, Kind::Box, 0, [0.0, 0.0, 100.0, 100.0]);
        p.bind_event(1, event::bit(ev::CONTEXT_MENU) | event::bit(ev::CLICK));
    });
    w.events();
    w.ptr(POINTER_DOWN, 30.0, 40.0, BUTTON_RIGHT);
    w.ptr(POINTER_UP, 30.0, 40.0, BUTTON_RIGHT);
    let evs = w.events();
    assert_eq!(kinds(&evs), vec![(ev::CONTEXT_MENU, 1)]);
    assert_eq!(f32::from_le_bytes(evs[0].2[0..4].try_into().unwrap()), 30.0);
}

#[test]
fn coalescer_keeps_the_last_move_and_sums_scroll_in_order() {
    let mv = |x: f32| PointerInput {
        kind: POINTER_MOVE,
        x,
        y: 0.0,
        button: 0,
        mods: 0,
    };
    let sc = |y: f32| PointerInput {
        kind: POINTER_SCROLL,
        x: 0.0,
        y,
        button: 0,
        mods: 0,
    };
    let down = PointerInput {
        kind: POINTER_DOWN,
        x: 9.0,
        y: 0.0,
        button: 1,
        mods: 0,
    };
    let mut c = Coalescer::default();
    let mut out = Vec::new();
    for x in 0..50 {
        c.push(mv(x as f32), &mut out);
    }
    assert!(out.is_empty(), "moves wait for the frame");
    c.push(sc(3.0), &mut out);
    c.push(sc(4.0), &mut out);
    c.push(down, &mut out);
    assert_eq!(
        out,
        vec![mv(49.0), sc(7.0), down],
        "a press flushes first, in order"
    );
    out.clear();
    c.push(sc(1.0), &mut out);
    c.push(mv(5.0), &mut out);
    assert_eq!(
        out,
        vec![sc(1.0)],
        "a scroll is not moved after a later move"
    );
    c.flush(&mut out);
    assert_eq!(out, vec![sc(1.0), mv(5.0)]);
    assert_eq!((c.taken, c.given), (55, 5));
}
