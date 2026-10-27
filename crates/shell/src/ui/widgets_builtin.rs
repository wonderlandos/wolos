//! Built-in desktop widgets (Clock, Calendar, Weather, Music, Batteries, System) in each size,
//! made of Kit parts only (`kit::data`, `kit::controls`, `kit::widgets`). A view is a function of
//! its data: the desktop rebuilds a card's content when that data changes, never on a timer of
//! its own, so nothing moves unless something changed.
use crate::kit::controls as k;
use crate::kit::data::{self as d, IconSize, Role, Tone};
use crate::kit::widgets as w;
use crate::widgets::{Now, calendar, media, power, weather};
use gtk::prelude::*;
use shared::widgets::{Builtin, Face, Place, Size};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum WeatherView {
    /// No city chosen yet.
    NoPlace,
    Loading(Place),
    /// `stale`: the last refresh failed; the forecast is from the cache ("Updated 40 min ago").
    Ready {
        place: Place,
        forecast: weather::Forecast,
        stale: Option<String>,
    },
    /// Nothing cached and the service is unreachable.
    Offline(Place),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemView {
    /// CPU busy percent, oldest first.
    pub cpu: Vec<f64>,
    pub mem_used: i64,
    pub mem_total: i64,
    pub load: Option<(f64, f64, f64)>,
    pub uptime: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Clock {
        now: Now,
        face: Face,
    },
    /// `events`: None = no calendar service (month only).
    Calendar {
        now: Now,
        events: Option<Vec<calendar::Event>>,
    },
    Weather(WeatherView),
    Music(Option<media::Playing>),
    Batteries {
        devices: Vec<power::Device>,
        /// This computer's time to empty or full, when known.
        minutes: Option<u32>,
    },
    System(SystemView),
}

/// What a built-in may ask the desktop to do.
#[derive(Clone)]
pub struct Hooks {
    /// Weather without a city: open the gallery's city picker.
    pub choose_city: Rc<dyn Fn()>,
}

impl Default for Hooks {
    fn default() -> Self {
        Hooks {
            choose_city: Rc::new(|| {}),
        }
    }
}

/// The card for `data` at `size`.
pub fn card(data: &Data, size: Size, hooks: &Hooks) -> gtk::Box {
    let body = match data {
        Data::Clock { now, face } => clock(now, *face, size),
        Data::Calendar { now, events } => calendar_view(now, events.as_deref(), size),
        Data::Weather(v) => weather_view(v, size, hooks),
        Data::Music(p) => music(p.as_ref(), size),
        Data::Batteries { devices, minutes } => batteries(devices, *minutes, size),
        Data::System(s) => system(s, size),
    };
    body.set_vexpand(true);
    d::card(size, &body)
}

/// Sizes a built-in offers (all three for every built-in).
pub fn sizes(_b: Builtin) -> &'static [Size] {
    &Size::ALL
}

fn t(s: &str, role: Role) -> gtk::Label {
    d::text(s, role, Tone::Text)
}

fn dim(s: &str) -> gtk::Label {
    d::text(s, Role::Secondary, Tone::Dim)
}

/// Wrapping secondary text, at most `lines` lines.
fn note(s: &str, lines: i32) -> gtk::Label {
    let l = dim(s);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_lines(lines);
    l.set_xalign(0.0);
    l
}

fn centered(l: gtk::Label) -> gtk::Label {
    l.set_halign(gtk::Align::Center);
    l.set_justify(gtk::Justification::Center);
    l.set_xalign(0.5);
    l
}

fn date_line(n: &Now) -> String {
    format!(
        "{} {}",
        n.day,
        calendar::MONTHS[(n.month as usize - 1) % 12]
    )
}

fn weekday(n: &Now) -> &'static str {
    calendar::WEEKDAYS[n.weekday as usize % 7]
}

fn hhmm(n: &Now) -> String {
    format!("{:02}:{:02}", n.hour, n.minute)
}

/// Placeholder for "nothing to show": icon, title, one line of help.
fn empty(icon: &str, title: &str, hint: &str, size: Size) -> gtk::Box {
    let b = w::vbox(w::S1);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    let i = d::tinted_icon(
        icon,
        if size == Size::Small {
            IconSize::Medium
        } else {
            IconSize::Large
        },
        Tone::Dim,
    );
    i.set_margin_bottom(w::S1);
    b.append(&i);
    b.append(&centered(t(title, Role::Title)));
    let h = centered(note(hint, 3));
    b.append(&h);
    b
}

// ---- Clock ----

fn clock(n: &Now, face: Face, size: Size) -> gtk::Box {
    let (h, m) = (n.hour as u8, n.minute as u8);
    let b = w::vbox(w::S1);
    match (size, face) {
        (Size::Small, Face::Analog) => {
            b.set_valign(gtk::Align::Center);
            b.append(&d::ClockFace::new(d::CLOCK_S, h, m).root);
        }
        (Size::Small, Face::Digital) => {
            b.append(&d::text(weekday(n), Role::Caption, Tone::Accent));
            b.append(&d::fill());
            b.append(&t(&hhmm(n), Role::Display));
            b.append(&dim(&date_line(n)));
        }
        (Size::Medium, Face::Analog) => {
            let r = w::hbox(w::S4);
            r.append(&d::ClockFace::new(d::CLOCK_S, h, m).root);
            let txt = w::vbox(0);
            txt.set_valign(gtk::Align::Center);
            txt.set_hexpand(true);
            txt.append(&t(&hhmm(n), Role::Display));
            txt.append(&t(weekday(n), Role::Title));
            txt.append(&dim(&format!("{} {}", date_line(n), n.y)));
            r.append(&txt);
            b.append(&r);
            b.set_valign(gtk::Align::Center);
        }
        (Size::Large, Face::Analog) => {
            b.set_spacing(w::S3);
            b.set_valign(gtk::Align::Center);
            b.append(&d::ClockFace::new(d::CLOCK_L, h, m).root);
            b.append(&centered(t(
                &format!("{}, {}", weekday(n), date_line(n)),
                Role::Title,
            )));
        }
        (Size::Medium | Size::Large, Face::Digital) => {
            b.set_valign(gtk::Align::Center);
            b.append(&centered(d::text(weekday(n), Role::Caption, Tone::Accent)));
            b.append(&centered(t(&hhmm(n), Role::Display)));
            b.append(&centered(dim(&format!("{} {}", date_line(n), n.y))));
        }
    }
    b
}

// ---- Calendar ----

fn event_row(e: &calendar::Event) -> gtk::Box {
    let r = w::hbox(w::S2);
    let mark = d::badge(&e.time_label(), Tone::Accent);
    r.append(&mark);
    let title = t(&e.title, Role::Body);
    title.set_hexpand(true);
    title.set_xalign(0.0);
    r.append(&title);
    r
}

fn calendar_view(n: &Now, events: Option<&[calendar::Event]>, size: Size) -> gtk::Box {
    let weeks = calendar::weeks(n.y, n.month);
    let today = Some(n.day as u8);
    let month = calendar::MONTHS[(n.month as usize - 1) % 12];
    let b = w::vbox(w::S1);
    match size {
        Size::Small => {
            b.set_valign(gtk::Align::Center);
            b.append(&d::text(month, Role::Caption, Tone::Accent));
            b.append(&d::month_grid(&weeks, today, true));
        }
        Size::Medium => {
            let r = w::hbox(w::S4);
            r.add_css_class("no-row-check");
            let left = w::vbox(0);
            left.set_hexpand(true);
            left.append(&d::text(weekday(n), Role::Caption, Tone::Accent));
            left.append(&t(&n.day.to_string(), Role::Display));
            left.append(&d::fill());
            match events {
                Some([first, rest @ ..]) => {
                    left.append(&t(&first.title, Role::Title));
                    let more = if rest.is_empty() {
                        first.time_label()
                    } else {
                        format!("{} · {} more", first.time_label(), rest.len())
                    };
                    left.append(&dim(&more));
                }
                Some([]) => left.append(&note("No more events today", 2)),
                None => left.append(&dim(&format!("{month} {}", n.y))),
            }
            r.append(&left);
            let g = d::month_grid(&weeks, today, true);
            g.set_hexpand(false);
            g.set_valign(gtk::Align::Center);
            r.append(&g);
            b.append(&r);
        }
        Size::Large => {
            let head = w::hbox(w::S2);
            let wd = d::text(weekday(n), Role::Caption, Tone::Accent);
            head.append(&wd);
            head.append(&w::spacer());
            head.append(&d::text(
                &format!("{month} {}", n.y),
                Role::Caption,
                Tone::Dim,
            ));
            b.append(&head);
            let g = d::month_grid(&weeks, today, false);
            g.set_vexpand(true);
            b.append(&g);
            if let Some(ev) = events {
                let list = w::vbox(w::S2);
                list.set_margin_top(w::S2);
                if ev.is_empty() {
                    list.append(&note("No more events today", 1));
                }
                for e in ev.iter().take(3) {
                    list.append(&event_row(e));
                }
                b.append(&list);
            }
        }
    }
    b
}

// ---- Weather ----

fn place_title(p: &Place, offline: bool) -> gtk::Box {
    let r = w::hbox(w::S1);
    let l = t(&p.name, Role::Title);
    r.append(&l);
    if offline {
        r.append(&d::tinted_icon(
            "network-offline-symbolic",
            IconSize::Small,
            Tone::Dim,
        ));
    }
    r
}

fn high_low(f: &weather::Forecast) -> String {
    format!(
        "H:{} L:{}",
        weather::degrees(f.high),
        weather::degrees(f.low)
    )
}

fn weather_now(p: &Place, f: &weather::Forecast, stale: bool, size: Size) -> gtk::Box {
    let (cond, icon) = weather::condition(f.code, f.day);
    if size == Size::Small {
        let b = w::vbox(0);
        b.append(&place_title(p, stale));
        b.append(&t(&weather::degrees(f.temp), Role::Display));
        b.append(&d::fill());
        let i = d::tinted_icon(icon, IconSize::Small, Tone::Text);
        i.set_halign(gtk::Align::Start);
        b.append(&i);
        b.append(&t(cond, Role::Body));
        b.append(&dim(&high_low(f)));
        return b;
    }
    let r = w::hbox(w::S2);
    r.add_css_class("no-row-check");
    let left = w::vbox(0);
    left.set_hexpand(true);
    left.append(&place_title(p, stale));
    left.append(&t(&weather::degrees(f.temp), Role::Display));
    r.append(&left);
    let right = w::vbox(0);
    right.set_valign(gtk::Align::End);
    let i = d::tinted_icon(icon, IconSize::Small, Tone::Text);
    i.set_halign(gtk::Align::End);
    right.append(&i);
    for l in [t(cond, Role::Body), dim(&high_low(f))] {
        l.set_halign(gtk::Align::End);
        l.set_xalign(1.0);
        right.append(&l);
    }
    r.append(&right);
    r
}

fn hourly(f: &weather::Forecast) -> gtk::Box {
    let r = w::hbox(0);
    r.set_homogeneous(true);
    for h in f.hours.iter().take(weather::FORECAST_HOURS) {
        let c = w::vbox(w::S1);
        let (_, icon) = weather::condition(h.code, h.day);
        c.append(&centered(dim(&h.label)));
        let i = d::tinted_icon(icon, IconSize::Small, Tone::Text);
        i.set_halign(gtk::Align::Center);
        c.append(&i);
        c.append(&centered(t(&weather::degrees(h.temp), Role::Body)));
        r.append(&c);
    }
    r
}

fn daily(f: &weather::Forecast) -> gtk::Box {
    let list = w::vbox(w::S1);
    let (min, max) = f.days.iter().fold((f64::MAX, f64::MIN), |(a, b), d| {
        (a.min(d.low), b.max(d.high))
    });
    for day in f.days.iter().take(weather::FORECAST_DAYS) {
        let r = w::hbox(w::S2);
        let name = t(&day.label, Role::Body);
        name.set_width_chars(weather::DAY_CHARS);
        name.set_xalign(0.0);
        r.append(&name);
        let (_, icon) = weather::condition(day.code, true);
        r.append(&d::tinted_icon(icon, IconSize::Small, Tone::Text));
        let lo = dim(&weather::degrees(day.low));
        lo.set_width_chars(weather::TEMP_CHARS);
        lo.set_xalign(1.0);
        r.append(&lo);
        r.append(&d::range_bar(day.low, day.high, min, max, Tone::Accent));
        let hi = t(&weather::degrees(day.high), Role::Body);
        hi.set_width_chars(weather::TEMP_CHARS);
        hi.set_xalign(1.0);
        r.append(&hi);
        list.append(&r);
    }
    list
}

fn weather_view(v: &WeatherView, size: Size, hooks: &Hooks) -> gtk::Box {
    match v {
        WeatherView::NoPlace => {
            let b = empty(
                "weather-few-clouds-symbolic",
                "Weather",
                if size == Size::Small {
                    "Choose a city."
                } else {
                    "Choose a city to see its forecast."
                },
                size,
            );
            let pick = hooks.choose_city.clone();
            let btn = w::button("Choose City…", w::Kind::Default, move || pick());
            btn.set_halign(gtk::Align::Center);
            btn.set_margin_top(w::S2);
            b.append(&btn);
            b
        }
        WeatherView::Loading(p) => {
            let b = w::vbox(0);
            b.append(&place_title(p, false));
            b.append(&t("—", Role::Display));
            b.append(&d::fill());
            b.append(&dim("Loading…"));
            b
        }
        WeatherView::Offline(p) => {
            let b = w::vbox(0);
            b.append(&place_title(p, true));
            b.append(&t("—", Role::Display));
            b.append(&d::fill());
            b.append(&note(
                if size == Size::Small {
                    "Offline"
                } else {
                    crate::copy::WEATHER_OFFLINE
                },
                2,
            ));
            b
        }
        WeatherView::Ready {
            place,
            forecast,
            stale,
        } => {
            let b = w::vbox(w::S3);
            if size == Size::Medium {
                b.set_valign(gtk::Align::Center);
            }
            b.append(&weather_now(place, forecast, stale.is_some(), size));
            if size != Size::Small {
                b.append(&hourly(forecast));
            }
            if size == Size::Large {
                // Days pinned to the bottom edge: equal insets top and bottom.
                b.append(&d::fill());
                b.append(&daily(forecast));
                if let Some(s) = stale {
                    b.append(&dim(s));
                }
            }
            b
        }
    }
}

// ---- Music ----

fn controls(p: &media::Playing, large: bool) -> gtk::Box {
    let r = w::hbox(if large { w::S4 } else { 0 });
    r.set_halign(gtk::Align::Center);
    let bus = p.bus.clone();
    let mk = |icon: &str, tip: &str, c: media::Control, on: bool| {
        let bus = bus.clone();
        let b = w::icon_button(icon, tip, move || media::control(&bus, c));
        b.add_css_class("media-button");
        b.set_sensitive(on);
        b
    };
    if large {
        r.append(&mk(
            "media-skip-backward-symbolic",
            "Previous",
            media::Control::Previous,
            p.can_prev,
        ));
    }
    r.append(&mk(
        if p.playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        },
        if p.playing { "Pause" } else { "Play" },
        media::Control::PlayPause,
        true,
    ));
    if large {
        r.append(&mk(
            "media-skip-forward-symbolic",
            "Next",
            media::Control::Next,
            p.can_next,
        ));
    }
    r
}

fn music(p: Option<&media::Playing>, size: Size) -> gtk::Box {
    let Some(p) = p else {
        return empty(
            "audio-x-generic-symbolic",
            "Not Playing",
            if size == Size::Small {
                "Play some music."
            } else {
                "Music and videos you play show here."
            },
            size,
        );
    };
    let b = w::vbox(w::S1);
    match size {
        Size::Small => {
            let top = w::hbox(w::S2);
            top.add_css_class("no-row-check");
            top.append(&d::cover(p.art.as_deref(), d::COVER_S));
            top.append(&w::spacer());
            let c = controls(p, false);
            c.set_valign(gtk::Align::Start);
            top.append(&c);
            b.append(&top);
            b.append(&d::fill());
            b.append(&t(&p.title, Role::Title));
            b.append(&dim(&p.artist));
        }
        Size::Medium => {
            let row = w::hbox(w::S4);
            row.add_css_class("no-row-check");
            row.set_vexpand(true);
            row.append(&d::cover(p.art.as_deref(), d::COVER_M));
            let col = w::vbox(w::S1);
            col.set_hexpand(true);
            col.append(&d::text(&p.player, Role::Caption, Tone::Dim));
            col.append(&t(&p.title, Role::Title));
            col.append(&dim(&p.artist));
            col.append(&d::fill());
            let c = controls(p, true);
            c.set_halign(gtk::Align::Start);
            col.append(&c);
            row.append(&col);
            b.append(&row);
        }
        Size::Large => {
            b.set_spacing(w::S3);
            let cover = d::cover(p.art.as_deref(), d::COVER_L);
            cover.set_halign(gtk::Align::Center);
            b.append(&cover);
            let title = centered(t(&p.title, Role::Title));
            b.append(&title);
            b.append(&centered(dim(&format!("{} · {}", p.artist, p.player))));
            b.append(&d::fill());
            b.append(&controls(p, true));
        }
    }
    b
}

// ---- Batteries ----

fn battery_tone(dv: &power::Device) -> Tone {
    if dv.charging {
        Tone::Success
    } else if dv.percent <= k::LOW_BATTERY {
        Tone::Warning
    } else {
        Tone::Text
    }
}

fn ring(dv: &power::Device, size: IconSize) -> gtk::Box {
    d::Gauge::new(
        dv.percent as f64 / 100.0,
        "",
        Some(dv.kind.icon()),
        Some(&format!("{}%", dv.percent)),
        battery_tone(dv),
        size,
    )
    .root
}

fn batteries(devs: &[power::Device], minutes: Option<u32>, size: Size) -> gtk::Box {
    if devs.is_empty() {
        return empty(
            "battery-missing-symbolic",
            "No Batteries",
            if size == Size::Small {
                "On mains power."
            } else {
                "This computer runs on mains power. Wireless devices show here when connected."
            },
            size,
        );
    }
    let b = w::vbox(w::S2);
    b.set_valign(gtk::Align::Center);
    match size {
        Size::Small if devs.len() == 1 => b.append(&ring(&devs[0], IconSize::Large)),
        Size::Small => {
            for pair in devs.chunks(2).take(2) {
                let r = w::hbox(w::S2);
                r.set_homogeneous(true);
                for dv in pair {
                    r.append(&ring(dv, IconSize::Small));
                }
                b.append(&r);
            }
        }
        Size::Medium => {
            let r = w::hbox(w::S6);
            r.set_halign(gtk::Align::Center);
            for dv in devs.iter().take(4) {
                r.append(&ring(dv, IconSize::Medium));
            }
            b.append(&r);
        }
        Size::Large => {
            // Rings across the top, the same devices as named rows anchored to the bottom.
            b.set_valign(gtk::Align::Fill);
            let rings = w::hbox(w::S6);
            rings.set_halign(gtk::Align::Center);
            for dv in devs.iter().take(4) {
                rings.append(&ring(dv, IconSize::Medium));
            }
            b.append(&rings);
            b.append(&d::fill());
            let list = w::vbox(w::S3);
            for dv in devs.iter().take(4) {
                let row = w::hbox(w::S3);
                row.append(&d::tinted_icon(dv.kind.icon(), IconSize::Small, Tone::Text));
                let col = w::vbox(w::S1);
                col.set_hexpand(true);
                let head = w::hbox(w::S2);
                head.append(&t(&dv.name, Role::Body));
                head.append(&w::spacer());
                let detail = match (dv.kind, minutes) {
                    (power::DeviceKind::Computer, Some(_)) => format!(
                        "{}% · {}",
                        dv.percent,
                        k::battery_detail(
                            if dv.charging {
                                crate::status::BatteryStatus::Charging
                            } else {
                                crate::status::BatteryStatus::Discharging
                            },
                            minutes,
                        )
                    ),
                    _ => format!("{}%", dv.percent),
                };
                head.append(&dim(&detail));
                col.append(&head);
                col.append(&d::meter(dv.percent as f64 / 100.0, battery_tone(dv)));
                row.append(&col);
                list.append(&row);
            }
            b.append(&list);
        }
    }
    b
}

// ---- System ----

fn pct(x: f64) -> String {
    format!("{:.0}%", x.clamp(0.0, 100.0))
}

fn system(s: &SystemView, size: Size) -> gtk::Box {
    let cpu = s.cpu.last().copied().unwrap_or(0.0);
    let mem = if s.mem_total > 0 {
        s.mem_used as f64 / s.mem_total as f64
    } else {
        0.0
    };
    let used = crate::widgets::gigabytes(s.mem_used);
    let total = crate::widgets::gigabytes(s.mem_total);
    let b = w::vbox(w::S1);
    match size {
        Size::Small => {
            b.set_valign(gtk::Align::Center);
            b.set_spacing(w::S3);
            let r = w::hbox(w::S2);
            r.set_homogeneous(true);
            r.append(
                &d::Gauge::new(
                    cpu / 100.0,
                    &pct(cpu),
                    None,
                    Some("CPU"),
                    Tone::Accent,
                    IconSize::Medium,
                )
                .root,
            );
            r.append(
                &d::Gauge::new(
                    mem,
                    &pct(mem * 100.0),
                    None,
                    Some("Memory"),
                    Tone::Text,
                    IconSize::Medium,
                )
                .root,
            );
            b.append(&r);
            b.append(&centered(dim(&format!("{used} of {total}"))));
        }
        Size::Medium => {
            let r = w::hbox(w::S6);
            r.set_homogeneous(true);
            r.add_css_class("no-row-check");
            let c1 = w::vbox(0);
            c1.append(&d::text("CPU", Role::Caption, Tone::Dim));
            c1.append(&t(&pct(cpu), Role::Display));
            c1.append(&d::fill());
            c1.append(
                &d::Chart::new(
                    d::ChartKind::Line,
                    s.cpu.clone(),
                    Some(100.0),
                    Tone::Accent,
                    IconSize::Small,
                )
                .area,
            );
            r.append(&c1);
            let c2 = w::vbox(0);
            c2.append(&d::text("Memory", Role::Caption, Tone::Dim));
            c2.append(&t(&used, Role::Display));
            c2.append(&d::fill());
            c2.append(&d::meter(mem, Tone::Text));
            let of = dim(&format!("of {total}"));
            of.set_margin_top(w::S1);
            c2.append(&of);
            r.append(&c2);
            b.append(&r);
        }
        Size::Large => {
            b.append(&d::text("CPU", Role::Caption, Tone::Dim));
            b.append(&t(&pct(cpu), Role::Display));
            b.append(
                &d::Chart::new(
                    d::ChartKind::Line,
                    s.cpu.clone(),
                    Some(100.0),
                    Tone::Accent,
                    IconSize::Large,
                )
                .area,
            );
            let m = d::text("Memory", Role::Caption, Tone::Dim);
            m.set_margin_top(w::S3);
            b.append(&m);
            b.append(&t(&format!("{used} of {total}"), Role::Title));
            b.append(&d::meter(mem, Tone::Text));
            b.append(&d::fill());
            let foot = w::hbox(w::S2);
            if let Some((a, bb, c)) = s.load {
                foot.append(&dim(&format!("Load {a:.2} {bb:.2} {c:.2}")));
            }
            foot.append(&w::spacer());
            if let Some(u) = s.uptime {
                foot.append(&dim(&format!("Up {}", crate::widgets::uptime(u))));
            }
            b.append(&foot);
        }
    }
    b
}

// ---- Sample data (gallery previews, goldens) ----

/// Mon 26 October 2026, 14:05 (`kit::FAKE_NOW`).
pub fn sample_now() -> Now {
    Now {
        y: 2026,
        month: 10,
        day: 26,
        weekday: 0,
        hour: 14,
        minute: 5,
    }
}

pub fn sample_place() -> Place {
    Place {
        name: "Lisbon".into(),
        region: "Lisbon, Portugal".into(),
        lat: 38.717,
        lon: -9.133,
    }
}

pub fn sample_forecast() -> weather::Forecast {
    let hour = |label: &str, temp: f64, code: u8| weather::Hour {
        label: label.into(),
        temp,
        code,
        day: true,
    };
    let day = |label: &str, low: f64, high: f64, code: u8| weather::Day {
        label: label.into(),
        low,
        high,
        code,
    };
    weather::Forecast {
        temp: 19.0,
        code: 2,
        day: true,
        high: 21.0,
        low: 13.0,
        hours: vec![
            hour("Now", 19.0, 2),
            hour("15:00", 20.0, 1),
            hour("16:00", 21.0, 0),
            hour("17:00", 20.0, 0),
            hour("18:00", 18.0, 2),
            hour("19:00", 16.0, 3),
        ],
        days: vec![
            day("Today", 13.0, 21.0, 2),
            day("Tue", 12.0, 19.0, 61),
            day("Wed", 11.0, 17.0, 80),
            day("Thu", 10.0, 18.0, 3),
            day("Fri", 12.0, 22.0, 0),
            day("Sat", 14.0, 23.0, 1),
        ],
    }
}

pub fn sample_events() -> Vec<calendar::Event> {
    let e = |title: &str, h: u32, m: u32| calendar::Event {
        title: title.into(),
        location: String::new(),
        start: Some((h, m)),
        end: Some((h + 1, m)),
    };
    vec![
        e("Tea with the Hatter", 16, 0),
        e("Croquet practice", 17, 30),
        e("Read to Dinah", 21, 0),
    ]
}

pub fn sample_playing() -> media::Playing {
    media::Playing {
        player: "Music".into(),
        bus: String::new(),
        title: "Golden Afternoon".into(),
        artist: "The Looking Glass".into(),
        art: None,
        playing: true,
        can_next: true,
        can_prev: true,
    }
}

pub fn sample_devices() -> Vec<power::Device> {
    let dv = |name: &str, kind, percent, charging| power::Device {
        name: name.into(),
        kind,
        percent,
        charging,
    };
    vec![
        dv("This Computer", power::DeviceKind::Computer, 82, false),
        dv("Headphones", power::DeviceKind::Headphones, 64, true),
        dv("Mouse", power::DeviceKind::Mouse, 18, false),
    ]
}

pub fn sample_system() -> SystemView {
    SystemView {
        cpu: vec![
            12.0, 18.0, 15.0, 22.0, 35.0, 28.0, 19.0, 24.0, 41.0, 37.0, 26.0, 21.0, 18.0, 23.0,
            30.0, 27.0, 22.0, 19.0, 25.0, 23.0,
        ],
        mem_used: 6_550_000_000,
        mem_total: 17_179_869_184,
        load: Some((1.24, 0.98, 0.71)),
        uptime: Some(273_600.0),
    }
}

/// Representative data for a built-in (gallery previews).
pub fn sample(b: Builtin) -> Data {
    match b {
        Builtin::Clock => Data::Clock {
            now: sample_now(),
            face: Face::Analog,
        },
        Builtin::Calendar => Data::Calendar {
            now: sample_now(),
            events: Some(sample_events()),
        },
        Builtin::Weather => Data::Weather(WeatherView::Ready {
            place: sample_place(),
            forecast: sample_forecast(),
            stale: None,
        }),
        Builtin::Music => Data::Music(Some(sample_playing())),
        Builtin::Battery => Data::Batteries {
            devices: sample_devices(),
            minutes: Some(191),
        },
        Builtin::System => Data::System(sample_system()),
    }
}
