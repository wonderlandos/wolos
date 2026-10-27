//! Data display components: the desktop widget Card and the pieces any surface shows data with —
//! label roles and tones, Badge, Meter (progress with a tone), Gauge (ring), Chart (line or bar),
//! RangeBar, ClockFace, MonthGrid, cover art and local pictures. Built-in desktop widgets and
//! custom widget layouts (`ui/widgets_layout.rs`) are made of these and nothing else.
//! Drawn parts take their colour from CSS `color` (tone classes), so every theme and accent applies
//! live and no colour is written here. Style: kit.css "Data display".
use super::widgets::{self as w, ICON, ICON_L, ICON_M, S1, S4, S6};
use gtk::prelude::*;
use shared::widgets::Size;
use shared::widgets::grid::METRICS;
pub use shared::widgets::manifest::{ChartKind, IconSize, Role, Tone};
use std::cell::{Cell, RefCell};
use std::f64::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

/// Desktop widget grid: cell edge, gap between cards, margin to the free area's edge (one source
/// with `wol widgets`: `shared::widgets::grid::METRICS`).
pub const CARD_CELL: i32 = METRICS.cell;
pub const CARD_GAP: i32 = METRICS.gap;
pub const CARD_MARGIN: i32 = METRICS.margin;
const _: () = assert!(
    CARD_GAP == S4 && CARD_MARGIN == S6,
    "grid on the spacing scale"
);
/// Card padding (kit.css `.widget-card`) and the content edge of a small card.
pub const CARD_PAD: i32 = S4;
pub const CARD_INNER: i32 = CARD_CELL - 2 * CARD_PAD;
/// Gauge diameters.
pub const GAUGE_S: i32 = 40;
pub const GAUGE_M: i32 = 56;
pub const GAUGE_L: i32 = 88;
/// Chart heights.
pub const CHART_S: i32 = 28;
pub const CHART_M: i32 = 48;
pub const CHART_L: i32 = 96;
/// Clock face diameters: fills a small card / headlines a large one.
pub const CLOCK_S: i32 = CARD_INNER;
pub const CLOCK_L: i32 = 216;
/// Cover art edges (small card, large card).
pub const COVER_S: i32 = 56;
/// Medium card: the art fills the card height.
pub const COVER_M: i32 = CARD_INNER;
pub const COVER_L: i32 = 152;
/// Custom widget image edges (avatars, logos).
pub const IMAGE_S: i32 = 24;
pub const IMAGE_M: i32 = 40;
pub const IMAGE_L: i32 = COVER_S;
/// Range bar height (track is drawn 4 px thick, centred).
pub const RANGE_H: i32 = S4;
/// Characters before a gauge caption ellipsizes (rings sit side by side).
pub const CARD_CHARS: i32 = 14;

pub fn card_metrics() -> crate::widgets::grid::Metrics {
    METRICS
}

pub fn card_size(s: Size) -> (i32, i32) {
    card_metrics().size(s)
}

/// Desktop widget card holding `body`: glass, rounded, exactly its grid size whatever the body
/// asks for, so text ellipsizes at the card's real width (never at a guessed character count) and
/// content clips at the corners.
pub fn card(s: Size, body: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = w::vbox(0);
    b.add_css_class("widget-card");
    b.add_css_class(s.id());
    let (wd, ht) = card_size(s);
    b.set_size_request(wd, ht);
    b.set_overflow(gtk::Overflow::Hidden);
    // A scroller that never scrolls: its natural size is its child's minimum, so a long title
    // cannot widen the card, and the child still gets the whole inner area.
    let clamp = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .propagate_natural_height(false)
        .hexpand(true)
        .vexpand(true)
        .child(body)
        .build();
    b.append(&clamp);
    b
}

pub fn role_class(r: Role) -> &'static str {
    match r {
        Role::Display => "display",
        Role::Large => "large-title",
        Role::Title => "title",
        Role::Body => "",
        Role::Secondary => "secondary",
        Role::Caption => "caption",
    }
}

pub fn tone_class(t: Tone) -> &'static str {
    match t {
        Tone::Text => "tone-text",
        Tone::Dim => "tone-dim",
        Tone::Accent => "tone-accent",
        Tone::Success => "tone-success",
        Tone::Warning => "tone-warning",
        Tone::Danger => "tone-danger",
    }
}

pub fn icon_px(s: IconSize) -> i32 {
    match s {
        IconSize::Small => ICON,
        IconSize::Medium => ICON_M,
        IconSize::Large => ICON_L,
    }
}

/// Label in a type role and tone. Captions are upper case; text ellipsizes at the width it gets.
pub fn text(s: &str, role: Role, tone: Tone) -> gtk::Label {
    let s = if role == Role::Caption {
        s.to_uppercase()
    } else {
        s.to_string()
    };
    let l = w::label(&s, role_class(role));
    if tone != Tone::Text {
        l.add_css_class(tone_class(tone));
    }
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.add_css_class("may-ellipsize");
    l
}

/// Symbolic icon in a tone.
pub fn tinted_icon(name: &str, size: IconSize, tone: Tone) -> gtk::Image {
    let i = w::icon(name, icon_px(size));
    if tone != Tone::Text {
        i.add_css_class(tone_class(tone));
    }
    i
}

/// Count or status pill: "3", "Online".
pub fn badge(s: &str, tone: Tone) -> gtk::Label {
    let l = w::label(s, "badge");
    l.add_css_class(tone_class(tone));
    l.set_valign(gtk::Align::Center);
    l.set_halign(gtk::Align::Start);
    l
}

/// Bar of a share of a whole (memory, XP, download), filled in the tone colour.
pub fn meter(fraction: f64, tone: Tone) -> gtk::ProgressBar {
    let p = w::progress();
    p.add_css_class("meter");
    p.add_css_class(tone_class(tone));
    p.set_fraction(fraction.clamp(0.0, 1.0));
    p
}

fn rgba(cr: &gtk::cairo::Context, c: &gtk::gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        c.red() as f64,
        c.green() as f64,
        c.blue() as f64,
        c.alpha() as f64 * alpha,
    );
}

fn area(class: &str, tone: Tone, wd: i32, ht: i32) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::builder()
        .content_width(wd)
        .content_height(ht)
        .valign(gtk::Align::Center)
        .build();
    a.add_css_class(class);
    a.add_css_class(tone_class(tone));
    a
}

pub fn image_px(s: IconSize) -> i32 {
    match s {
        IconSize::Small => IMAGE_S,
        IconSize::Medium => IMAGE_M,
        IconSize::Large => IMAGE_L,
    }
}

pub fn gauge_px(s: IconSize) -> i32 {
    match s {
        IconSize::Small => GAUGE_S,
        IconSize::Medium => GAUGE_M,
        IconSize::Large => GAUGE_L,
    }
}

/// Ring gauge: track and fill in the tone colour, a value (or icon) in the middle, optional caption.
pub struct Gauge {
    pub root: gtk::Box,
    area: gtk::DrawingArea,
    fraction: Rc<Cell<f64>>,
    value: gtk::Label,
}

impl Gauge {
    pub fn new(
        fraction: f64,
        value: &str,
        icon: Option<&str>,
        caption: Option<&str>,
        tone: Tone,
        size: IconSize,
    ) -> Gauge {
        let px = gauge_px(size);
        let root = w::vbox(S1);
        root.add_css_class("gauge");
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);
        let a = area("gauge-ring", tone, px, px);
        a.set_halign(gtk::Align::Center);
        let fr = Rc::new(Cell::new(fraction.clamp(0.0, 1.0)));
        let f2 = fr.clone();
        let thick = (px as f64 * 0.11).max(4.0);
        a.set_draw_func(move |a, cr, wd, ht| {
            let c = a.color();
            let (cx, cy) = (wd as f64 / 2.0, ht as f64 / 2.0);
            let r = cx.min(cy) - thick / 2.0;
            cr.set_line_width(thick);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            rgba(cr, &c, 0.18);
            cr.arc(cx, cy, r, 0.0, TAU);
            cr.stroke().ok();
            let f = f2.get();
            if f > 0.0 {
                rgba(cr, &c, 1.0);
                cr.arc(cx, cy, r, -FRAC_PI_2, -FRAC_PI_2 + TAU * f.max(0.01));
                cr.stroke().ok();
            }
        });
        let o = gtk::Overlay::new();
        o.set_child(Some(&a));
        o.set_halign(gtk::Align::Center);
        let value_l = w::label(value, &format!("gauge-value-{}", size_id(size)));
        value_l.set_halign(gtk::Align::Center);
        value_l.set_valign(gtk::Align::Center);
        match icon {
            Some(i) => {
                let img = w::icon(
                    i,
                    if size == IconSize::Large {
                        ICON_M
                    } else {
                        ICON
                    },
                );
                img.set_halign(gtk::Align::Center);
                o.add_overlay(&img);
            }
            None => o.add_overlay(&value_l),
        }
        root.append(&o);
        if let Some(c) = caption {
            let l = w::centered(c, "secondary");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(CARD_CHARS);
            l.add_css_class("may-ellipsize");
            root.append(&l);
        }
        Gauge {
            root,
            area: a,
            fraction: fr,
            value: value_l,
        }
    }

    pub fn set(&self, fraction: f64, value: &str) {
        let f = fraction.clamp(0.0, 1.0);
        if (self.fraction.get() - f).abs() > 0.001 {
            self.fraction.set(f);
            self.area.queue_draw();
        }
        if self.value.text() != value {
            self.value.set_label(value);
        }
    }
}

fn size_id(s: IconSize) -> &'static str {
    match s {
        IconSize::Small => "small",
        IconSize::Medium => "medium",
        IconSize::Large => "large",
    }
}

pub fn chart_px(s: IconSize) -> i32 {
    match s {
        IconSize::Small => CHART_S,
        IconSize::Medium => CHART_M,
        IconSize::Large => CHART_L,
    }
}

/// Sparkline (line with a soft fill) or bars over a hairline baseline; scales to `max`, else to
/// the data. Fills the width it is given.
pub struct Chart {
    pub area: gtk::DrawingArea,
    values: Rc<RefCell<Vec<f64>>>,
}

impl Chart {
    pub fn new(
        kind: ChartKind,
        values: Vec<f64>,
        max: Option<f64>,
        tone: Tone,
        size: IconSize,
    ) -> Chart {
        let a = area("chart", tone, CHART_S, chart_px(size));
        a.set_hexpand(true);
        a.set_valign(gtk::Align::End);
        let vals = Rc::new(RefCell::new(values));
        let v2 = vals.clone();
        a.set_draw_func(move |a, cr, wd, ht| {
            let c = a.color();
            let (wd, ht) = (wd as f64, ht as f64);
            let v = v2.borrow();
            rgba(cr, &c, 0.15);
            cr.rectangle(0.0, ht - 1.0, wd, 1.0);
            cr.fill().ok();
            if v.is_empty() {
                return;
            }
            let top = max
                .unwrap_or_else(|| v.iter().cloned().fold(0.0, f64::max) * 1.15)
                .max(f64::EPSILON);
            let y = |x: f64| ht - 1.0 - (x.clamp(0.0, top) / top) * (ht - 3.0);
            match kind {
                ChartKind::Bar => {
                    let n = v.len() as f64;
                    let gap = 2.0;
                    let bw = ((wd - gap * (n - 1.0)) / n).max(1.0);
                    rgba(cr, &c, 1.0);
                    for (i, x) in v.iter().enumerate() {
                        let x0 = i as f64 * (bw + gap);
                        let y0 = y(*x);
                        let r = (bw / 2.0).min(2.0).min(ht - 1.0 - y0);
                        cr.new_sub_path();
                        cr.move_to(x0, ht - 1.0);
                        cr.line_to(x0, y0 + r);
                        cr.arc(
                            x0 + r,
                            y0 + r,
                            r,
                            std::f64::consts::PI,
                            1.5 * std::f64::consts::PI,
                        );
                        cr.arc(x0 + bw - r, y0 + r, r, 1.5 * std::f64::consts::PI, TAU);
                        cr.line_to(x0 + bw, ht - 1.0);
                        cr.close_path();
                        cr.fill().ok();
                    }
                }
                ChartKind::Line => {
                    let step = if v.len() > 1 {
                        wd / (v.len() - 1) as f64
                    } else {
                        0.0
                    };
                    let pts: Vec<(f64, f64)> = v
                        .iter()
                        .enumerate()
                        .map(|(i, x)| (i as f64 * step, y(*x)))
                        .collect();
                    let path = |cr: &gtk::cairo::Context| {
                        cr.move_to(pts[0].0, pts[0].1);
                        for p in &pts[1..] {
                            cr.line_to(p.0, p.1);
                        }
                        if pts.len() == 1 {
                            cr.line_to(wd, pts[0].1);
                        }
                    };
                    path(cr);
                    cr.line_to(
                        if pts.len() == 1 {
                            wd
                        } else {
                            pts[pts.len() - 1].0
                        },
                        ht,
                    );
                    cr.line_to(0.0, ht);
                    cr.close_path();
                    let g = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, ht);
                    g.add_color_stop_rgba(
                        0.0,
                        c.red() as f64,
                        c.green() as f64,
                        c.blue() as f64,
                        0.28,
                    );
                    g.add_color_stop_rgba(
                        1.0,
                        c.red() as f64,
                        c.green() as f64,
                        c.blue() as f64,
                        0.0,
                    );
                    cr.set_source(&g).ok();
                    cr.fill().ok();
                    rgba(cr, &c, 1.0);
                    cr.set_line_width(2.0);
                    cr.set_line_join(gtk::cairo::LineJoin::Round);
                    cr.set_line_cap(gtk::cairo::LineCap::Round);
                    path(cr);
                    cr.stroke().ok();
                }
            }
        });
        Chart {
            area: a,
            values: vals,
        }
    }

    pub fn set(&self, values: Vec<f64>) {
        if *self.values.borrow() != values {
            *self.values.borrow_mut() = values;
            self.area.queue_draw();
        }
    }
}

/// Today's low..high inside the week's min..max (weather): a faint track with the span filled.
pub fn range_bar(lo: f64, hi: f64, min: f64, max: f64, tone: Tone) -> gtk::DrawingArea {
    let a = area("range-bar", tone, CHART_S, RANGE_H);
    a.set_hexpand(true);
    a.set_draw_func(move |a, cr, wd, ht| {
        let c = a.color();
        let (wd, ht) = (wd as f64, ht as f64);
        let t = 4.0;
        let y = (ht - t) / 2.0;
        let pill = |x0: f64, x1: f64| {
            let r = t / 2.0;
            cr.new_sub_path();
            cr.arc(x0 + r, y + r, r, FRAC_PI_2, 3.0 * FRAC_PI_2);
            cr.arc(x1 - r, y + r, r, -FRAC_PI_2, FRAC_PI_2);
            cr.close_path();
        };
        rgba(cr, &c, 0.15);
        pill(0.0, wd);
        cr.fill().ok();
        let span = (max - min).max(f64::EPSILON);
        let x0 = ((lo - min) / span).clamp(0.0, 1.0) * wd;
        let x1 = (((hi - min) / span).clamp(0.0, 1.0) * wd).max(x0 + t);
        rgba(cr, &c, 1.0);
        pill(x0, x1.min(wd));
        cr.fill().ok();
    });
    a
}

/// Analog dial: minute and hour ticks, hour and minute hands (no second hand: nothing moves
/// between minutes), accent pin. `set` once a minute.
pub struct ClockFace {
    pub root: gtk::Overlay,
    area: gtk::DrawingArea,
    time: Rc<Cell<(u8, u8)>>,
}

impl ClockFace {
    pub fn new(size: i32, hour: u8, minute: u8) -> ClockFace {
        let a = area("clock-face", Tone::Text, size, size);
        a.set_halign(gtk::Align::Center);
        let time = Rc::new(Cell::new((hour, minute)));
        let t2 = time.clone();
        a.set_draw_func(move |a, cr, wd, ht| {
            let c = a.color();
            let (cx, cy) = (wd as f64 / 2.0, ht as f64 / 2.0);
            let r = cx.min(cy);
            rgba(cr, &c, 0.06);
            cr.arc(cx, cy, r, 0.0, TAU);
            cr.fill().ok();
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            for i in 0..60 {
                let hour_tick = i % 5 == 0;
                let ang = i as f64 / 60.0 * TAU;
                let (outer, inner) = if hour_tick {
                    (r * 0.9, r * 0.78)
                } else {
                    (r * 0.9, r * 0.85)
                };
                rgba(cr, &c, if hour_tick { 0.85 } else { 0.3 });
                cr.set_line_width(if hour_tick { r * 0.035 } else { r * 0.015 }.max(1.0));
                cr.move_to(cx + inner * ang.sin(), cy - inner * ang.cos());
                cr.line_to(cx + outer * ang.sin(), cy - outer * ang.cos());
                cr.stroke().ok();
            }
            let (h, m) = t2.get();
            let hand = |ang: f64, len: f64, width: f64| {
                cr.set_line_width(width);
                cr.move_to(cx - r * 0.08 * ang.sin(), cy + r * 0.08 * ang.cos());
                cr.line_to(cx + len * ang.sin(), cy - len * ang.cos());
                cr.stroke().ok();
            };
            rgba(cr, &c, 1.0);
            hand(
                ((h % 12) as f64 + m as f64 / 60.0) / 12.0 * TAU,
                r * 0.5,
                (r * 0.07).max(2.0),
            );
            hand(m as f64 / 60.0 * TAU, r * 0.76, (r * 0.045).max(1.5));
        });
        let root = gtk::Overlay::new();
        root.set_child(Some(&a));
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);
        let pin = w::hbox(0);
        pin.add_css_class("clock-pin");
        pin.set_halign(gtk::Align::Center);
        pin.set_valign(gtk::Align::Center);
        pin.set_can_target(false);
        root.add_overlay(&pin);
        ClockFace {
            root,
            area: a,
            time,
        }
    }

    pub fn set(&self, hour: u8, minute: u8) {
        if self.time.get() != (hour, minute) {
            self.time.set((hour, minute));
            self.area.queue_draw();
        }
    }
}

const WEEK_LETTERS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

/// Month grid, weeks from Monday: weekday letters, weekend dim, today in an accent disc.
/// `compact` fits a small card.
pub fn month_grid(weeks: &[[Option<u8>; 7]], today: Option<u8>, compact: bool) -> gtk::Grid {
    let g = gtk::Grid::builder()
        .column_homogeneous(true)
        .row_homogeneous(true)
        .hexpand(true)
        .build();
    g.add_css_class("month-grid");
    if compact {
        g.add_css_class("compact");
    }
    for (i, d) in WEEK_LETTERS.iter().enumerate() {
        let l = w::centered(d, "caption");
        l.add_css_class("tone-dim");
        g.attach(&l, i as i32, 0, 1, 1);
    }
    for (r, week) in weeks.iter().enumerate() {
        for (c, day) in week.iter().enumerate() {
            let Some(day) = day else { continue };
            let l = w::centered(&day.to_string(), "day");
            l.set_halign(gtk::Align::Center);
            l.set_valign(gtk::Align::Center);
            if Some(*day) == today {
                l.add_css_class("today");
            } else if c >= 5 {
                l.add_css_class("tone-dim");
            }
            g.attach(&l, c as i32, r as i32 + 1, 1, 1);
        }
    }
    g
}

/// Album cover (local file) over a note placeholder; rounded square of `size`.
pub fn cover(path: Option<&std::path::Path>, size: i32) -> gtk::Overlay {
    let frame = gtk::Overlay::new();
    frame.add_css_class("art");
    frame.set_size_request(size, size);
    frame.set_valign(gtk::Align::Center);
    frame.set_halign(gtk::Align::Start);
    frame.set_overflow(gtk::Overflow::Hidden);
    let note = w::icon(
        "audio-x-generic-symbolic",
        if size >= COVER_L { ICON_L } else { ICON_M },
    );
    note.set_halign(gtk::Align::Center);
    note.add_css_class("dim-icon");
    frame.set_child(Some(&note));
    if let Some(p) = path {
        let pic = picture(p, size);
        frame.add_overlay(&pic);
    }
    frame
}

/// Local image file, cropped to fill a `size` square (custom widget images, avatars, covers).
pub fn picture(path: &std::path::Path, size: i32) -> gtk::Picture {
    let pic = gtk::Picture::for_filename(path);
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_can_shrink(true);
    pic.set_size_request(size, size);
    pic.add_css_class("picture");
    pic.set_halign(gtk::Align::Center);
    pic.set_valign(gtk::Align::Center);
    pic
}

/// Vertical spacer: takes the free height of a column (pushes what follows to the bottom).
pub fn fill() -> gtk::Box {
    let b = w::vbox(0);
    b.set_vexpand(true);
    b
}

/// Where a dragged card will land: a faint rounded cell of the card's size.
pub fn drop_mark(s: Size) -> gtk::Box {
    let b = w::vbox(0);
    b.add_css_class("drop-mark");
    let (wd, ht) = card_size(s);
    b.set_size_request(wd, ht);
    b.set_can_target(false);
    b
}

/// Transparent, full-size area that reports its size (the desktop grid follows the work area).
pub fn size_probe(resized: impl Fn(i32, i32) + 'static) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::builder()
        .hexpand(true)
        .vexpand(true)
        .build();
    a.connect_resize(move |_, wd, ht| resized(wd, ht));
    a
}

/// Context menu at `(x, y)` of `parent` (right-click); removed again once closed.
pub fn popup_menu(parent: &impl IsA<gtk::Widget>, x: f64, y: f64, rows: Vec<w::MenuRow>) {
    let m = w::menu_keys(rows);
    m.set_parent(parent);
    m.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    m.connect_closed(|p| {
        let p = p.clone();
        gtk::glib::idle_add_local_once(move || p.unparent());
    });
    m.popup();
}

/// Free-position surface (the desktop widget grid): children placed at pixel positions.
pub fn canvas() -> gtk::Fixed {
    let f = gtk::Fixed::new();
    f.add_css_class("canvas");
    f
}

/// `child` with things layered over it (corner buttons, drop marks).
pub fn layers(child: &impl IsA<gtk::Widget>) -> gtk::Overlay {
    let o = gtk::Overlay::new();
    o.set_child(Some(child));
    o
}

/// Vertical scroller for lists that can outgrow their panel.
pub fn scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(child)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build()
}

/// Small round icon button pinned to a card corner: remove (edit mode), add (gallery).
pub fn corner_button(icon_name: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .child(&w::icon(icon_name, ICON))
        .tooltip_text(tooltip)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .build();
    b.add_css_class("corner-button");
    b
}

/// Opens a URL with the user's default handler (xdg MIME defaults; never a fixed browser).
pub fn open_uri(uri: &str) {
    if let Err(e) =
        gtk::gio::AppInfo::launch_default_for_uri(uri, None::<&gtk::gio::AppLaunchContext>)
    {
        eprintln!("wol: open {uri}: {e}");
    }
}

/// Launches an installed app by desktop entry id (`org.gnome.Calendar` or `org.gnome.Calendar.desktop`).
pub fn launch_app(id: &str) {
    let id = if id.ends_with(".desktop") {
        id.to_string()
    } else {
        format!("{id}.desktop")
    };
    match gtk::gio::AppInfo::all()
        .into_iter()
        .find(|a| a.id().is_some_and(|i| i == id.as_str()))
    {
        Some(a) => {
            if let Err(e) = a.launch(&[], None::<&gtk::gio::AppLaunchContext>) {
                eprintln!("wol: launch {id}: {e}");
            }
        }
        None => eprintln!("wol: no app {id}"),
    }
}
