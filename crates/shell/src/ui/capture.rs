//! Screenshot toolbar (Super+Shift+5) and the floating thumbnail that follows a capture.
use crate::capture::{self, DELAYS, Dest, Mode, Options};
use crate::kit::{surface::Spec, widgets as w};
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

/// `wol shell show screen-record`: the toolbar opens in Record Entire Screen mode.
pub const RECORD: &str = "screen-record";

/// Seconds the thumbnail stays.
const THUMB_SECS: u32 = 5;

pub struct Toolbar {
    pub root: gtk::Box,
    pub mode: Rc<Cell<Mode>>,
    pub options: Rc<Cell<Options>>,
    pub capture: gtk::Button,
    pub close: gtk::Button,
    /// One toggle per mode, in `Mode::ALL` order.
    pub modes: Vec<(Mode, gtk::ToggleButton)>,
}

impl Toolbar {
    pub fn new(mode: Mode, options: Options) -> Self {
        let root = w::glass_panel(0);
        root.add_css_class("capture-bar");
        let row = w::hbox(w::S1);
        root.append(&row);
        let (mode_c, opts_c) = (Rc::new(Cell::new(mode)), Rc::new(Cell::new(options)));

        let close = w::icon_button("window-close-symbolic", "Close", || {});
        close.add_css_class("capture-close");
        row.append(&close);
        let capture = gtk::Button::with_label(capture::action(mode));
        capture.add_css_class("btn");
        capture.add_css_class("btn-primary");
        capture.add_css_class("capture-go");

        let mut first: Option<gtk::ToggleButton> = None;
        let mut modes = Vec::new();
        for m in Mode::ALL {
            if matches!(m, Mode::Screen | Mode::RecordScreen) {
                row.append(&gtk::Separator::new(gtk::Orientation::Vertical));
            }
            let b = gtk::ToggleButton::builder()
                .child(&w::icon(m.icon(), w::ICON_TOOL))
                .tooltip_text(m.label())
                .active(m == mode)
                .build();
            b.add_css_class("capture-mode");
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            let (mc, cap) = (mode_c.clone(), capture.clone());
            b.connect_toggled(move |b| {
                if b.is_active() {
                    mc.set(m);
                    cap.set_label(capture::action(m));
                }
            });
            row.append(&b);
            modes.push((m, b));
        }
        row.append(&gtk::Separator::new(gtk::Orientation::Vertical));

        let opts = gtk::MenuButton::builder()
            .label("Options")
            .popover(&options_menu(opts_c.clone()))
            .build();
        opts.add_css_class("capture-options");
        row.append(&opts);
        row.append(&capture);
        Toolbar {
            root,
            mode: mode_c,
            options: opts_c,
            capture,
            close,
            modes,
        }
    }

    /// Selects `m` (its toggle updates the mode and the capture button label).
    pub fn select(&self, m: Mode) {
        if let Some((_, b)) = self.modes.iter().find(|(x, _)| *x == m) {
            b.set_active(true);
        }
    }
}

/// Save to (Desktop / Pictures) and Timer (None / 5 / 10 s), radio rows.
fn options_menu(opts: Rc<Cell<Options>>) -> gtk::Popover {
    let p = gtk::Popover::new();
    p.add_css_class("menu");
    p.set_has_arrow(false);
    let col = w::vbox(0);
    col.append(&w::label("Save to", "menu-heading"));
    let cur = opts.get();
    let radio = |text: &str, on: bool, group: Option<&gtk::CheckButton>| {
        let c = gtk::CheckButton::with_label(text);
        c.set_active(on);
        c.set_group(group);
        c.add_css_class("menu-check");
        col.append(&c);
        c
    };
    let mut g: Option<gtk::CheckButton> = None;
    for (name, d) in [("Desktop", Dest::Desktop), ("Pictures", Dest::Pictures)] {
        let c = radio(name, cur.dest == d, g.as_ref());
        let o = opts.clone();
        c.connect_toggled(move |c| {
            if c.is_active() {
                o.set(Options { dest: d, ..o.get() });
            }
        });
        g.get_or_insert(c);
    }
    col.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    col.append(&w::label("Timer", "menu-heading"));
    let mut g: Option<gtk::CheckButton> = None;
    for d in DELAYS {
        let name = if d == 0 {
            "None".to_string()
        } else {
            format!("{d} Seconds")
        };
        let c = radio(&name, cur.delay == d, g.as_ref());
        let o = opts.clone();
        c.connect_toggled(move |c| {
            if c.is_active() {
                o.set(Options {
                    delay: d,
                    ..o.get()
                });
            }
        });
        g.get_or_insert(c);
    }
    p.set_child(Some(&col));
    p
}

/// Floating thumbnail of a saved capture: click opens it, drag drops the file.
pub fn thumbnail(texture: &gdk::Texture, path: &std::path::Path) -> gtk::Widget {
    let pic = gtk::Picture::for_paintable(texture);
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_can_shrink(true);
    pic.set_size_request(w::CAPTURE_THUMB.0, w::CAPTURE_THUMB.1);
    let frame = gtk::Overlay::new();
    let sizer = w::vbox(0);
    sizer.set_size_request(w::CAPTURE_THUMB.0, w::CAPTURE_THUMB.1);
    frame.set_child(Some(&sizer));
    frame.add_overlay(&pic);
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.add_css_class("capture-thumb");
    frame.set_halign(gtk::Align::End);
    frame.set_valign(gtk::Align::End);
    let file = gio::File::for_path(path);
    let click = gtk::GestureClick::new();
    let f2 = file.clone();
    click.connect_released(move |_, _, _, _| {
        let _ = gio::AppInfo::launch_default_for_uri(&f2.uri(), None::<&gio::AppLaunchContext>);
    });
    frame.add_controller(click);
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::COPY);
    drag.set_content(Some(&gdk::ContentProvider::for_value(
        &gdk::FileList::from_array(&[file]).to_value(),
    )));
    let t2 = texture.clone();
    drag.connect_drag_begin(move |d, _| d.set_icon(Some(&t2), 0, 0));
    frame.add_controller(drag);
    frame.upcast()
}

/// Focused window as "x,y wxh" for Window mode.
fn active_window() -> Option<String> {
    let j = crate::hypr::request("j/activewindow")?;
    let v: serde_json::Value = serde_json::from_str(&j).ok()?;
    let n = |k: &str, i: usize| v[k][i].as_i64();
    Some(format!(
        "{},{} {}x{}",
        n("at", 0)?,
        n("at", 1)?,
        n("size", 0)?,
        n("size", 1)?
    ))
}

/// Shell command handler for [`RECORD`]: show/toggle opens the toolbar in Record Entire Screen mode.
pub type Handler = Rc<dyn Fn(&crate::protocol::Command) -> bool>;

/// The toolbar surface (bottom centre) and its thumbnail surface (bottom right), plus the
/// [`RECORD`] handler (Control Center Record tile).
pub fn build(app: &gtk::Application) -> (gtk::Window, Handler) {
    let win = super::window(app, "panel", Spec::capture_bar());
    win.add_css_class("capture");
    let thumb_win = super::window(app, "panel", Spec::capture_thumb());
    thumb_win.add_css_class("capture");
    let bar = Rc::new(Toolbar::new(Mode::Selection, Options::default()));
    win.set_child(Some(&bar.root));
    let record: Handler = {
        let (bar, win) = (bar.clone(), win.clone());
        Rc::new(move |c: &crate::protocol::Command| {
            if c.target != RECORD {
                return false;
            }
            let show = crate::protocol::apply(c.verb, win.is_visible());
            if show {
                bar.select(Mode::RecordScreen);
            }
            win.set_visible(show);
            true
        })
    };
    // Return captures (macOS); the button holds focus whenever the bar opens.
    let cap = bar.capture.clone();
    win.connect_show(move |_| {
        cap.grab_focus();
    });
    let w2 = win.clone();
    bar.close.connect_clicked(move |_| w2.set_visible(false));
    let keys = gtk::EventControllerKey::new();
    let w2 = win.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gdk::Key::Escape {
            w2.set_visible(false);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    win.add_controller(keys);
    let (mode, opts, w2) = (bar.mode.clone(), bar.options.clone(), win.clone());
    let shown = Rc::new(Cell::new(0u32));
    bar.capture.connect_clicked(move |_| {
        let geom = (mode.get() == Mode::Window).then(active_window).flatten();
        let home = std::env::var("HOME").unwrap_or_default();
        let argv = capture::argv(mode.get(), opts.get(), geom.as_deref(), &home);
        // Hide first so the toolbar (and a previous thumbnail) is not in the shot.
        w2.set_visible(false);
        thumb_win.set_visible(false);
        let (tw, shown) = (thumb_win.clone(), shown.clone());
        glib::spawn_future_local(async move {
            glib::timeout_future(capture::SETTLE).await;
            let out = gio::spawn_blocking(move || {
                std::process::Command::new(&argv[0])
                    .args(&argv[1..])
                    .env("WOL_SCREENSHOT_QUIET", "1")
                    .output()
                    .ok()
            })
            .await;
            let Ok(Some(o)) = out else { return };
            let path = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let p = std::path::PathBuf::from(&path);
            let Ok(t) = gdk::Texture::from_filename(&p) else {
                return;
            };
            tw.set_child(Some(&thumbnail(&t, &p)));
            tw.set_visible(true);
            let id = shown.get().wrapping_add(1);
            shown.set(id);
            glib::timeout_add_seconds_local_once(THUMB_SECS, move || {
                if shown.get() == id {
                    tw.set_visible(false);
                }
            });
        });
    });
    (win.upcast(), record)
}
