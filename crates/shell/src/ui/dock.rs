//! Dock: pinned apps, running dots from Hyprland clients, separator, Trash.
use crate::apps::{self, PINS};
use crate::kit::{self, surface::Spec, widgets as w};
use gtk::prelude::*;
use std::collections::HashSet;
use std::rc::Rc;

pub struct Dock {
    pub root: gtk::Box,
    tiles: Vec<w::DockTile>,
}

pub fn content(running: &HashSet<String>) -> Rc<Dock> {
    let root = w::hbox(w::S2);
    root.add_css_class("dock-shelf");
    root.add_css_class("no-row-check"); // tiles lift on hover by design
    root.set_halign(gtk::Align::Center);
    root.set_valign(gtk::Align::End);
    let tiles: Vec<w::DockTile> = PINS
        .iter()
        .map(|p| {
            let argv: Vec<String> = p.exec.iter().map(|s| s.to_string()).collect();
            w::DockTile::new(p.icon, p.name, move || kit::spawn(&argv))
        })
        .collect();
    for t in &tiles {
        root.append(&t.button);
    }
    root.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    root.append(
        &w::DockTile::new("wolos-trash", "Trash", || {
            kit::spawn(
                &apps::TRASH
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>(),
            )
        })
        .button,
    );
    let d = Rc::new(Dock { root, tiles });
    d.set_running(running);
    d
}

/// The Dock window, its tiles, and its keyboard access (⌃F2/⌃F3 style, see [`keyboard`]).
pub type Built = (gtk::ApplicationWindow, Rc<Dock>, Rc<dyn Fn(bool)>);

pub fn build(app: &gtk::Application) -> Built {
    let win = super::window(app, "dock", Spec::dock());
    let d = content(&crate::hypr::running_classes(&crate::hypr::clients()));
    // Autohide: the shelf slides away and a thin hot zone at the bottom edge brings it back.
    let reveal = gtk::Revealer::builder()
        .child(&d.root)
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(true)
        .build();
    let zone = w::vbox(0);
    zone.add_css_class("dock-hotzone");
    zone.append(&reveal);
    let hide = Rc::new(std::cell::Cell::new(false));
    let motion = gtk::EventControllerMotion::new();
    {
        let (r, h) = (reveal.clone(), hide.clone());
        let r2 = r.clone();
        motion.connect_enter(move |_, _, _| r2.set_reveal_child(true));
        motion.connect_leave(move |_| {
            if h.get() {
                r.set_reveal_child(false)
            }
        });
    }
    zone.add_controller(motion);
    win.set_child(Some(&zone));
    let keys = keyboard(&win, &d, &reveal, &hide);
    {
        let (root, win) = (d.root.clone(), win.clone());
        super::ipc::on_reload(move |p| {
            hide.set(p.dock_autohide);
            reveal.set_reveal_child(!p.dock_autohide);
            kit::surface::set_exclusive(&win, !p.dock_autohide);
            set_magnify(&root, p.dock_magnify);
        });
    }
    (win, d, keys)
}

/// ⌃F3 (`wol shell show dock`): the Dock shows and takes the keyboard, the first app focused;
/// arrows move, Return opens the app and gives the keyboard back, as does Esc.
fn keyboard(
    win: &gtk::ApplicationWindow,
    d: &Dock,
    reveal: &gtk::Revealer,
    hide: &Rc<std::cell::Cell<bool>>,
) -> Rc<dyn Fn(bool)> {
    let Some(first) = d.tiles.first() else {
        return Rc::new(|_| {});
    };
    let (reveal, hide) = (reveal.clone(), hide.clone());
    // Autohide: shown while it has the keyboard.
    let keys = kit::surface::keyboard_access(win, &first.button, move |on| {
        reveal.set_reveal_child(on || !hide.get())
    });
    let mut c = d.root.first_child();
    while let Some(w) = c {
        if let Some(b) = w.downcast_ref::<gtk::Button>() {
            let k = keys.clone();
            b.connect_clicked(move |_| k(false));
        }
        c = w.next_sibling();
    }
    keys
}

/// Settings > Dock > Magnification: tiles lift on hover only when on.
pub fn set_magnify(root: &gtk::Box, on: bool) {
    if on {
        root.remove_css_class("flat")
    } else {
        root.add_css_class("flat")
    }
}

impl Dock {
    pub fn set_running(&self, classes: &HashSet<String>) {
        for (p, t) in PINS.iter().zip(&self.tiles) {
            t.set_running(apps::is_running(p, classes));
        }
    }
}
