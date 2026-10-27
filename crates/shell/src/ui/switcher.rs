//! App switcher: glass strip of 64 px app icons, accent ring on the selection, its name below.
//! Super+Tab opens/advances, Super+Shift+Tab goes back, releasing Super focuses (see `switcher::op`).
use crate::kit::{surface::Spec, widgets as w};
use crate::switcher::{self, App, Cycle, Op};
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The strip, also used by the goldens.
/// Unmap round trip before focusing (Hyprland refocuses on layer unmap).
const FOCUS_AFTER_UNMAP: std::time::Duration = std::time::Duration::from_millis(60);

pub struct Strip {
    pub root: gtk::Box,
    tiles: Vec<(gtk::Box, gtk::Label)>,
}

impl Strip {
    pub fn new(apps: &[App], selected: usize) -> Self {
        let root = w::glass_panel(0);
        root.add_css_class("switcher-strip");
        let row = w::hbox(w::S2);
        root.append(&row);
        let tiles: Vec<(gtk::Box, gtk::Label)> = apps
            .iter()
            .map(|a| {
                let tile = w::vbox(w::S2);
                tile.add_css_class("switcher-tile");
                let ring = w::vbox(0);
                ring.add_css_class("switcher-ring");
                ring.set_halign(gtk::Align::Center);
                ring.append(&w::app_icon(&a.icon, w::ICON_XL));
                tile.append(&ring);
                let name = w::centered(&a.name, "switcher-name");
                name.add_css_class("may-ellipsize");
                name.set_max_width_chars(w::TILE_CHARS);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                tile.append(&name);
                row.append(&tile);
                (tile, name)
            })
            .collect();
        let s = Strip { root, tiles };
        s.select(selected);
        s
    }

    pub fn select(&self, index: usize) {
        for (i, (tile, name)) in self.tiles.iter().enumerate() {
            if i == index {
                tile.add_css_class("selected");
            } else {
                tile.remove_css_class("selected");
            }
            // Only the selection shows its name; the others keep their slot so nothing shifts.
            name.set_opacity(if i == index { 1.0 } else { 0.0 });
        }
    }
}

pub struct Switcher {
    pub window: gtk::Window,
    apps: RefCell<Vec<App>>,
    cycle: Cell<Cycle>,
    strip: RefCell<Option<Strip>>,
}

impl Switcher {
    pub fn build(app: &gtk::Application) -> Rc<Self> {
        let window = super::window(
            app,
            "panel",
            Spec::panel(
                "wol-panel-switcher",
                [false; 4],
                crate::kit::surface::Keyboard::Exclusive,
            ),
        );
        window.add_css_class("switcher");
        let s = Rc::new(Switcher {
            window: window.upcast(),
            apps: Default::default(),
            cycle: Cell::new(Cycle::open(0)),
            strip: Default::default(),
        });
        // Fallback when the compositor forwards keys instead of running the binds.
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&s);
        keys.connect_key_pressed(move |_, k, _, m| {
            let Some(s) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match k {
                gdk::Key::Escape => s.window.set_visible(false),
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab
                    if m.contains(gdk::ModifierType::SHIFT_MASK) || k == gdk::Key::ISO_Left_Tab =>
                {
                    s.run(Op::Prev)
                }
                gdk::Key::Tab => s.run(Op::Next),
                gdk::Key::Return => s.run(Op::Commit),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&s);
        keys.connect_key_released(move |_, k, _, _| {
            if matches!(k, gdk::Key::Super_L | gdk::Key::Super_R)
                && let Some(s) = weak.upgrade()
            {
                s.run(Op::Commit);
            }
        });
        s.window.add_controller(keys);
        s
    }

    /// Shell socket entry point; true when the command was the switcher's.
    pub fn command(&self, verb: crate::protocol::Verb, target: &str) -> bool {
        if !target.starts_with("switcher") {
            return false;
        }
        self.run(switcher::op(verb, target, self.window.is_visible()));
        true
    }

    fn run(&self, op: Op) {
        match op {
            Op::Open => {
                let reply = crate::hypr::request("j/clients");
                let Some(apps) = switcher::to_show(reply.as_deref(), &crate::desktop::load_all())
                else {
                    return; // no compositor answer or nothing open: never guess
                };
                let cycle = Cycle::open(apps.len());
                let strip = Strip::new(&apps, cycle.index);
                self.window.set_child(Some(&strip.root));
                *self.strip.borrow_mut() = Some(strip);
                *self.apps.borrow_mut() = apps;
                self.cycle.set(cycle);
                self.window.set_visible(true);
            }
            Op::Next | Op::Prev => {
                let c = self.cycle.get();
                let c = if op == Op::Next { c.next() } else { c.prev() };
                self.cycle.set(c);
                if let Some(s) = &*self.strip.borrow() {
                    s.select(c.index);
                }
            }
            Op::Commit => {
                let steps = switcher::commit(self.apps.borrow().get(self.cycle.get().index));
                for step in steps {
                    match step {
                        switcher::Step::Hide => self.window.set_visible(false),
                        // After the compositor has unmapped the layer (and refocused).
                        switcher::Step::Focus(arg) => {
                            glib::timeout_add_local_once(FOCUS_AFTER_UNMAP, move || {
                                let _ =
                                    crate::hypr::request(&format!("dispatch focuswindow {arg}"));
                            });
                        }
                    }
                }
            }
            Op::None => {}
        }
    }
}
