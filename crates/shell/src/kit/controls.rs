//! Control-surface components shared by Control Center, Settings and widgets: toggle tiles, detail
//! pages, device rows (Wi-Fi networks, Bluetooth devices, audio outputs), slider modules, the Now
//! Playing card and the battery gauge. Style: kit.css "Control surfaces" (tokens only).
use super::widgets::{self as w, ICON, S1, S2, S3};
use crate::status::BatteryStatus as Power;
use crate::system::NowPlaying;
use gtk::prelude::*;
use std::cell::RefCell;

/// Icon circle diameter in tiles and device rows.
pub const CIRCLE: i32 = 28;
/// Now Playing cover art edge.
pub const ART: i32 = 48;

/// 16 px icon centred in a [`CIRCLE`] disc (the disc is the image's own CSS box).
fn circle(icon_name: &str) -> gtk::Image {
    let i = w::icon(icon_name, ICON);
    i.add_css_class("icon-circle");
    i.set_size_request(CIRCLE, CIRCLE);
    i.set_halign(gtk::Align::Center);
    i
}

fn set_class(wd: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        wd.add_css_class(class)
    } else {
        wd.remove_css_class(class)
    }
}

/// Square tile: icon circle (accent while on) over a centred caption of up to two lines. Tiles in
/// one homogeneous row share width and height. Action tiles (Screenshot) never turn on.
pub struct ToggleTile {
    pub button: gtk::Button,
    circle: gtk::Image,
}

impl ToggleTile {
    pub fn new(icon_name: &str, title: &str) -> Self {
        let col = w::vbox(S1);
        col.set_valign(gtk::Align::Center);
        let circle = circle(icon_name);
        col.append(&circle);
        let l = w::centered(title, "secondary");
        l.set_wrap(true);
        l.set_lines(2);
        col.append(&l);
        let button = gtk::Button::builder().child(&col).hexpand(true).build();
        button.add_css_class("toggle-tile");
        ToggleTile { button, circle }
    }
    pub fn set_on(&self, on: bool) {
        set_class(&self.circle, "on", on);
    }
    pub fn is_on(&self) -> bool {
        self.circle.has_css_class("on")
    }
}

/// One selectable thing in a list: Wi-Fi network, Bluetooth device, audio output, Focus mode.
/// Icon circle (accent while active), title, optional secondary subtitle, optional trailing icon
/// (lock, check mark). 40 px tall with or without a subtitle.
pub struct DeviceRow {
    pub button: gtk::Button,
    circle: gtk::Image,
    subtitle: gtk::Label,
    trailing: gtk::Image,
}

impl DeviceRow {
    pub fn new(
        icon_name: &str,
        title: &str,
        subtitle: &str,
        active: bool,
        trailing: Option<&str>,
    ) -> Self {
        let row = w::hbox(S2);
        row.add_css_class("device-row");
        let circle = circle(icon_name);
        row.append(&circle);
        let text = w::vbox(0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let t = w::label(title, "");
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.add_css_class("may-ellipsize");
        text.append(&t);
        let sub = w::label(subtitle, "secondary");
        sub.set_visible(!subtitle.is_empty());
        text.append(&sub);
        row.append(&text);
        let tr = w::icon(trailing.unwrap_or("object-select-symbolic"), ICON);
        tr.add_css_class("dim-icon");
        tr.set_visible(trailing.is_some());
        row.append(&tr);
        let button = gtk::Button::builder().child(&row).build();
        button.add_css_class("device-button");
        let r = DeviceRow {
            button,
            circle,
            subtitle: sub,
            trailing: tr,
        };
        r.set_active(active);
        r
    }
    pub fn set_active(&self, on: bool) {
        set_class(&self.circle, "on", on);
    }
    pub fn set_subtitle(&self, s: &str) {
        if self.subtitle.text() != s {
            self.subtitle.set_label(s);
        }
        self.subtitle.set_visible(!s.is_empty());
    }
    pub fn set_trailing(&self, icon_name: Option<&str>) {
        if let Some(n) = icon_name {
            self.trailing.set_icon_name(Some(n));
        }
        self.trailing.set_visible(icon_name.is_some());
    }
}

/// Detail page of a control (Wi-Fi, Bluetooth, Sound output, Focus): back button, title, optional
/// power switch, a secondary heading, the rows, and an optional "… Settings" link at the bottom.
/// Flat: it draws on its panel's glass (one surface, the panel's inset), never as a card in a card.
#[derive(Clone)]
pub struct DetailPage {
    pub root: gtk::Box,
    pub back: gtk::Button,
    pub toggle: Option<gtk::Switch>,
    pub footer: Option<gtk::Button>,
    list: gtk::Box,
    empty: gtk::Label,
    status: gtk::Box,
    status_text: gtk::Label,
    spinner: gtk::Spinner,
}

impl DetailPage {
    pub fn new(title: &str, toggle: Option<bool>, heading: &str, footer: Option<&str>) -> Self {
        let root = w::vbox(S2);
        root.add_css_class("detail-page");
        let head = w::hbox(S2);
        head.add_css_class("detail-head");
        let back = w::icon_button("go-previous-symbolic", "Back", || {});
        head.append(&back);
        let t = w::label(title, "title");
        t.set_hexpand(true);
        t.set_valign(gtk::Align::Center);
        head.append(&t);
        let toggle = toggle.map(|on| {
            // Built off, then set: a switch created on keeps the full-size state icons' height
            // and breaks the compact header switch.
            let s = w::toggle(false, |_| {});
            s.set_active(on);
            head.append(&s);
            s
        });
        root.append(&head);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        if !heading.is_empty() {
            let h = w::label(heading, "secondary");
            h.add_css_class("detail-heading");
            root.append(&h);
        }
        let list = w::vbox(0);
        let empty = w::label("", "secondary");
        empty.add_css_class("detail-empty");
        empty.set_visible(false);
        root.append(&list);
        root.append(&empty);
        let status = w::hbox(S2);
        status.add_css_class("detail-status");
        let spinner = w::spinner();
        spinner.set_size_request(ICON, ICON);
        spinner.set_vexpand(false);
        status.append(&spinner);
        let status_text = w::label("", "secondary");
        status_text.set_wrap(true);
        status_text.set_hexpand(true);
        status.append(&status_text);
        status.set_visible(false);
        root.append(&status);
        let footer = footer.map(|f| {
            root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            let l = w::label(f, "");
            l.set_valign(gtk::Align::Center);
            let b = gtk::Button::builder().child(&l).build();
            b.add_css_class("detail-link");
            root.append(&b);
            b
        });
        DetailPage {
            root,
            back,
            toggle,
            footer,
            list,
            empty,
            status,
            status_text,
            spinner,
        }
    }

    /// A sheet (join a network) shown under the rows, inside the page.
    pub fn set_sheet(&self, sheet: &gtk::Revealer) {
        self.root.insert_child_after(sheet, Some(&self.status));
    }

    /// Progress or outcome under the rows ("Joining “Home”…" with a spinner, then the result);
    /// empty hides it.
    pub fn set_status(&self, text: &str, busy: bool) {
        self.status_text.set_label(text);
        self.spinner.set_visible(busy);
        self.spinner.set_spinning(busy);
        self.status.set_visible(!text.is_empty());
    }

    /// Replaces the rows; `empty` shows instead when there are none.
    pub fn set_rows(&self, rows: &[gtk::Widget], empty: &str) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        for r in rows {
            self.list.append(r);
        }
        self.empty.set_label(empty);
        self.empty.set_visible(rows.is_empty());
    }

    pub fn set_toggle(&self, on: bool) {
        if let Some(s) = &self.toggle
            && s.is_active() != on
        {
            s.set_active(on);
        }
    }
}

/// Pages of one panel (Control Center main page and its detail pages): slide left and right; the
/// panel takes the visible page's height.
pub fn page_stack() -> gtk::Stack {
    gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .vhomogeneous(false)
        .interpolate_size(true)
        .build()
}

/// Shows `page` of `stack` at once (panel reopened, golden render), then slides again.
pub fn show_page_now(stack: &gtk::Stack, page: &str) {
    stack.set_transition_type(gtk::StackTransitionType::None);
    stack.set_visible_child_name(page);
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
}

/// Scroller that takes its content's height until `fit_scroller` caps it (Control Center on a
/// short screen).
pub fn fit_scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let s = gtk::ScrolledWindow::new();
    s.set_child(Some(child));
    s.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
    s
}

/// Caps `scroll` at `cap` px and scrolls only when its content is taller (None = no cap).
pub fn cap_scroller(scroll: &gtk::ScrolledWindow, width: i32, cap: Option<i32>) {
    let Some(main) = scroll.child() else { return };
    let (_, natural, _, _) = main.measure(gtk::Orientation::Vertical, width);
    match cap {
        Some(cap) if natural > cap => {
            scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroll.set_min_content_height(cap);
        }
        _ => {
            scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
            scroll.set_min_content_height(-1);
        }
    }
}

/// Module with a title over a fat slider (icon inside the track), plus an optional chevron that
/// opens a detail page (Sound -> outputs).
pub struct SliderModule {
    pub root: gtk::Box,
    pub scale: gtk::Scale,
    pub detail: Option<gtk::Button>,
    icon: gtk::Image,
}

impl SliderModule {
    pub fn new(
        title: &str,
        icon_name: &str,
        value: f64,
        changed: impl Fn(f64) + 'static,
        detail: Option<&str>,
    ) -> Self {
        let b = w::vbox(S2);
        b.append(&w::label(title, "title"));
        let row = w::hbox(S2);
        let o = gtk::Overlay::new();
        o.set_hexpand(true);
        let scale = w::slider(value, changed);
        scale.add_css_class("fat");
        o.set_child(Some(&scale));
        let icon = w::icon(icon_name, ICON);
        icon.set_halign(gtk::Align::Start);
        icon.add_css_class("fat-icon");
        icon.set_can_target(false);
        o.add_overlay(&icon);
        row.append(&o);
        let detail = detail.map(|tip| {
            let d = w::icon_button("go-next-symbolic", tip, || {});
            d.add_css_class("slider-detail");
            row.append(&d);
            d
        });
        b.append(&row);
        SliderModule {
            root: w::module(&b),
            scale,
            detail,
            icon,
        }
    }
    pub fn set_value(&self, v: f64) {
        if (self.scale.value() - v).abs() >= 0.5 {
            self.scale.set_value(v);
        }
    }
    pub fn set_icon(&self, name: &str) {
        if self.icon.icon_name().as_deref() != Some(name) {
            self.icon.set_icon_name(Some(name));
        }
    }
}

/// "1:01" from seconds.
pub fn clock(secs: u32) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Now Playing: cover art (the player's app icon without one, else a note), title, artist,
/// previous / play-pause / next, and a progress bar with elapsed and remaining time when the
/// length is known. [`NowPlayingCard::compact`] (Control Center) leaves the progress out.
pub struct NowPlayingCard {
    pub root: gtk::Box,
    pub prev: gtk::Button,
    pub play: gtk::Button,
    pub next: gtk::Button,
    art: gtk::Picture,
    frame: gtk::Overlay,
    app_icon: RefCell<Option<String>>,
    compact: bool,
    title: gtk::Label,
    artist: gtk::Label,
    progress: gtk::ProgressBar,
    times: gtk::Box,
    elapsed: gtk::Label,
    remaining: gtk::Label,
    shown: RefCell<Option<NowPlaying>>,
}

impl NowPlayingCard {
    pub fn new() -> Self {
        Self::build(false)
    }

    /// One row: art, title and artist, transport keys (no progress).
    pub fn compact() -> Self {
        Self::build(true)
    }

    fn build(compact: bool) -> Self {
        let col = w::vbox(S2);
        let top = w::hbox(S3);
        let frame = gtk::Overlay::new();
        frame.add_css_class("art");
        frame.set_size_request(ART, ART);
        frame.set_valign(gtk::Align::Center);
        frame.set_overflow(gtk::Overflow::Hidden);
        let note = w::icon("audio-x-generic-symbolic", ICON);
        note.set_halign(gtk::Align::Center);
        note.add_css_class("dim-icon");
        frame.set_child(Some(&note));
        let art = gtk::Picture::new();
        art.set_content_fit(gtk::ContentFit::Cover);
        art.set_can_shrink(true);
        frame.add_overlay(&art);
        top.append(&frame);
        let text = w::vbox(0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let title = w::label("", "title");
        let artist = w::label("", "secondary");
        for l in [&title, &artist] {
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.add_css_class("may-ellipsize");
            text.append(l);
        }
        top.append(&text);
        let keys = w::hbox(0);
        keys.set_valign(gtk::Align::Center);
        let prev = w::icon_button("media-skip-backward-symbolic", "Previous", || {});
        let play = w::icon_button("media-playback-start-symbolic", "Play or pause", || {});
        let next = w::icon_button("media-skip-forward-symbolic", "Next", || {});
        for b in [&prev, &play, &next] {
            b.add_css_class("media-button");
            keys.append(b);
        }
        top.append(&keys);
        col.append(&top);
        let progress = w::progress();
        progress.add_css_class("media-progress");
        col.append(&progress);
        let times = w::hbox(0);
        let elapsed = w::label("", "secondary");
        let remaining = w::label("", "secondary");
        times.append(&elapsed);
        times.append(&w::spacer());
        times.append(&remaining);
        col.append(&times);
        NowPlayingCard {
            root: w::module(&col),
            prev,
            play,
            next,
            art,
            frame,
            app_icon: RefCell::new(None),
            compact,
            title,
            artist,
            progress,
            times,
            elapsed,
            remaining,
            shown: RefCell::new(None),
        }
    }

    /// Updates only what changed (called every refresh while visible). `file://` art loads; any
    /// other art URL shows the placeholder.
    pub fn set(&self, t: &NowPlaying) {
        let first = self.shown.borrow().is_none();
        let old = self.shown.borrow().clone().unwrap_or_default();
        if !first && old == *t {
            return;
        }
        if first || old.title != t.title {
            self.title.set_label(&t.title);
        }
        if first || old.artist != t.artist {
            self.artist.set_label(&t.artist);
        }
        if (first || old.playing != t.playing)
            && let Some(i) = self.play.child().and_downcast::<gtk::Image>()
        {
            i.set_icon_name(Some(if t.playing {
                "media-playback-pause-symbolic"
            } else {
                "media-playback-start-symbolic"
            }));
        }
        if first || old.art != t.art {
            let tex = t
                .art
                .as_deref()
                .and_then(|u| u.strip_prefix("file://"))
                .and_then(|p| gtk::gdk::Texture::from_filename(p).ok());
            self.art.set_paintable(tex.as_ref());
        }
        let len = t.length.filter(|l| *l > 0 && !self.compact);
        self.progress.set_visible(len.is_some());
        self.times.set_visible(len.is_some());
        if let Some(l) = len {
            let p = t.position.min(l);
            self.progress.set_fraction(p as f64 / l as f64);
            self.elapsed.set_label(&clock(p));
            self.remaining.set_label(&format!("-{}", clock(l - p)));
        }
        *self.shown.borrow_mut() = Some(t.clone());
    }
}

impl NowPlayingCard {
    /// Placeholder under missing cover art: the player's app icon, or the note without one.
    pub fn set_app_icon(&self, name: Option<&str>) {
        if self.app_icon.borrow().as_deref() == name {
            return;
        }
        let child: gtk::Widget = match name {
            Some(n) => w::app_icon(n, ART).upcast(),
            None => {
                let note = w::icon("audio-x-generic-symbolic", ICON);
                note.add_css_class("dim-icon");
                note.upcast()
            }
        };
        child.set_halign(gtk::Align::Center);
        child.set_valign(gtk::Align::Center);
        self.frame.set_child(Some(&child));
        set_class(&self.frame, "app", name.is_some());
        *self.app_icon.borrow_mut() = name.map(String::from);
    }
}

impl Default for NowPlayingCard {
    fn default() -> Self {
        Self::new()
    }
}

/// Secondary line under the percentage: "3:11 remaining", "Charging, 1:20 until full", ...
pub fn battery_detail(p: Power, minutes: Option<u32>) -> String {
    let hm = |m: u32| format!("{}:{:02}", m / 60, m % 60);
    match (p, minutes) {
        (Power::Discharging, Some(m)) => format!("{} remaining", hm(m)),
        (Power::Discharging, None) => "On battery".into(),
        (Power::Charging, Some(m)) => format!("Charging, {} until full", hm(m)),
        (Power::Charging, None) => "Charging".into(),
        (Power::Full, _) => "Power adapter connected".into(),
    }
}

/// Charge at or below this percentage draws in the warning colour.
pub const LOW_BATTERY: u8 = 20;
/// Charge at or below this percentage draws in the danger colour.
pub const CRITICAL_BATTERY: u8 = 10;
/// Battery glyph body + nub, drawn (fits a [`CIRCLE`] disc).
pub const BATTERY_GLYPH: (i32, i32) = (20, 10);

/// Tone class of the glyph's fill: green, orange at [`LOW_BATTERY`], red at [`CRITICAL_BATTERY`];
/// green while charging or full.
pub fn battery_tint(percent: u8, p: Power) -> &'static str {
    match (p, percent) {
        (Power::Charging | Power::Full, _) => "tone-success",
        (_, ..=CRITICAL_BATTERY) => "tone-danger",
        (_, ..=LOW_BATTERY) => "tone-warning",
        _ => "tone-success",
    }
}

/// Battery glyph: outlined cell and nub in the text colour, fill to the charge in the tint
/// ([`battery_tint`] class on the area), bolt while charging.
pub struct BatteryGlyph {
    pub area: gtk::DrawingArea,
    state: std::rc::Rc<std::cell::Cell<(u8, bool)>>,
}

impl BatteryGlyph {
    pub fn new() -> Self {
        let area = gtk::DrawingArea::builder()
            .content_width(BATTERY_GLYPH.0)
            .content_height(BATTERY_GLYPH.1)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        area.add_css_class("battery-glyph");
        let state = std::rc::Rc::new(std::cell::Cell::new((0u8, false)));
        let st = state.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let (pct, charging) = st.get();
            let (w, h) = (w as f64, h as f64);
            let fill = a.color();
            let line = a.parent().map_or(fill, |p| p.color());
            let rgba = |c: gtk::gdk::RGBA, al: f64| {
                cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, al)
            };
            let rounded = |x: f64, y: f64, w: f64, h: f64, r: f64| {
                use std::f64::consts::FRAC_PI_2 as Q;
                cr.new_sub_path();
                cr.arc(x + w - r, y + r, r, -Q, 0.0);
                cr.arc(x + w - r, y + h - r, r, 0.0, Q);
                cr.arc(x + r, y + h - r, r, Q, 2.0 * Q);
                cr.arc(x + r, y + r, r, 2.0 * Q, 3.0 * Q);
                cr.close_path();
            };
            let nub = 2.0;
            let body = w - nub - 1.0;
            cr.set_line_width(1.0);
            rgba(line, 0.55);
            rounded(0.5, 0.5, body - 1.0, h - 1.0, 2.5);
            cr.stroke().ok();
            rounded(body + 0.5, h / 2.0 - 2.0, nub, 4.0, 1.0);
            cr.fill().ok();
            let inner = body - 4.0;
            let fw = (inner * pct.min(100) as f64 / 100.0).max(1.5);
            rgba(fill, 1.0);
            rounded(2.0, 2.0, fw, h - 4.0, 1.0);
            cr.fill().ok();
            if charging {
                let (cx, cy) = (body / 2.0, h / 2.0);
                cr.move_to(cx + 1.0, cy - 4.5);
                cr.line_to(cx - 2.5, cy + 0.5);
                cr.line_to(cx - 0.2, cy + 0.5);
                cr.line_to(cx - 1.0, cy + 4.5);
                cr.line_to(cx + 2.5, cy - 0.5);
                cr.line_to(cx + 0.2, cy - 0.5);
                cr.close_path();
                rgba(line, 1.0);
                cr.fill().ok();
            }
        });
        BatteryGlyph { area, state }
    }

    pub fn set(&self, percent: u8, p: Power) {
        let s = (percent.min(100), p == Power::Charging);
        let tint = battery_tint(percent, p);
        if self.state.get() != s || !self.area.has_css_class(tint) {
            for c in ["tone-success", "tone-warning", "tone-danger"] {
                set_class(&self.area, c, c == tint);
            }
            self.state.set(s);
            self.area.queue_draw();
        }
    }
}

impl Default for BatteryGlyph {
    fn default() -> Self {
        Self::new()
    }
}

/// Battery tile for a [`ToggleTile`] row: the glyph in the icon disc over the percentage. A click
/// toggles Low Power Mode (disc in the accent while on); the tooltip says the time remaining.
pub struct BatteryTile {
    pub button: gtk::Button,
    disc: gtk::Box,
    glyph: BatteryGlyph,
    caption: gtk::Label,
}

impl BatteryTile {
    pub fn new() -> Self {
        let col = w::vbox(S1);
        col.set_valign(gtk::Align::Center);
        let disc = w::hbox(0);
        disc.add_css_class("icon-circle");
        disc.set_size_request(CIRCLE, CIRCLE);
        disc.set_halign(gtk::Align::Center);
        let glyph = BatteryGlyph::new();
        glyph.area.set_hexpand(true);
        disc.append(&glyph.area);
        col.append(&disc);
        let caption = w::centered("", "secondary");
        caption.set_wrap(true);
        caption.set_lines(2);
        col.append(&caption);
        let button = gtk::Button::builder().child(&col).hexpand(true).build();
        button.add_css_class("toggle-tile");
        BatteryTile {
            button,
            disc,
            glyph,
            caption,
        }
    }
    /// Charge, state and minutes left; `low_power` lights the disc and names the mode.
    pub fn set(&self, percent: u8, p: Power, minutes: Option<u32>, low_power: bool) {
        self.glyph.set(percent, p);
        set_class(&self.disc, "on", low_power);
        let t = if low_power {
            format!("{percent}%\nLow Power")
        } else {
            format!("{percent}%")
        };
        if self.caption.text() != t {
            self.caption.set_label(&t);
        }
        let tip = battery_detail(p, minutes);
        if self.button.tooltip_text().as_deref() != Some(tip.as_str()) {
            self.button.set_tooltip_text(Some(&tip));
        }
    }
}

impl Default for BatteryTile {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review: panels padded 12 and cards 8 read as uneven insets. One token pads every panel
    /// and card on all four sides, and Rust layout maths (Control Center chrome) uses the same value.
    #[test]
    fn one_inset_token_pads_panels_and_cards() {
        let css = include_str!("kit.css");
        assert!(
            css.contains(&format!("--inset: {}px;", w::INSET)),
            "kit.css must define --inset as kit::widgets::INSET"
        );
        for sel in [".glass-panel {", ".module {"] {
            let rule = css[css.find(sel).unwrap_or_else(|| panic!("{sel}"))..]
                .split('}')
                .next()
                .unwrap();
            assert!(rule.contains("padding: var(--inset);"), "{sel} {rule}");
            assert_eq!(rule.matches("padding").count(), 1, "{sel} {rule}");
        }
    }

    /// Review: the gauge was a plain white bar. The glyph fill is green above 20 %, orange at
    /// 20 % and below, red at 10 % and below; charging is always green.
    #[test]
    fn battery_glyph_tint() {
        for (p, st, want) in [
            (82, Power::Discharging, "tone-success"),
            (21, Power::Discharging, "tone-success"),
            (20, Power::Discharging, "tone-warning"),
            (11, Power::Discharging, "tone-warning"),
            (10, Power::Discharging, "tone-danger"),
            (3, Power::Discharging, "tone-danger"),
            (5, Power::Charging, "tone-success"),
            (100, Power::Full, "tone-success"),
        ] {
            assert_eq!(battery_tint(p, st), want, "{p} {st:?}");
        }
    }

    #[test]
    fn clock_and_battery_text() {
        for (s, want) in [(0, "0:00"), (61, "1:01"), (600, "10:00"), (3661, "1:01:01")] {
            assert_eq!(clock(s), want);
        }
        for (p, m, want) in [
            (Power::Discharging, Some(191), "3:11 remaining"),
            (Power::Discharging, None, "On battery"),
            (Power::Charging, Some(80), "Charging, 1:20 until full"),
            (Power::Charging, None, "Charging"),
            (Power::Full, Some(3), "Power adapter connected"),
        ] {
            assert_eq!(battery_detail(p, m), want);
        }
    }
}
