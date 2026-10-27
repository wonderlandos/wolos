//! Control Center (macOS Sonoma layout). Every control goes through `System` (core backend via
//! `Host`; root actions through wol-admind, which may open the Authenticate sheet, IaraOS ADR 0026).
//! Built once as a resident wol-shell panel; refreshes only while visible. Optional modules follow
//! Settings > Control Center (`ShellPrefs.control_center`). UI pieces are Kit components only.
mod pages;

use crate::focus::{self, Focus};
use crate::kit::{
    self,
    controls::{
        BatteryTile, NowPlayingCard, SliderModule, ToggleTile, cap_scroller, fit_scroller,
        page_stack, show_page_now,
    },
    surface::{Keyboard, Spec},
    widgets as w,
};
use crate::system::{Action, ActionError, PowerMode, System};
use crate::{copy, theme};
use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wol_core::prefs::ControlCenterPrefs;

/// Every argv the Control Center spawns (tests/cc_spawns.rs checks each resolves).
pub const SPAWNS: &[&[&str]] = &[SCREENSHOT, RECORD, WIFI_SETTINGS, SOUND_SETTINGS];
/// Screenshot tile: the capture toolbar, like Super+Shift+5.
const SCREENSHOT: &[&str] = &["wol", "shell", "show", "screenshot"];
/// Record tile: the capture toolbar in Record Entire Screen mode.
const RECORD: &[&str] = &["wol", "shell", "show", crate::ui::capture::RECORD];
const WIFI_SETTINGS: &[&str] = &["wol-settings", "Network"];
const SOUND_SETTINGS: &[&str] = &["wol-settings", "Sound"];

/// Stack page names.
pub const PAGES: [&str; 5] = ["main", "wifi", "bluetooth", "sound", "focus"];

fn spec() -> Spec {
    Spec::panel(
        "wol-panel-controlcenter",
        [true, false, false, true],
        Keyboard::OnDemand,
    )
}

pub fn run() -> glib::ExitCode {
    super::run_panel_app(
        "org.wolos.controlcenter",
        "controlcenter",
        spec(),
        |_, sys| content(sys),
    )
}

/// Resident panel in wol-shell (`wol shell toggle control-center`): opening is one frame, no
/// process start. Built once (live perf: rebuilding per open leaked ~0.5 MB each); the snapshot
/// watcher (1 s refresh) only lives while visible, and every open starts on the main page.
pub fn panel(app: &gtk::Application, sys: Rc<dyn System>) -> gtk::Window {
    let win = super::window(app, "controlcenter", spec());
    win.set_hide_on_close(true);
    let cc = build(sys.clone(), ControlCenterPrefs::default());
    win.set_child(Some(&cc.root));
    {
        let cc = Rc::downgrade(&cc);
        super::ipc::on_reload(move |p| {
            if let Some(cc) = cc.upgrade() {
                cc.ui.prefs.set(p.control_center);
                cc.sync();
            }
        });
    }
    let running: Rc<RefCell<Option<glib::JoinHandle<()>>>> = Default::default();
    {
        let (running, cc) = (running.clone(), cc.clone());
        win.connect_show(move |win| {
            show_page_now(&cc.ui.pages, "main");
            cc.sync();
            fit(&cc.ui.scroll, screen_height(win));
            *running.borrow_mut() = follow(&sys, cc.clone());
        });
    }
    win.connect_hide(move |_| {
        if let Some(h) = running.borrow_mut().take() {
            h.abort();
        }
    });
    let keys = gtk::EventControllerKey::new();
    let w2 = win.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            w2.set_visible(false);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    win.add_controller(keys);
    win.upcast()
}

/// Screen height the panel may not use: menu bar, glass margins and padding.
const CHROME: i32 = 28 + 2 * 24 + 2 * w::INSET + 2 * 8;

/// Scrolls the main page only when its modules are taller than the screen allows.
fn fit(scroll: &gtk::ScrolledWindow, screen: Option<i32>) {
    cap_scroller(scroll, w::CONTROL_CENTER_WIDTH, screen.map(|h| h - CHROME));
}

fn screen_height(win: &impl IsA<gtk::Widget>) -> Option<i32> {
    let d = win.display();
    let m = win
        .native()
        .and_then(|n| n.surface())
        .and_then(|s| d.monitor_at_surface(&s))
        .or_else(|| d.monitors().item(0).and_downcast::<gtk::gdk::Monitor>())?;
    Some(m.geometry().height())
}

/// Control Center app content: follows the snapshot for the life of the process.
pub fn content(sys: Rc<dyn System>) -> gtk::Widget {
    content_with(sys, super::ipc::shell_prefs().control_center, "main")
}

/// Content with explicit module prefs, showing `page` (one of [`PAGES`], or `wifi:other` for the
/// Wi-Fi page with the Other Network sheet open; golden renders).
pub fn content_with(sys: Rc<dyn System>, prefs: ControlCenterPrefs, page: &str) -> gtk::Widget {
    let cc = build(sys.clone(), prefs);
    show_page_now(&cc.ui.pages, page.split(':').next().unwrap_or(page));
    cc.sync();
    if page == "wifi:other" {
        cc.pages.other_network();
    }
    std::mem::forget(follow(&sys, cc.clone()));
    cc.root.clone()
}

/// Follows the snapshot until the returned handle is aborted (dropping the watcher).
fn follow(sys: &Rc<dyn System>, cc: Rc<ControlCenter>) -> Option<glib::JoinHandle<()>> {
    let rx = sys.watch()?;
    Some(glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            cc.sync();
        }
    }))
}

pub(crate) struct Ui {
    pub sys: Rc<dyn System>,
    pub pages: gtk::Stack,
    scroll: gtk::ScrolledWindow,
    pub prefs: Cell<ControlCenterPrefs>,
    message: gtk::Label,
    auth: w::AuthSheet,
    /// Admin action waiting for the password.
    pending: RefCell<Option<Action>>,
    /// Slider actions: one in flight per kind, newest value wins (no fork storm while dragging).
    busy: RefCell<std::collections::HashMap<&'static str, Option<Action>>>,
    /// True while a snapshot pushes values into widgets (their handlers must not act).
    pub syncing: Cell<bool>,
    /// Re-reads every widget after an action settles (failed toggles snap back).
    resync: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Ui {
    /// Runs `a` off the UI thread; `NeedsAuth`/`Denied` open the Authenticate sheet and retry after Allow.
    pub fn act(self: &Rc<Self>, a: Action) {
        if self.syncing.get() {
            return;
        }
        let rx = self.sys.spawn(a.clone());
        let u = self.clone();
        glib::spawn_future_local(async move {
            let Ok(r) = rx.recv().await else { return };
            u.done(a, r);
        });
    }
    fn done(self: &Rc<Self>, a: Action, r: Result<(), ActionError>) {
        match r {
            Ok(()) => self.message.set_visible(false),
            Err(e @ (ActionError::NeedsAuth | ActionError::Denied)) if asks_password(&a) => {
                self.auth.message.set_label(if e == ActionError::Denied {
                    e.message()
                } else {
                    ""
                });
                *self.pending.borrow_mut() = Some(a);
                self.auth.revealer.set_reveal_child(true);
                self.auth.field.grab_focus();
            }
            Err(e) => {
                self.message.set_label(copy::action(e));
                self.message.set_visible(true);
            }
        }
        let f = self.resync.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }
    /// Runs `a` off the UI thread and hands the result to `then` (a page that shows its own
    /// progress and outcome, like joining a network); never opens the Authenticate sheet.
    pub fn act_with(
        self: &Rc<Self>,
        a: Action,
        then: impl FnOnce(Result<(), ActionError>) + 'static,
    ) {
        let rx = self.sys.spawn(a);
        let u = self.clone();
        glib::spawn_future_local(async move {
            let r = rx.recv().await.unwrap_or(Err(ActionError::Failed));
            then(r);
            let f = u.resync.borrow().clone();
            if let Some(f) = f {
                f();
            }
        });
    }
    /// Coalesced variant of [`Ui::act`] for sliders.
    pub fn act_latest(self: &Rc<Self>, kind: &'static str, a: Action) {
        if self.syncing.get() {
            return;
        }
        if let Some(slot) = self.busy.borrow_mut().get_mut(kind) {
            *slot = Some(a);
            return;
        }
        self.busy.borrow_mut().insert(kind, None);
        let rx = self.sys.spawn(a.clone());
        let u = self.clone();
        glib::spawn_future_local(async move {
            let r = rx.recv().await.unwrap_or(Ok(()));
            let next = u.busy.borrow_mut().remove(kind).flatten();
            if r.is_err() {
                u.done(a, r);
            }
            if let Some(n) = next {
                u.act_latest(kind, n);
            }
        });
    }
    fn allow(self: &Rc<Self>) {
        let pw = self.auth.field.text();
        self.auth.field.set_text("");
        match self.sys.authorize(&pw) {
            Ok(()) => {
                self.auth.revealer.set_reveal_child(false);
                if let Some(a) = self.pending.borrow_mut().take() {
                    self.act(a);
                }
            }
            Err(e) => self.auth.message.set_label(e.message()),
        }
    }
    pub fn show(&self, page: &str) {
        self.pages.set_visible_child_name(page);
    }
    fn sliding(&self, kind: &str) -> bool {
        self.busy.borrow().contains_key(kind)
    }
}

/// Wi-Fi and Bluetooth are the active user's to change (no administrator password): their
/// refusals read as a sentence, never as the Authenticate sheet.
fn asks_password(a: &Action) -> bool {
    !matches!(
        a,
        Action::Wifi(_) | Action::Bluetooth(_) | Action::WifiJoin(..) | Action::BtConnect(..)
    )
}

/// Closes the Control Center window holding `wd`, then runs `argv` (capture toolbar, Settings).
pub(crate) fn launch(wd: &impl IsA<gtk::Widget>, argv: &[&str]) {
    if let Some(win) = wd.root().and_downcast::<gtk::Window>() {
        win.close();
    }
    kit::spawn(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>());
}

struct Accessibility {
    row: gtk::Box,
    reduce_motion: ToggleTile,
    contrast: ToggleTile,
    zoom: ToggleTile,
}

pub(crate) struct ControlCenter {
    root: gtk::Widget,
    ui: Rc<Ui>,
    wifi: w::ControlTile,
    bluetooth: w::ControlTile,
    focus: w::ControlTile,
    appearance: w::ControlTile,
    display: SliderModule,
    sound: SliderModule,
    keyboard: SliderModule,
    media: NowPlayingCard,
    battery: BatteryTile,
    tiles: gtk::Box,
    night: ToggleTile,
    screenshot: gtk::Button,
    record: gtk::Button,
    a11y: Accessibility,
    pages: pages::Pages,
}

fn dark_now() -> bool {
    gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme())
}

/// Theme id for the Dark Mode tile: the appearance set explicitly (Auto ends), the accent kept.
fn dark_mode_id(saved: Option<&str>, dark: bool) -> Option<String> {
    use wol_core::theme::accent;
    let variant = if dark {
        theme::Variant::Dark
    } else {
        theme::Variant::Light
    };
    accent::theme_id(Some(variant), accent::split(saved).1)
}

fn focus_label(f: Focus) -> String {
    let t = kit::now();
    focus::label(f, focus::unix_now(), t.hour, t.minute)
}

fn build(sys: Rc<dyn System>, prefs: ControlCenterPrefs) -> Rc<ControlCenter> {
    let panel = w::glass_panel(w::S2);
    panel.add_css_class("control-center");
    panel.set_width_request(w::CONTROL_CENTER_WIDTH);
    panel.set_valign(gtk::Align::Start);
    let pages = page_stack();
    let main = w::vbox(w::S2);
    // Every module on outgrows a 720 px screen: the main page scrolls past the screen height.
    let scroll = fit_scroller(&main);
    let ui = Rc::new(Ui {
        sys: sys.clone(),
        pages: pages.clone(),
        scroll: scroll.clone(),
        prefs: Cell::new(prefs),
        message: w::label("", "secondary"),
        auth: w::authenticate_sheet("Control Center"),
        pending: Default::default(),
        busy: Default::default(),
        syncing: Cell::new(false),
        resync: Default::default(),
    });
    {
        let u = ui.clone();
        ui.auth.cancel.connect_clicked(move |_| {
            u.pending.borrow_mut().take();
            u.auth.revealer.set_reveal_child(false);
        });
    }
    {
        let u = ui.clone();
        ui.auth.allow.connect_clicked(move |_| u.allow());
    }
    {
        let u = ui.clone();
        ui.auth.field.connect_activate(move |_| u.allow());
    }
    panel.append(&ui.auth.revealer);
    pages.add_named(&scroll, Some("main"));
    let detail = pages::Pages::build(&ui);
    panel.append(&pages);

    // Two cards of two controls each (Wi-Fi + Bluetooth, Focus + Dark Mode): equal heights by
    // construction, nothing stretched, so every card keeps its inset on all four sides.
    let top = w::hbox(w::S2);
    top.set_homogeneous(true);
    let conn = w::vbox(w::S2);
    let wifi = w::ControlTile::new("network-wireless-symbolic", "Wi-Fi", "Off", Some(false));
    {
        let (u, t) = (ui.clone(), wifi.circle.clone());
        wifi.circle
            .connect_clicked(move |_| u.act(Action::Wifi(!t.has_css_class("on"))));
        let u = ui.clone();
        wifi.detail.connect_clicked(move |_| u.show("wifi"));
    }
    conn.append(&wifi.root);
    let bluetooth =
        w::ControlTile::new("bluetooth-active-symbolic", "Bluetooth", "Off", Some(false));
    {
        let (u, t) = (ui.clone(), bluetooth.circle.clone());
        bluetooth
            .circle
            .connect_clicked(move |_| u.act(Action::Bluetooth(!t.has_css_class("on"))));
        let u = ui.clone();
        bluetooth
            .detail
            .connect_clicked(move |_| u.show("bluetooth"));
    }
    conn.append(&bluetooth.root);
    top.append(&w::module(&conn));
    let right = w::vbox(w::S2);
    let focus_tile =
        w::ControlTile::new("weather-clear-night-symbolic", "Focus", "Off", Some(false));
    {
        let (u, c) = (ui.clone(), focus_tile.circle.clone());
        focus_tile.circle.connect_clicked(move |_| {
            let on = !c.has_css_class("on");
            let _ = focus::write(if on { Focus::On } else { Focus::Off });
            if let Some(f) = u.resync.borrow().clone() {
                f()
            }
        });
        let u = ui.clone();
        focus_tile.detail.connect_clicked(move |_| u.show("focus"));
    }
    right.append(&focus_tile.root);
    let appearance =
        w::ControlTile::new("wolos-dark-mode-symbolic", "Dark Mode", "Off", Some(false));
    {
        let c = appearance.circle.clone();
        appearance.circle.connect_clicked(move |_| {
            let on = !c.has_css_class("on");
            if let Some(id) = dark_mode_id(kit::style::active_id().as_deref(), on) {
                let _ = kit::style::set_active(&id);
            }
            if on {
                c.add_css_class("on")
            } else {
                c.remove_css_class("on")
            }
        });
    }
    right.append(&appearance.root);
    top.append(&w::module(&right));
    main.append(&top);

    let display = {
        let u = ui.clone();
        SliderModule::new(
            "Display",
            "display-brightness-symbolic",
            0.0,
            move |v| u.act_latest("brightness", Action::Brightness(v as u8)),
            None,
        )
    };
    main.append(&display.root);
    let sound = {
        let u = ui.clone();
        SliderModule::new(
            "Sound",
            "audio-volume-high-symbolic",
            0.0,
            move |v| u.act_latest("volume", Action::Volume(v as u8)),
            Some("Output"),
        )
    };
    if let Some(d) = &sound.detail {
        let u = ui.clone();
        d.connect_clicked(move |_| u.show("sound"));
    }
    main.append(&sound.root);

    let media = NowPlayingCard::compact();
    for (b, a) in [
        (&media.prev, Action::MediaPrevious),
        (&media.play, Action::MediaToggle),
        (&media.next, Action::MediaNext),
    ] {
        let u = ui.clone();
        b.connect_clicked(move |_| u.act(a.clone()));
    }
    main.append(&media.root);

    let keyboard = {
        let u = ui.clone();
        SliderModule::new(
            "Keyboard Brightness",
            "keyboard-brightness-symbolic",
            0.0,
            move |v| u.act_latest("keyboard", Action::KeyboardBrightness(v as u8)),
            None,
        )
    };
    main.append(&keyboard.root);

    let tiles = w::hbox(w::S2);
    tiles.set_homogeneous(true);
    let night = ToggleTile::new("night-light-symbolic", "Night Shift");
    {
        let u = ui.clone();
        night.button.connect_clicked(move |_| {
            u.act(Action::NightLight(!u.sys.night_light().unwrap_or(false)))
        });
    }
    tiles.append(&night.button);
    let screenshot = ToggleTile::new("camera-photo-symbolic", "Screenshot").button;
    screenshot.connect_clicked(|b| launch(b, SCREENSHOT));
    tiles.append(&screenshot);
    let record = ToggleTile::new("media-record-symbolic", "Record").button;
    record.connect_clicked(|b| launch(b, RECORD));
    tiles.append(&record);
    // Battery folds into the tile row: charge at a glance, a click toggles Low Power Mode.
    let battery = BatteryTile::new();
    {
        let u = ui.clone();
        battery.button.connect_clicked(move |_| {
            let Some(pm) = u.sys.power_mode() else { return };
            let low = pm == PowerMode::LowPower;
            u.act(Action::Power(if low {
                PowerMode::Automatic
            } else {
                PowerMode::LowPower
            }))
        });
    }
    tiles.append(&battery.button);
    main.append(&tiles);

    let a11y = {
        let row = w::hbox(w::S2);
        row.set_homogeneous(true);
        let reduce_motion = ToggleTile::new("wolos-reduce-motion-symbolic", "Reduce Motion");
        let contrast = ToggleTile::new("wolos-contrast-symbolic", "Increase Contrast");
        let zoom = ToggleTile::new("zoom-in-symbolic", "Zoom");
        let read = |f: fn(crate::system::Accessibility) -> Option<bool>| f;
        for (t, get, make) in [
            (
                &reduce_motion,
                read(|a| a.reduce_motion),
                Action::ReduceMotion as fn(bool) -> Action,
            ),
            (
                &contrast,
                read(|a| Some(a.increase_contrast)),
                Action::IncreaseContrast,
            ),
            (&zoom, read(|a| a.zoom), Action::Zoom),
        ] {
            let u = ui.clone();
            t.button.connect_clicked(move |_| {
                let on = get(u.sys.accessibility()).unwrap_or(false);
                u.act(make(!on));
            });
            row.append(&t.button);
        }
        Accessibility {
            row,
            reduce_motion,
            contrast,
            zoom,
        }
    };
    main.append(&a11y.row);
    ui.message.set_visible(false);
    main.append(&ui.message);

    let cc = Rc::new(ControlCenter {
        root: panel.upcast(),
        ui: ui.clone(),
        wifi,
        bluetooth,
        focus: focus_tile,
        appearance,
        display,
        sound,
        keyboard,
        media,
        battery,
        tiles,
        night,
        screenshot,
        record,
        a11y,
        pages: detail,
    });
    {
        let weak = Rc::downgrade(&cc);
        *ui.resync.borrow_mut() = Some(Rc::new(move || {
            if let Some(cc) = weak.upgrade() {
                cc.sync();
            }
        }));
    }
    cc
}

/// `bt` -> Bluetooth tile subtitle.
fn bluetooth_label(bt: Option<bool>) -> &'static str {
    match bt {
        Some(true) => "On",
        Some(false) => "Off",
        None => "Unavailable",
    }
}

impl ControlCenter {
    /// Pushes the current snapshot into every widget; modules hide when their hardware or pref is off.
    fn sync(&self) {
        let u = &self.ui;
        let s = &*u.sys;
        let p = u.prefs.get();
        u.syncing.set(true);

        let nets = s.wifi_networks();
        self.wifi
            .set_subtitle(&crate::system::wifi_subtitle(s.network(), &nets));
        self.wifi.set_on(s.wifi_enabled().unwrap_or(false));
        let bt = s.bluetooth();
        self.bluetooth.set_subtitle(bluetooth_label(bt));
        self.bluetooth.set_on(bt.unwrap_or(false));
        self.bluetooth.circle.set_sensitive(bt.is_some());
        let f = focus::read();
        self.focus.set_on(f.active());
        self.focus.set_subtitle(&match f {
            Focus::Off => "Off".to_string(),
            Focus::On => "Do Not Disturb".to_string(),
            Focus::Until(_) => focus_label(f),
        });
        let dark = dark_now();
        self.appearance.set_on(dark);
        self.appearance
            .set_subtitle(if dark { "On" } else { "Off" });

        let b = s.brightness();
        self.display.root.set_visible(b.is_some());
        if let Some(b) = b
            && !u.sliding("brightness")
        {
            self.display.set_value(b as f64);
        }
        let a = s.audio();
        self.sound.set_icon(crate::status::volume(a));
        if let Some(v) = a.volume
            && !u.sliding("volume")
        {
            self.sound.set_value(v as f64);
        }

        let np = s.now_playing().filter(|_| p.now_playing);
        self.media.root.set_visible(np.is_some());
        if let Some(np) = &np {
            self.media.set(np);
            self.media.set_app_icon(s.now_playing_icon().as_deref());
        }

        let bat = s.battery().filter(|_| p.battery);
        self.battery.button.set_visible(bat.is_some());
        let pm = s.power_mode();
        if let Some(bt) = bat {
            self.battery.set(
                bt.percent,
                bt.status,
                s.battery_time(),
                pm == Some(PowerMode::LowPower),
            );
        }

        let k = s.keyboard_brightness().filter(|_| p.keyboard_brightness);
        self.keyboard.root.set_visible(k.is_some());
        if let Some(k) = k
            && !u.sliding("keyboard")
        {
            self.keyboard.set_value(k as f64);
        }

        let night = s.night_light().filter(|_| p.night_shift);
        self.night.button.set_visible(night.is_some());
        self.night.set_on(night.unwrap_or(false));
        self.screenshot.set_visible(p.screenshot);
        self.record.set_visible(p.record);
        self.tiles
            .set_visible(night.is_some() || p.screenshot || p.record || bat.is_some());

        let acc = s.accessibility();
        self.a11y.row.set_visible(p.accessibility);
        self.a11y
            .reduce_motion
            .button
            .set_visible(acc.reduce_motion.is_some());
        self.a11y
            .reduce_motion
            .set_on(acc.reduce_motion.unwrap_or(false));
        self.a11y.contrast.set_on(acc.increase_contrast);
        self.a11y.zoom.button.set_visible(acc.zoom.is_some());
        self.a11y.zoom.set_on(acc.zoom.unwrap_or(false));
        if let (Some(rm), Some(g)) = (acc.reduce_motion, gtk::Settings::default())
            && g.is_gtk_enable_animations() == rm
        {
            g.set_gtk_enable_animations(!rm);
        }

        self.pages.sync(u, f, &focus_label(f));
        u.syncing.set(false);
        super::menubar::refresh_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UX review: the Dark Mode tile switched to the plain dark theme, dropping the accent and
    /// leaving Auto on in Settings. It sets the appearance explicitly and keeps the accent.
    #[test]
    fn dark_mode_keeps_the_accent_and_ends_auto() {
        for (saved, on, want) in [
            (None, true, Some("wol-dark")),
            (Some("wol-auto-blue"), true, Some("wol-dark-blue")),
            (Some("wol-auto-blue"), false, Some("wol-light-blue")),
            (Some("wol-light-pink"), true, Some("wol-dark-pink")),
            (Some("wol-dark-green"), false, Some("wol-light-green")),
            (Some("wol-dark"), false, Some("wol-light")),
        ] {
            assert_eq!(dark_mode_id(saved, on).as_deref(), want, "{saved:?} {on}");
        }
    }
}
