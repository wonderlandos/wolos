// SPDX-License-Identifier: BSD-2-Clause
//! The Todos window the accessibility tests share (`a11y_tree.rs` against the golden dump,
//! `a11y_atspi.rs` against the same dump read back over the bus), and the window harness.

use wolkit::a11y::{self, Context, roles};
use wolkit::env::Environment;
use wolkit::event::{self, EventQueue, kind as ev};
use wolkit::input::{InputState, edit, focus};
use wolkit::scene::patch::{APPEND, Patch, Writer};
use wolkit::scene::prop::{DIM_PX, id};
use wolkit::scene::{Kind, state};
use wolkit::window::{Sink, WindowCore};

pub fn px(v: f32) -> [u32; 2] {
    [DIM_PX, v.to_bits()]
}

pub fn place(p: &mut Writer, node: u32, kind: Kind, parent: u32, r: [f32; 4]) {
    p.create(node, kind).insert(parent, node, APPEND);
    p.set_prop(node, id::POSITION, &[1]);
    p.set_prop(node, id::INSET_LEFT, &px(r[0]));
    p.set_prop(node, id::INSET_TOP, &px(r[1]));
    p.set_prop(node, id::WIDTH, &px(r[2]));
    p.set_prop(node, id::HEIGHT, &px(r[3]));
}

pub struct Win {
    pub core: WindowCore,
    pub st: InputState,
    events: EventQueue,
    seq: u32,
}

impl Win {
    pub fn new(f: impl FnOnce(&mut Writer)) -> Win {
        let mut w = Win {
            core: WindowCore::new(1, "Todos", 320, 240, 0, Environment::default()),
            st: InputState::new(),
            events: EventQueue::new(),
            seq: 0,
        };
        w.apply(f);
        w
    }

    pub fn apply(&mut self, f: impl FnOnce(&mut Writer)) {
        let mut p = Writer::new();
        f(&mut p);
        self.seq += 1;
        let bytes = p.finish(self.seq).to_vec();
        let c = &mut self.core;
        let patch = Patch::parse(&bytes, &mut c.scratch).unwrap();
        c.scene.try_apply(&patch, &mut c.journal).unwrap();
        c.scene.commit(&mut c.journal);
        wolkit::window::pipeline::ensure_layout(c);
        let mut s = Sink {
            events: &mut self.events,
            window: 1,
            seq: self.seq,
        };
        focus::sync(c, &mut s, &mut self.st);
        edit::focus_changed(c, &mut self.st);
    }

    pub fn tree(&self) -> a11y::Tree {
        a11y::build(
            &self.core.scene,
            &Context {
                window: 1,
                title: &self.core.title,
                active: true,
                resizable: true,
                field: Some(&self.st.field),
            },
        )
    }
}

pub fn todos() -> Win {
    Win::new(|p| {
        place(p, 1, Kind::Box, 0, [0.0, 0.0, 320.0, 240.0]);
        place(p, 2, Kind::Text, 1, [16.0, 8.0, 200.0, 24.0]);
        p.set_text(2, "Todos");
        p.set_a11y(2, roles::wk::HEADING, "", "");
        place(p, 3, Kind::TextInput, 1, [16.0, 40.0, 200.0, 28.0]);
        p.set_a11y(3, 0, "New todo", "");
        p.set_text(3, "milk");
        place(p, 4, Kind::Button, 1, [224.0, 40.0, 80.0, 28.0]);
        place(p, 5, Kind::Text, 4, [232.0, 44.0, 64.0, 20.0]);
        p.set_text(5, "Add");
        p.bind_event(4, event::bit(ev::CLICK));
        place(p, 6, Kind::Box, 1, [16.0, 80.0, 288.0, 120.0]);
        p.set_a11y(6, roles::wk::LIST, "Todo list", "");
        place(p, 7, Kind::Toggle, 6, [16.0, 80.0, 288.0, 30.0]);
        p.set_text(7, "Buy bread");
        p.set_state(7, state::CHECKED);
        place(p, 8, Kind::Toggle, 6, [16.0, 110.0, 288.0, 30.0]);
        p.set_text(8, "Call Ana");
        let due = p.string("Due today");
        p.set_prop(8, id::TOOLTIP, &[due]);
        place(p, 9, Kind::Slider, 1, [16.0, 204.0, 200.0, 20.0]);
        p.set_a11y(9, 0, "Priority", "How urgent the new todo is");
        p.set_f32(9, id::SLIDER_MAX, 3.0);
        p.set_f32(9, id::SLIDER_STEP, 1.0);
        p.set_f32(9, id::SLIDER_VALUE, 2.0);
        place(p, 10, Kind::Text, 1, [0.0, 0.0, 10.0, 10.0]);
        p.set_text(10, "hidden");
        p.set_prop(10, id::DISPLAY, &[1]);
        place(p, 11, Kind::Button, 1, [224.0, 204.0, 80.0, 20.0]);
        p.set_a11y(11, 0, "Clear done", "");
        p.set_state(11, state::DISABLED);
        p.focus(3);
    })
}
