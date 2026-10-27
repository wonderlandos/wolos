//! Force Quit Applications (⌥⌘Esc, macOS): open apps, most recent first; Force Quit ends every
//! window of the chosen one (`killwindow`, same as the app menu's Force Quit). Kit only.
use crate::appmenu::{self, Act};
use crate::hypr::Client;
use crate::kit::widgets as w;
use crate::switcher::App;
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const TITLE: &str = "Force Quit Applications";
pub const HINT: &str = "If an app doesn't respond for a while, choose it and click Force Quit.";
pub const EMPTY: &str = "No apps are open.";
/// Apps listed before the list scrolls.
const ROWS: usize = 6;
/// Time for the windows to go before the list is read again.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// Hyprland request ending every window of `app` (None when it has no window left).
pub fn batch(app: &App, clients: &[Client]) -> Option<String> {
    let front = clients.iter().find(|c| c.class == app.class)?;
    appmenu::batch(Act::ForceQuit, front, clients, (0, 0, 0, 0))
}

pub struct Picker {
    pub root: gtk::Box,
    list: gtk::Box,
    scroll: gtk::ScrolledWindow,
    empty: gtk::Label,
    button: gtk::Button,
    rows: RefCell<Vec<(gtk::Button, App)>>,
    sel: Cell<usize>,
}

impl Picker {
    /// `quit` runs with the picker and the chosen app (button, Return).
    pub fn new(quit: impl Fn(&Rc<Picker>, &App) + 'static) -> Rc<Self> {
        let root = w::glass_panel(w::S3);
        root.add_css_class("force-quit");
        root.set_width_request(w::FORCE_QUIT.0);
        root.append(&w::label(TITLE, "title"));
        let hint = w::label(HINT, "secondary");
        hint.set_wrap(true);
        hint.set_max_width_chars(1);
        hint.set_hexpand(true);
        root.append(&hint);
        let list = w::vbox(0);
        list.add_css_class("force-quit-list");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        root.append(&scroll);
        let empty = w::centered(EMPTY, "secondary");
        empty.set_visible(false);
        list.append(&empty);
        let quit = Rc::new(quit);
        let p = Rc::new_cyclic(|me: &std::rc::Weak<Picker>| {
            let me = me.clone();
            let q = quit.clone();
            let button = w::button("Force Quit", w::Kind::Destructive, move || {
                if let Some(p) = me.upgrade()
                    && let Some(a) = p.selected()
                {
                    q(&p, &a);
                }
            });
            button.set_halign(gtk::Align::End);
            Picker {
                root: root.clone(),
                list,
                scroll,
                empty,
                button,
                rows: Default::default(),
                sel: Cell::new(0),
            }
        });
        p.root.append(&p.button);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let me = Rc::downgrade(&p);
            keys.connect_key_pressed(move |_, k, _, _| {
                let Some(p) = me.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let n = p.rows.borrow().len();
                match k {
                    gdk::Key::Down if n > 0 => p.select((p.sel.get() + 1).min(n - 1)),
                    gdk::Key::Up if n > 0 => p.select(p.sel.get().saturating_sub(1)),
                    gdk::Key::Return | gdk::Key::KP_Enter => p.button.emit_clicked(),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
        }
        p.root.add_controller(keys);
        p
    }

    /// One row per app; the first (frontmost) is chosen.
    pub fn set_apps(self: &Rc<Self>, apps: &[App]) {
        for (b, _) in self.rows.borrow_mut().drain(..) {
            self.list.remove(&b);
        }
        for (i, a) in apps.iter().enumerate() {
            let row = w::hbox(w::S2);
            row.append(&w::app_icon(&a.icon, w::ICON_M));
            let name = w::label(&a.name, "");
            name.set_hexpand(true);
            name.add_css_class("may-ellipsize");
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_max_width_chars(1);
            row.append(&name);
            let b = gtk::Button::builder().child(&row).build();
            b.add_css_class("hit");
            // Arrows choose, Return and the button quit: the rows never take focus.
            b.set_focusable(false);
            let me = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(p) = me.upgrade() {
                    p.select(i);
                }
            });
            self.list.append(&b);
            self.rows.borrow_mut().push((b, a.clone()));
        }
        // A short list shows whole; a long one scrolls inside a fixed height.
        let long = apps.len() > ROWS;
        self.scroll.set_vscrollbar_policy(if long {
            gtk::PolicyType::Automatic
        } else {
            gtk::PolicyType::Never
        });
        self.scroll
            .set_min_content_height(if long { w::FORCE_QUIT.1 } else { -1 });
        self.empty.set_visible(apps.is_empty());
        self.button.set_sensitive(!apps.is_empty());
        self.select(0);
    }

    pub fn select(&self, i: usize) {
        self.sel.set(i);
        for (n, (b, _)) in self.rows.borrow().iter().enumerate() {
            if n == i {
                b.add_css_class("selected");
            } else {
                b.remove_css_class("selected");
            }
        }
    }

    pub fn selected(&self) -> Option<App> {
        self.rows.borrow().get(self.sel.get()).map(|r| r.1.clone())
    }
}

/// Resident panel: the list is read each time it opens; Esc closes it.
pub fn build(app: &gtk::Application) -> gtk::Window {
    let refresh = |p: &Rc<Picker>| {
        let installed = crate::desktop::load_all();
        p.set_apps(&crate::switcher::apps(&crate::hypr::clients(), &installed));
    };
    let picker = Picker::new(move |p, a| {
        let clients = crate::hypr::clients();
        if let Some(b) = batch(a, &clients) {
            crate::hypr::request(&b);
        }
        let p = p.clone();
        glib::timeout_add_local_once(SETTLE, move || refresh(&p));
    });
    let win = super::panels::panel(
        app,
        "force-quit",
        "wol-panel-force-quit",
        [false; 4],
        &picker.root,
    );
    win.connect_show(move |_| {
        refresh(&picker);
        picker.button.grab_focus();
    });
    win
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_quit_ends_every_window_of_the_chosen_app() {
        let c = |a: &str, class: &str| Client::new(a, class, "");
        let clients = [c("0x1", "foot"), c("0x2", "firefox"), c("0x3", "foot")];
        let app = |class: &str| App {
            class: class.into(),
            name: class.into(),
            icon: String::new(),
            address: String::new(),
        };
        assert_eq!(
            batch(&app("foot"), &clients).as_deref(),
            Some("[[BATCH]]dispatch killwindow address:0x1;dispatch killwindow address:0x3")
        );
        assert_eq!(batch(&app("gimp"), &clients), None, "already gone");
    }
}
