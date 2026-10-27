//! Form components: the rows System Settings (and any app's preferences) are built from. Each row is a
//! title (+ optional secondary subtitle) on the left and exactly one control on the right; put rows in a
//! [`w::section`] card. Panes compose these and never build raw GTK widgets. Style: kit.css "Form rows".
//!
//! ```ignore
//! p.append(&w::section("Windows", &[
//!     form::toggle_row("Tile windows", "", prefs.tiling, move |on| save(on)),
//!     form::picker_row("Layout", "", &["Dwindle", "Master"], 0, move |i| pick(i)),
//!     form::stepper_row("Gaps", "", 12, (0, 32, 4), |v| format!("{v} pt"), move |v| gaps(v)),
//! ]));
//! ```
use super::widgets::{self as w, ICON, S1, S2};
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wol_core::shortcuts::{self, Combo};

/// App icon size in permission and login-item rows.
pub const ICON_ROW: i32 = 24;
/// Wallpaper thumbnail (16:10).
pub const WALLPAPER_THUMB: (i32, i32) = (128, 80);
/// Characters a stepper value keeps room for ("400 ms").
pub const STEPPER_CHARS: i32 = 6;
/// Slider rows report a value this long after the knob stops (one backend call per drag).
pub const SETTLE_MS: u64 = 150;
/// Shortcut recorder width: room for "⌃⌥⇧⌘Space" without changing per row.
pub const SHORTCUT_WIDTH: i32 = 112;

/// Title + subtitle + `control` (the base of every row here).
pub fn row(title: &str, subtitle: &str, control: &impl IsA<gtk::Widget>) -> gtk::Widget {
    w::list_row("", title, subtitle, Some(control.upcast_ref())).upcast()
}

/// On/off setting.
///
/// ```ignore
/// form::toggle_row("Show date", "", true, |on| save(on))
/// ```
pub fn toggle_row(
    title: &str,
    subtitle: &str,
    on: bool,
    changed: impl Fn(bool) + 'static,
) -> gtk::Widget {
    row(title, subtitle, &w::toggle(on, changed))
}

/// Pop-up menu of `options`; `changed(index)` after a pick.
///
/// ```ignore
/// form::picker_row("Layout", "", &["Dwindle", "Master"], 0, |i| pick(i))
/// ```
pub fn picker_row(
    title: &str,
    subtitle: &str,
    options: &[&str],
    selected: usize,
    changed: impl Fn(usize) + 'static,
) -> gtk::Widget {
    let d = w::dropdown(options, selected as u32);
    d.connect_selected_notify(move |d| changed(d.selected() as usize));
    row(title, subtitle, &d)
}

/// Pop-up menu of many `options` with a search field on top (time zones, languages).
///
/// ```ignore
/// form::search_picker_row("Time zone", "", &zones, at, |i| pick(i))
/// ```
pub fn search_picker_row(
    title: &str,
    subtitle: &str,
    options: &[&str],
    selected: usize,
    changed: impl Fn(usize) + 'static,
) -> gtk::Widget {
    let d = w::dropdown(options, selected as u32);
    d.set_enable_search(true);
    d.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
    d.set_expression(Some(gtk::PropertyExpression::new(
        gtk::StringObject::static_type(),
        None::<gtk::Expression>,
        "string",
    )));
    d.connect_selected_notify(move |d| changed(d.selected() as usize));
    row(title, subtitle, &d)
}

/// Two to four mutually exclusive choices shown at once (Light / Dark / Auto).
///
/// ```ignore
/// form::segmented_row("Position on screen", "", &["Left", "Bottom", "Right"], 1, |i| pick(i))
/// ```
pub fn segmented_row(
    title: &str,
    subtitle: &str,
    options: &[&str],
    selected: usize,
    picked: impl Fn(usize) + 'static,
) -> gtk::Widget {
    row(title, subtitle, &w::segmented(options, selected, picked))
}

/// Row whose control is a push button ("Tiling Shortcuts…", "Add Printer…").
///
/// ```ignore
/// form::button_row("Window tiling shortcuts", "", "Show…", move || open_shortcuts())
/// ```
pub fn button_row(
    title: &str,
    subtitle: &str,
    label: &str,
    clicked: impl Fn() + 'static,
) -> gtk::Widget {
    row(
        title,
        subtitle,
        &w::button(label, w::Kind::Default, clicked),
    )
}

/// Row whose subtitle changes later (Do Not Disturb "On until 15:05"): the row and that label.
pub fn status_row(
    icon_name: &str,
    title: &str,
    subtitle: &str,
    control: &impl IsA<gtk::Widget>,
) -> (gtk::Widget, gtk::Label) {
    let r = w::list_row(icon_name, title, "", Some(control.upcast_ref()));
    let sub = w::label(subtitle, "secondary");
    let text = if icon_name.is_empty() {
        r.first_child()
    } else {
        r.first_child().and_then(|i| i.next_sibling())
    };
    if let Some(t) = text.and_downcast::<gtk::Box>() {
        t.append(&sub);
    }
    (r.upcast(), sub)
}

/// "More" button with a menu of actions for one row (a printer's Set as Default, Remove). None
/// when there is no action to offer.
///
/// ```ignore
/// let more = form::actions_button(vec![("Remove Printer", Box::new(move || remove()))]);
/// ```
pub fn actions_button(items: Vec<w::MenuItem>) -> Option<gtk::MenuButton> {
    if items.is_empty() {
        return None;
    }
    let b = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .valign(gtk::Align::Center)
        .has_frame(false)
        .popover(&w::menu(items))
        .build();
    Some(b)
}

/// Read-only value (About, Network details): a long one wraps instead of widening the page; a short
/// one stays on one line.
pub fn info_row(title: &str, value: &str) -> gtk::Widget {
    let v = w::label(value, "secondary");
    // A wrapping label is laid out at its minimum width: only values that could widen the page wrap.
    if value.chars().count() > 2 * w::VALUE_CHARS as usize {
        v.set_wrap(true);
        v.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        v.set_width_chars(w::VALUE_CHARS);
    }
    v.set_xalign(1.0);
    v.set_justify(gtk::Justification::Right);
    v.set_selectable(false);
    row(title, "", &v)
}

/// Row that opens a subpage (General > About): icon, title, subtitle and a chevron. Connect `clicked`.
pub fn nav_row(icon_name: &str, title: &str, subtitle: &str) -> gtk::Button {
    let chevron = w::icon("go-next-symbolic", ICON);
    chevron.add_css_class("dim-icon");
    let b = gtk::Button::builder()
        .child(&w::list_row(
            icon_name,
            title,
            subtitle,
            Some(chevron.upcast_ref()),
        ))
        .build();
    b.add_css_class("nav-row");
    b
}

/// One-line text setting (computer name); `committed` on Enter or when focus leaves with a change.
pub fn entry_row(
    title: &str,
    text: &str,
    placeholder: &str,
    committed: impl Fn(String) + 'static,
) -> gtk::Widget {
    let e = w::text_field(placeholder);
    e.set_text(text);
    e.set_width_chars(w::VALUE_CHARS);
    let last = Rc::new(RefCell::new(text.to_string()));
    let committed = Rc::new(committed);
    let fire = {
        let (last, committed) = (last.clone(), committed.clone());
        move |e: &gtk::Entry| {
            let t = e.text().to_string();
            if *last.borrow() != t {
                *last.borrow_mut() = t.clone();
                committed(t);
            }
        }
    };
    let f2 = fire.clone();
    e.connect_activate(move |e| f2(e));
    let focus = gtk::EventControllerFocus::new();
    let e2 = e.clone();
    focus.connect_leave(move |_| fire(&e2));
    e.add_controller(focus);
    row(title, "", &e)
}

/// Calls `f` once input settles: each call re-arms; only the last one in `ms` runs.
fn settle<T: 'static>(ms: u64, f: impl Fn(T) + 'static) -> impl Fn(T) {
    let turn = Rc::new(Cell::new(0u64));
    let f = Rc::new(f);
    move |v: T| {
        let g = turn.get() + 1;
        turn.set(g);
        let (turn, f) = (turn.clone(), f.clone());
        let v = RefCell::new(Some(v));
        glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
            if turn.get() == g
                && let Some(v) = v.take()
            {
                f(v)
            }
        });
    }
}

/// Slider with end captions ("Slow" ... "Fast") under its title. Values snap to `step`; `changed` fires
/// once the knob settles ([`SETTLE_MS`]), so a drag is one backend call.
///
/// ```ignore
/// form::slider_row("Key repeat rate", (10.0, 60.0, 5.0), 25.0, ("Slow", "Fast"), |v| rate(v))
/// ```
pub fn slider_row(
    title: &str,
    (min, max, step): (f64, f64, f64),
    value: f64,
    (low, high): (&str, &str),
    changed: impl Fn(f64) + 'static,
) -> gtk::Widget {
    let b = w::vbox(S1);
    b.add_css_class("list-row");
    b.add_css_class("slider-row");
    b.append(&w::label(title, ""));
    let line = w::hbox(S2);
    let lo = w::label(low, "secondary");
    let hi = w::label(high, "secondary");
    let settled = settle(SETTLE_MS, changed);
    let s = w::row_slider(value, |_| {});
    s.set_range(min, max);
    s.set_increments(step, step);
    s.set_value(value);
    s.connect_value_changed(move |s| {
        let v = min + ((s.value() - min) / step).round() * step;
        if (s.value() - v).abs() > f64::EPSILON {
            s.set_value(v);
            return;
        }
        settled(v);
    });
    line.append(&lo);
    line.append(&s);
    line.append(&hi);
    b.append(&line);
    b.upcast()
}

/// Number with − and + buttons, `step` at a time within `min..=max`; `format` renders it ("12 pt").
///
/// ```ignore
/// form::stepper_row("Gaps", "", 12, (0, 32, 4), |v| format!("{v} pt"), |v| gaps(v))
/// ```
pub fn stepper_row(
    title: &str,
    subtitle: &str,
    value: i64,
    (min, max, step): (i64, i64, i64),
    format: fn(i64) -> String,
    changed: impl Fn(i64) + 'static,
) -> gtk::Widget {
    let cur = Rc::new(Cell::new(value.clamp(min, max)));
    let shown = w::label(&format(cur.get()), "stepper-value");
    shown.set_width_chars(STEPPER_CHARS);
    shown.set_xalign(1.0);
    let changed = Rc::new(changed);
    let less: Rc<RefCell<Option<gtk::Button>>> = Default::default();
    let more: Rc<RefCell<Option<gtk::Button>>> = Default::default();
    let sync = {
        let (cur, less, more) = (cur.clone(), less.clone(), more.clone());
        move || {
            if let Some(b) = &*less.borrow() {
                b.set_sensitive(cur.get() > min);
            }
            if let Some(b) = &*more.borrow() {
                b.set_sensitive(cur.get() < max);
            }
        }
    };
    let sync = Rc::new(sync);
    let bump = |d: i64| {
        let (cur, shown, changed, sync) =
            (cur.clone(), shown.clone(), changed.clone(), sync.clone());
        move || {
            let v = (cur.get() + d).clamp(min, max);
            if v != cur.get() {
                cur.set(v);
                shown.set_label(&format(v));
                sync();
                changed(v);
            }
        }
    };
    let l = w::icon_button("list-remove-symbolic", "Less", bump(-step));
    let m = w::icon_button("list-add-symbolic", "More", bump(step));
    *less.borrow_mut() = Some(l.clone());
    *more.borrow_mut() = Some(m.clone());
    sync();
    let c = w::hbox(S1);
    c.add_css_class("stepper");
    c.append(&shown);
    c.append(&l);
    c.append(&m);
    row(title, subtitle, &c)
}

/// An app and one thing it may do (Privacy & Security, Login Items): app icon, name, detail, toggle.
pub fn permission_row(
    app_icon: &str,
    app: &str,
    detail: &str,
    granted: bool,
    changed: impl Fn(bool) + 'static,
) -> gtk::Widget {
    let r = w::list_row(
        "",
        app,
        detail,
        Some(w::toggle(granted, changed).upcast_ref()),
    );
    r.prepend(&w::app_icon(app_icon, ICON_ROW));
    r.upcast()
}

/// Inline warning next to its cause (HIG §3), with the ways out as buttons.
///
/// ```ignore
/// form::notice("⌘Space is used by “Show Spotlight search”.", &[swap, cancel])
/// ```
pub fn notice(text: &str, actions: &[gtk::Button]) -> gtk::Box {
    let b = w::hbox(S2);
    b.add_css_class("list-row");
    b.add_css_class("notice");
    let i = w::icon("dialog-warning-symbolic", ICON);
    i.add_css_class("warning-icon");
    b.append(&i);
    let l = w::label(text, "");
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_hexpand(true);
    b.append(&l);
    for a in actions {
        a.set_valign(gtk::Align::Center);
        b.append(a);
    }
    b
}

/// Wallpaper choice: thumbnail (cropped to fill, loaded off the UI thread) with an accent ring when
/// selected, and a caption. Group several with `set_group`.
pub fn wallpaper_tile(
    file: Option<&std::path::Path>,
    name: &str,
    selected: bool,
) -> gtk::ToggleButton {
    let files: Vec<&std::path::Path> = file.into_iter().collect();
    wallpaper_strips_tile(&files, name, selected)
}

/// [`wallpaper_tile`] showing several pictures side by side in equal strips (Dynamic: day, dusk,
/// night).
pub fn wallpaper_strips_tile(
    files: &[&std::path::Path],
    name: &str,
    selected: bool,
) -> gtk::ToggleButton {
    let (tw, th) = WALLPAPER_THUMB;
    // An overlay child never sizes its parent: the frame stays thumbnail-sized whatever the image is.
    let frame = gtk::Overlay::new();
    frame.add_css_class("wallpaper-thumb");
    frame.set_size_request(tw, th);
    frame.set_halign(gtk::Align::Center);
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.set_child(Some(&w::vbox(0)));
    let strips = w::hbox(0);
    strips.set_homogeneous(true);
    for f in files {
        let pic = gtk::Picture::new();
        pic.set_content_fit(gtk::ContentFit::Cover);
        pic.set_can_shrink(true);
        strips.append(&pic);
        let f = f.to_path_buf();
        glib::spawn_future_local(async move {
            let r = gtk::gio::spawn_blocking(move || {
                gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&f, tw * 2, -1, true)
                    .ok()
                    .map(|pb| gdk::Texture::for_pixbuf(&pb))
            })
            .await;
            if let Ok(Some(t)) = r {
                pic.set_paintable(Some(&t));
            }
        });
    }
    frame.add_overlay(&strips);
    let b = w::vbox(S1);
    b.append(&frame);
    let cap = w::centered(name, "secondary");
    cap.set_ellipsize(gtk::pango::EllipsizeMode::End);
    cap.add_css_class("may-ellipsize");
    cap.set_max_width_chars(w::TILE_CHARS);
    b.append(&cap);
    let t = gtk::ToggleButton::builder()
        .child(&b)
        .active(selected)
        .tooltip_text(name)
        .build();
    t.add_css_class("preview-tile");
    t.add_css_class("wallpaper-tile");
    t
}

/// GDK modifier state as [`shortcuts`] bits.
fn mods_of(state: gdk::ModifierType) -> u8 {
    let mut m = 0;
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        m |= shortcuts::CTRL;
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        m |= shortcuts::ALT;
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        m |= shortcuts::SHIFT;
    }
    if state.intersects(gdk::ModifierType::SUPER_MASK | gdk::ModifierType::META_MASK) {
        m |= shortcuts::SUPER;
    }
    m
}

/// Click-to-record keyboard shortcut (Keyboard > Shortcuts, any app's preferences). Shows the combo
/// ("⇧⌘4"); click, then press the new keys. Esc cancels, ⌫ turns the shortcut off. `recording(true)`
/// fires when it starts listening and `recording(false)` when it stops, so the caller can pause global
/// shortcuts meanwhile. [`set_conflict`](Self::set_conflict) marks a combo another shortcut already uses.
///
/// ```ignore
/// let r = ShortcutRecorder::new(Some(&combo), |on| pause(on), move |c| assign(c));
/// row.append(&r.button);
/// ```
pub struct ShortcutRecorder {
    pub button: gtk::Button,
    label: gtk::Label,
    combo: Rc<RefCell<Option<Combo>>>,
}

impl ShortcutRecorder {
    pub fn new(
        combo: Option<&Combo>,
        recording: impl Fn(bool) + 'static,
        recorded: impl Fn(Option<Combo>) + 'static,
    ) -> Self {
        let label = w::label("", "");
        label.set_xalign(0.5);
        let button = gtk::Button::builder().child(&label).build();
        button.add_css_class("shortcut-recorder");
        button.set_valign(gtk::Align::Center);
        // One width for every recorder: a column of shortcuts lines up.
        button.set_size_request(SHORTCUT_WIDTH, -1);
        button.set_focus_on_click(true);
        let r = ShortcutRecorder {
            button,
            label,
            combo: Rc::new(RefCell::new(combo.cloned())),
        };
        r.show();
        let listening = Rc::new(Cell::new(false));
        let recording = Rc::new(recording);
        let stop = {
            let (listening, recording, b, l, c) = (
                listening.clone(),
                recording.clone(),
                r.button.clone(),
                r.label.clone(),
                r.combo.clone(),
            );
            Rc::new(move || {
                if listening.replace(false) {
                    b.remove_css_class("recording");
                    show(&l, c.borrow().as_ref());
                    recording(false);
                }
            })
        };
        {
            let (listening, recording, l) = (listening.clone(), recording.clone(), r.label.clone());
            r.button.connect_clicked(move |b| {
                if !listening.replace(true) {
                    b.add_css_class("recording");
                    b.remove_css_class("conflict");
                    l.set_label("Type Shortcut");
                    b.grab_focus();
                    recording(true);
                }
            });
        }
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let recorded = Rc::new(recorded);
        {
            let (listening, stop, c, b) = (
                listening.clone(),
                stop.clone(),
                r.combo.clone(),
                r.button.downgrade(),
            );
            keys.connect_key_pressed(move |ctl, keyval, code, state| {
                if !listening.get() {
                    return glib::Propagation::Proceed;
                }
                let mods = mods_of(state);
                let plain = mods == 0;
                if plain && keyval == gdk::Key::Escape {
                    stop();
                    return glib::Propagation::Stop;
                }
                if plain && matches!(keyval, gdk::Key::BackSpace | gdk::Key::Delete) {
                    *c.borrow_mut() = None;
                    stop();
                    recorded(None);
                    return glib::Propagation::Stop;
                }
                // Hyprland matches the unshifted key (⇧⌘4 is SHIFT+4, not "dollar").
                let base = ctl
                    .widget()
                    .map(|wd| wd.display())
                    .and_then(|d| {
                        let layout = ctl
                            .current_event()
                            .and_then(|e| e.downcast::<gdk::KeyEvent>().ok())
                            .map_or(0, |e| e.layout() as i32);
                        d.translate_key(code, gdk::ModifierType::empty(), layout)
                    })
                    .map_or(keyval, |t| t.0);
                if let Some(k) = base.name().and_then(|n| Combo::recorded(mods, &n)) {
                    *c.borrow_mut() = Some(k.clone());
                    stop();
                    if let Some(b) = b.upgrade() {
                        b.remove_css_class("conflict");
                    }
                    recorded(Some(k));
                }
                glib::Propagation::Stop
            });
        }
        r.button.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(move |_| stop());
        r.button.add_controller(focus);
        r
    }

    fn show(&self) {
        show(&self.label, self.combo.borrow().as_ref());
    }

    /// Shows `combo` (after a reset or a swap).
    pub fn set(&self, combo: Option<&Combo>) {
        *self.combo.borrow_mut() = combo.cloned();
        self.show();
    }

    /// Warning outline: the shown combo is taken by another shortcut.
    pub fn set_conflict(&self, on: bool) {
        if on {
            self.button.add_css_class("conflict");
        } else {
            self.button.remove_css_class("conflict");
        }
    }

    /// Fixed shortcuts (mouse drags, media keys) show their keys but cannot be recorded.
    pub fn set_editable(&self, on: bool) {
        self.button.set_sensitive(on);
    }
}

fn show(l: &gtk::Label, c: Option<&Combo>) {
    match c {
        Some(c) => {
            l.set_label(&c.label());
            l.remove_css_class("dim-label");
        }
        None => {
            l.set_label("Off");
            l.add_css_class("dim-label");
        }
    }
}

/// Subpage body (General > About, Keyboard > Shortcuts): back button and large title, then content.
///
/// ```ignore
/// let p = form::subpage("Keyboard Shortcuts", move || aw.open("Keyboard"));
/// p.append(&w::section("Screenshots", &rows));
/// ```
pub fn subpage(title: &str, back: impl Fn() + 'static) -> gtk::Box {
    let p = w::vbox(super::widgets::S6);
    p.add_css_class("page");
    let head = w::hbox(S1);
    head.add_css_class("subpage-header");
    head.append(&w::icon_button("go-previous-symbolic", "Back", back));
    head.append(&w::label(title, "large-title"));
    p.append(&head);
    p
}
