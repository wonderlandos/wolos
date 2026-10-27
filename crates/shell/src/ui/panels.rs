//! Wallpaper, OSD, clipboard, session menu, window switcher.
use crate::kit::{
    self,
    surface::{Keyboard, Spec},
    widgets as w,
};
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Seconds between two looks at the clock for the Dynamic wallpaper.
const DYNAMIC_TICK: u32 = 60;

/// The desktop picture: the saved choice (`wol wallpaper set`, Settings > Wallpaper), followed
/// live through the shell reload and the choice file; Dynamic follows the clock.
pub fn wallpaper(app: &gtk::Application) -> gtk::ApplicationWindow {
    let win = super::window(app, "wallpaper", Spec::wallpaper());
    let pic = gtk::Picture::new();
    pic.set_content_fit(gtk::ContentFit::Cover);
    win.set_child(Some(&pic));
    let shown: RefCell<Option<std::path::PathBuf>> = Default::default();
    let show = Rc::new(move || {
        let now = crate::kit::pickers::desktop_wallpaper();
        if *shown.borrow() == now {
            return;
        }
        shown.replace(now.clone());
        let Some(p) = now else { return };
        // Decoded off the main thread: the CSS gradient shows until the picture is ready.
        let pic = pic.clone();
        glib::spawn_future_local(async move {
            if let Ok(Ok(t)) =
                gtk::gio::spawn_blocking(move || gdk::Texture::from_filename(p)).await
            {
                pic.set_paintable(Some(&t));
            }
        });
    });
    {
        let s = show.clone();
        super::ipc::on_reload(move |_| s());
    }
    let choice = gtk::gio::File::for_path(
        std::path::Path::new(&crate::kit::config_home()).join(shared::theme::WALLPAPER_FILE),
    );
    if let Ok(m) = choice.monitor_file(
        gtk::gio::FileMonitorFlags::NONE,
        gtk::gio::Cancellable::NONE,
    ) {
        let s = show.clone();
        m.connect_changed(move |_, _, _, _| s());
        // The monitor lives as long as the window.
        win.connect_destroy(move |_| {
            m.cancel();
        });
    }
    glib::timeout_add_seconds_local(DYNAMIC_TICK, move || {
        show();
        glib::ControlFlow::Continue
    });
    win
}

/// Panel window: transparent overlay layer, glass content, Esc hides.
pub(super) fn panel(
    app: &gtk::Application,
    class: &str,
    ns: &'static str,
    anchor: [bool; 4],
    child: &impl IsA<gtk::Widget>,
) -> gtk::Window {
    let win = super::window(app, "panel", Spec::panel(ns, anchor, Keyboard::OnDemand));
    win.add_css_class(class);
    win.set_child(Some(child));
    let keys = gtk::EventControllerKey::new();
    let w2 = win.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gdk::Key::Escape {
            w2.set_visible(false);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    win.add_controller(keys);
    win.upcast()
}

/// About <App> card: close light, 64 px icon, name, app id. Glass, centred (HIG panel).
pub fn about_card(name: &str, icon: &str, class: &str) -> gtk::Box {
    let g = w::glass_panel(w::S6);
    g.add_css_class("about-app");
    g.set_size_request(w::ABOUT_WIDTH, -1);
    let close = gtk::Button::new();
    for c in ["light", "close"] {
        close.add_css_class(c);
    }
    close.set_tooltip_text(Some("Close"));
    close.set_halign(gtk::Align::Start);
    close.connect_clicked(|b| {
        if let Some(win) = b.root().and_downcast::<gtk::Window>() {
            win.close();
        }
    });
    let lights = w::hbox(0);
    lights.add_css_class("traffic-lights");
    lights.append(&close);
    g.append(&lights);
    let i = w::icon(icon, w::ICON_XL);
    i.set_halign(gtk::Align::Center);
    g.append(&i);
    g.append(&w::centered(name, "title"));
    g.append(&w::centered(class, "secondary"));
    g
}

/// Shell-drawn About panel: the frontmost app keeps focus and its name in the menu bar.
pub fn about(app: &gtk::Application, name: &str, icon: &str, class: &str) -> gtk::Window {
    let win = panel(
        app,
        "about",
        "wol-panel-about",
        [false; 4],
        &about_card(name, icon, class),
    );
    win.connect_hide(|w| w.destroy());
    win
}

pub struct Osd {
    pub window: gtk::Window,
    icon: gtk::Image,
    bar: gtk::ProgressBar,
    hide_gen: Cell<u32>,
}

impl Osd {
    pub fn build(app: &gtk::Application) -> Rc<Osd> {
        let g = w::glass_panel(w::S2);
        g.add_css_class("osd-pill");
        let row = w::hbox(w::S3);
        let icon = w::icon("audio-volume-high-symbolic", w::ICON);
        let bar = gtk::ProgressBar::new();
        bar.set_hexpand(true);
        bar.set_valign(gtk::Align::Center);
        row.append(&icon);
        row.append(&bar);
        g.append(&row);
        let window = panel(app, "osd", "wol-panel-osd", [false, true, false, false], &g);
        Rc::new(Osd {
            window,
            icon,
            bar,
            hide_gen: Cell::new(0),
        })
    }

    /// Shows for 1.5 s; a newer show extends it.
    pub fn show(self: &Rc<Self>, icon: &str, level: u8) {
        self.icon.set_icon_name(Some(icon));
        self.bar.set_fraction(level as f64 / 100.0);
        self.window.set_visible(true);
        let seq = self.hide_gen.get() + 1;
        self.hide_gen.set(seq);
        let me = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            if me.hide_gen.get() == seq {
                me.window.set_visible(false);
            }
        });
    }
}

/// Clipboard history: watches the display clipboard (event-driven), click copies back.
pub fn clipboard(app: &gtk::Application) -> gtk::Window {
    let g = w::glass_panel(w::S2);
    let head = w::hbox(w::S2);
    head.append(&w::label("Clipboard", "title"));
    head.append(&w::spacer());
    let list = w::vbox(w::S1);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let history: Rc<RefCell<Vec<String>>> = Default::default();
    let render = {
        let (list, history) = (list.clone(), history.clone());
        Rc::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let h = history.borrow();
            if h.is_empty() {
                list.append(&w::label("Copied text shows up here", "secondary"));
            }
            for e in h.iter() {
                let row = w::list_button("edit-paste-symbolic", &crate::clipboard::preview(e), "");
                let e = e.clone();
                row.connect_clicked(move |b| b.display().clipboard().set_text(&e));
                list.append(&row);
            }
        })
    };
    let (h2, r2) = (history.clone(), render.clone());
    head.append(&w::button("Clear", w::Kind::Default, move || {
        h2.borrow_mut().clear();
        r2();
    }));
    g.append(&head);
    g.append(&scroll);
    render();
    let cb = gdk::Display::default().expect("display").clipboard();
    cb.connect_changed(move |cb| {
        let (history, render) = (history.clone(), render.clone());
        cb.read_text_async(gio_none(), move |r| {
            if let Ok(Some(t)) = r
                && crate::clipboard::push(&mut history.borrow_mut(), &t)
            {
                render();
            }
        });
    });
    panel(app, "clipboard", "wol-panel-clipboard", [false; 4], &g)
}

fn gio_none() -> Option<&'static gtk::gio::Cancellable> {
    None
}

/// Restart / Shut Down / Log Out ask first (macOS): the question, a 60 s countdown, Cancel and
/// the action. Doing nothing runs it; Cancel, Esc or closing the panel drops it.
struct Confirm {
    view: gtk::Box,
    question: gtk::Label,
    line: gtk::Label,
    go: gtk::Button,
    action: Cell<&'static str>,
    left: Cell<u32>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Confirm {
    fn new(row: &gtk::Box) -> Rc<Confirm> {
        let view = w::vbox(w::S2);
        let question = w::label("", "title");
        let line = w::label("", "secondary");
        let buttons = w::hbox(w::S2);
        buttons.set_halign(gtk::Align::End);
        let go = w::button("", w::Kind::Primary, || {});
        let c = Rc::new(Confirm {
            view,
            question,
            line,
            go,
            action: Cell::new(""),
            left: Cell::new(0),
            timer: RefCell::new(None),
        });
        let (c2, r2) = (Rc::downgrade(&c), row.clone());
        buttons.append(&w::button("Cancel", w::Kind::Default, move || {
            if let Some(c) = c2.upgrade() {
                c.stop();
                c.view.set_visible(false);
                r2.set_visible(true);
            }
        }));
        let c3 = Rc::downgrade(&c);
        c.go.connect_clicked(move |_| {
            if let Some(c) = c3.upgrade() {
                c.run();
            }
        });
        buttons.append(&c.go);
        c.view.append(&c.question);
        c.view.append(&c.line);
        c.view.append(&buttons);
        c.view.set_visible(false);
        c
    }

    fn show(self: &Rc<Self>, action: &'static str, row: &gtk::Box, countdown: bool) {
        self.stop();
        self.action.set(action);
        self.left.set(crate::clipboard::CONFIRM_SECS);
        self.render();
        row.set_visible(false);
        self.view.set_visible(true);
        self.go.grab_focus();
        if !countdown {
            return;
        }
        let c = Rc::downgrade(self);
        let id = glib::timeout_add_seconds_local(1, move || {
            let Some(c) = c.upgrade() else {
                return glib::ControlFlow::Break;
            };
            c.left.set(c.left.get().saturating_sub(1));
            if c.left.get() == 0 {
                c.timer.take();
                c.run();
                return glib::ControlFlow::Break;
            }
            c.render();
            glib::ControlFlow::Continue
        });
        self.timer.replace(Some(id));
    }

    fn render(&self) {
        if let Some((q, line, button)) =
            crate::copy::session_confirm(self.action.get(), self.left.get())
        {
            self.question.set_label(q);
            self.line.set_label(&line);
            self.go.set_label(button);
        }
    }

    fn stop(&self) {
        if let Some(id) = self.timer.take() {
            id.remove();
        }
    }

    fn run(&self) {
        self.stop();
        if let Some(win) = self.view.root().and_downcast::<gtk::Window>() {
            win.set_visible(false);
        }
        if let Some(argv) = crate::clipboard::session_command(self.action.get()) {
            kit::spawn(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        }
    }
}

/// The session menu: Lock, Sleep, Restart, Shut Down, Log Out, and the confirm view the last
/// three open.
pub struct SessionMenu {
    pub root: gtk::Box,
    row: gtk::Box,
    confirm: Rc<Confirm>,
}

impl SessionMenu {
    pub fn new() -> SessionMenu {
        let root = w::glass_panel(w::S2);
        let row = w::hbox(w::S2);
        let confirm = Confirm::new(&row);
        for (icon, label, action) in [
            ("system-lock-screen-symbolic", "Lock", "lock"),
            ("weather-clear-night-symbolic", "Sleep", "sleep"),
            ("system-reboot-symbolic", "Restart", "restart"),
            ("system-shutdown-symbolic", "Shut Down", "shutdown"),
            ("system-log-out-symbolic", "Log Out", "logout"),
        ] {
            let b = w::vbox(w::S1);
            let i = w::icon(icon, w::ICON_M);
            i.set_halign(gtk::Align::Center);
            b.append(&i);
            b.append(&w::centered(label, ""));
            let btn = gtk::Button::builder().child(&b).build();
            btn.add_css_class("session-item");
            let (c, r) = (confirm.clone(), row.clone());
            btn.connect_clicked(move |b| {
                if crate::clipboard::needs_confirm(action) {
                    c.show(action, &r, true);
                    return;
                }
                if let Some(win) = b.root().and_downcast::<gtk::Window>() {
                    win.set_visible(false);
                }
                if let Some(argv) = crate::clipboard::session_command(action) {
                    kit::spawn(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>());
                }
            });
            row.append(&btn);
        }
        root.append(&row);
        root.append(&confirm.view);
        SessionMenu { root, row, confirm }
    }

    /// Ask before `action` (restart, shutdown, logout); the countdown starts.
    pub fn ask(&self, action: &'static str) {
        self.confirm.show(action, &self.row, true);
    }

    /// The question as it first shows, without a countdown running (goldens).
    pub fn preview(&self, action: &'static str) {
        self.confirm.show(action, &self.row, false);
    }

    /// Back to the five actions; any countdown stops.
    pub fn reset(&self) {
        self.confirm.stop();
        self.confirm.view.set_visible(false);
        self.row.set_visible(true);
    }
}

impl Default for SessionMenu {
    fn default() -> Self {
        Self::new()
    }
}

pub fn session(app: &gtk::Application) -> gtk::Window {
    let menu = SessionMenu::new();
    let win = panel(app, "session", "wol-panel-session", [false; 4], &menu.root);
    // Hidden (Esc, toggled away, clicked outside): the countdown stops and the icons come back.
    win.connect_visible_notify(move |win| {
        if !win.is_visible() {
            menu.reset();
        }
    });
    win
}

#[cfg(test)]
mod tests {
    /// `wol wallpaper set` and Settings save the choice; the desktop showed one hard-coded picture.
    #[test]
    fn desktop_follows_the_saved_wallpaper() {
        let src = include_str!("panels.rs");
        let body =
            &src[src.find("pub fn wallpaper(").unwrap()..src.find("/// Panel window").unwrap()];
        for want in [
            "pickers::desktop_wallpaper()",
            "ipc::on_reload",
            "monitor_file",
        ] {
            assert!(body.contains(want), "{want}");
        }
        assert!(
            !src.contains(concat!("wonderland-dusk", ".svg")),
            "a hard-coded picture"
        );
    }
}
