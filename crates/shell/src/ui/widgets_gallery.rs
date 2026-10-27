//! Edit Widgets gallery: every built-in and installed package, previewed in each size with sample
//! data (sources never run here). Click a preview to put it on the desktop. Weather adds a city
//! search (Open-Meteo geocoding, only what the user types is sent).
use super::widgets_builtin as b;
use super::widgets_layout as lay;
use crate::kit::data as d;
use crate::kit::widgets as w;
use gtk::{glib, prelude::*};
use shared::widgets::{Builtin, Kind, Place, Size};
use std::cell::RefCell;
use std::rc::Rc;
use wol_core::widgets::Package;

/// A gallery entry: a built-in or a package (valid or not; an invalid one previews its errors).
#[derive(Clone)]
pub enum Entry {
    Builtin(Builtin),
    Package(Rc<Package>),
}

impl Entry {
    pub fn kind(&self) -> Kind {
        match self {
            Entry::Builtin(b) => Kind::Builtin(*b),
            Entry::Package(p) => Kind::Custom(p.id.clone()),
        }
    }
    pub fn name(&self) -> String {
        match self {
            Entry::Builtin(b) => b.name().into(),
            Entry::Package(p) => p.name().into(),
        }
    }
    fn description(&self) -> String {
        match self {
            Entry::Builtin(b) => b.description().into(),
            Entry::Package(p) => match &p.manifest {
                Ok(m) if !m.description.is_empty() => m.description.clone(),
                Ok(_) => String::new(),
                Err(_) => "This widget has problems. Fix its widget.json and reload.".into(),
            },
        }
    }
    fn icon(&self) -> String {
        match self {
            Entry::Builtin(b) => builtin_icon(*b).into(),
            Entry::Package(p) => match &p.manifest {
                Ok(m) => lay::icon_name(&m.icon).into(),
                Err(_) => "dialog-warning-symbolic".into(),
            },
        }
    }
    pub fn sizes(&self) -> Vec<Size> {
        match self {
            Entry::Builtin(x) => b::sizes(*x).to_vec(),
            Entry::Package(p) => match &p.manifest {
                Ok(m) => m.sizes.clone(),
                Err(_) => vec![Size::Small, Size::Medium],
            },
        }
    }
}

pub fn builtin_icon(b: Builtin) -> &'static str {
    match b {
        Builtin::Clock => "preferences-system-time-symbolic",
        Builtin::Calendar => "x-office-calendar-symbolic",
        Builtin::Weather => "weather-few-clouds-symbolic",
        Builtin::Music => "audio-x-generic-symbolic",
        Builtin::Battery => "battery-good-symbolic",
        Builtin::System => "computer-symbolic",
    }
}

/// Preview card for `e` at `s` (sample data, not interactive).
pub fn preview(e: &Entry, s: Size) -> gtk::Box {
    let c = match e {
        Entry::Builtin(x) => b::card(&b::sample(*x), s, &b::Hooks::default()),
        Entry::Package(p) => match &p.manifest {
            Ok(m) => {
                let data = m.sample.clone().unwrap_or(serde_json::Value::Null);
                let cache = std::path::PathBuf::new();
                let cx = lay::Ctx {
                    dir: &p.dir,
                    cache: &cache,
                    interactive: false,
                };
                lay::card(m, s, &data, &cx)
            }
            Err(problems) => lay::error_card(
                s,
                p.name(),
                &crate::copy::widget_problems(p.name()),
                &lay::problem_lines(problems),
            ),
        },
    };
    c.set_can_target(false);
    c
}

pub type AddFn = Box<dyn Fn(Kind, Size, Option<Place>) -> Result<(), String>>;
pub type PlaceFn = Box<dyn Fn(&str, Place)>;
pub type Found = Box<dyn Fn(Result<Vec<Place>, String>)>;
pub type SearchFn = Box<dyn Fn(String, Found)>;

/// What the gallery asks of the desktop.
pub struct Actions {
    /// Put a widget on the desktop (Weather with the city picked here, if any).
    pub add: AddFn,
    /// A city picked while choosing one for an existing Weather widget.
    pub set_place: PlaceFn,
    pub done: Box<dyn Fn()>,
    /// Geocoding search, answered on the main loop (`results`).
    pub search: SearchFn,
}

pub struct Gallery {
    pub root: gtk::Box,
    list: gtk::Box,
    page: gtk::Box,
    status: gtk::Label,
    entries: RefCell<Vec<Entry>>,
    selected: RefCell<usize>,
    /// Size previewed on the selected entry's page.
    size: RefCell<Size>,
    /// Weather widget whose city is being chosen.
    target: RefCell<Option<String>>,
    picked: RefCell<Option<Place>>,
    actions: Actions,
}

impl Gallery {
    pub fn new(entries: Vec<Entry>, actions: Actions) -> Rc<Gallery> {
        let root = w::glass_panel(w::S3);
        root.add_css_class("widget-gallery");
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::End);
        let head = w::hbox(w::S2);
        head.append(&d::text("Widgets", d::Role::Large, d::Tone::Text));
        head.append(&w::spacer());
        let status = d::text("", d::Role::Secondary, d::Tone::Dim);
        head.append(&status);
        let done_btn = w::button("Done", w::Kind::Primary, || {});
        head.append(&done_btn);
        root.append(&head);
        let body = w::hbox(w::S4);
        body.add_css_class("no-row-check");
        let list = w::vbox(0);
        list.set_size_request(w::SIDEBAR_WIDTH, -1);
        body.append(&d::scroller(&list));
        let page = w::vbox(w::S3);
        page.set_hexpand(true);
        body.append(&page);
        root.append(&body);
        let g = Rc::new(Gallery {
            root,
            list,
            page,
            status,
            entries: RefCell::new(entries),
            selected: RefCell::new(0),
            size: RefCell::new(Size::Small),
            target: RefCell::new(None),
            picked: RefCell::new(None),
            actions,
        });
        let g2 = Rc::downgrade(&g);
        done_btn.connect_clicked(move |_| {
            if let Some(g) = g2.upgrade() {
                (g.actions.done)();
            }
        });
        g.rebuild_list();
        g.show_page();
        g
    }

    pub fn set_entries(self: &Rc<Self>, e: Vec<Entry>) {
        *self.entries.borrow_mut() = e;
        let n = self.entries.borrow().len();
        let mut s = self.selected.borrow_mut();
        *s = (*s).min(n.saturating_sub(1));
        drop(s);
        self.rebuild_list();
        self.show_page();
    }

    /// Opens on `kind`; with `target`, a Weather widget's city is being chosen.
    pub fn select(self: &Rc<Self>, kind: &Kind, target: Option<String>) {
        let i = self.entries.borrow().iter().position(|e| &e.kind() == kind);
        if let Some(i) = i {
            *self.selected.borrow_mut() = i;
            *self.size.borrow_mut() = Size::Small;
        }
        *self.target.borrow_mut() = target;
        self.status.set_label("");
        self.rebuild_list();
        self.show_page();
    }

    fn rebuild_list(self: &Rc<Self>) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let sel = *self.selected.borrow();
        for (i, e) in self.entries.borrow().iter().enumerate() {
            let row = w::list_button(&e.icon(), &e.name(), "");
            if i == sel {
                row.add_css_class("selected");
            }
            let g = Rc::downgrade(self);
            row.connect_clicked(move |_| {
                if let Some(g) = g.upgrade() {
                    *g.selected.borrow_mut() = i;
                    *g.size.borrow_mut() = Size::Small;
                    *g.target.borrow_mut() = None;
                    g.status.set_label("");
                    g.rebuild_list();
                    g.show_page();
                }
            });
            self.list.append(&row);
        }
    }

    fn show_page(self: &Rc<Self>) {
        while let Some(c) = self.page.first_child() {
            self.page.remove(&c);
        }
        let Some(e) = self.entries.borrow().get(*self.selected.borrow()).cloned() else {
            return;
        };
        let head = w::vbox(0);
        head.append(&d::text(&e.name(), d::Role::Title, d::Tone::Text));
        let desc = e.description();
        if !desc.is_empty() {
            head.append(&d::text(&desc, d::Role::Secondary, d::Tone::Dim));
        }
        self.page.append(&head);
        if matches!(e, Entry::Builtin(Builtin::Weather)) {
            self.page.append(&self.city_picker());
        }
        let sizes = e.sizes();
        let size = {
            let mut cur = self.size.borrow_mut();
            if !sizes.contains(&cur) {
                *cur = sizes.first().copied().unwrap_or(Size::Small);
            }
            *cur
        };
        let labels: Vec<&str> = sizes.iter().map(|s| s.label()).collect();
        let at = sizes.iter().position(|s| *s == size).unwrap_or(0);
        let g = Rc::downgrade(self);
        let picked = sizes.clone();
        let seg = w::segmented(&labels, at, move |i| {
            let Some(g) = g.upgrade() else { return };
            if let Some(s) = picked.get(i).copied()
                && *g.size.borrow() != s
            {
                *g.size.borrow_mut() = s;
                // Rebuilt outside the toggle handler of the button being replaced.
                let g = Rc::downgrade(&g);
                glib::idle_add_local_once(move || {
                    if let Some(g) = g.upgrade() {
                        g.show_page();
                    }
                });
            }
        });
        seg.set_halign(gtk::Align::Start);
        self.page.append(&seg);
        // The stage is the largest card's size, so the gallery never resizes between sizes.
        let card = preview(&e, size);
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        let stage = d::layers(&card);
        stage.add_css_class("gallery-stage");
        let (wd, ht) = d::card_size(Size::Large);
        stage.set_size_request(wd, ht);
        stage.set_tooltip_text(Some(&format!("Add {} ({})", e.name(), size.label())));
        let kind = e.kind();
        let g = Rc::downgrade(self);
        let go = Rc::new(move || {
            if let Some(g) = g.upgrade() {
                g.add(kind.clone(), size);
            }
        });
        let go2 = go.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| go2());
        stage.add_controller(click);
        self.page.append(&stage);
        let foot = w::hbox(w::S2);
        foot.append(&w::spacer());
        foot.append(&w::button("Add Widget", w::Kind::Default, move || go()));
        self.page.append(&foot);
    }

    fn add(&self, kind: Kind, s: Size) {
        let place = self.picked.borrow().clone();
        match (self.actions.add)(kind, s, place) {
            Ok(()) => self.status.set_label(""),
            Err(why) => self.status.set_label(&why),
        }
    }

    fn city_picker(self: &Rc<Self>) -> gtk::Box {
        let b = w::vbox(w::S2);
        let field = w::search_field("Search for a city");
        field.set_hexpand(true);
        let target = self.target.borrow().clone();
        let picked = self.picked.borrow().clone();
        let hint = match (&target, &picked) {
            (Some(_), _) => "Choose the city this widget shows.".to_string(),
            (None, Some(p)) => format!("New Weather widgets show {}.", p.name),
            (None, None) => "Pick a city before adding, or choose one on the widget.".to_string(),
        };
        b.append(&field);
        let results = w::vbox(0);
        b.append(&results);
        b.append(&w::footnote(&format!(
            "{hint} Forecasts come from Open-Meteo: only the city you search for is sent."
        )));
        let g = Rc::downgrade(self);
        field.connect_search_changed(move |f| {
            let q = f.text().trim().to_string();
            let Some(g) = g.upgrade() else { return };
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            if q.chars().count() < 2 {
                return;
            }
            let (results, gw, field) = (results.clone(), Rc::downgrade(&g), f.clone());
            (g.actions.search)(
                q.clone(),
                Box::new(move |r| {
                    // A newer search replaced this one.
                    if field.text().trim() != q {
                        return;
                    }
                    let Some(g) = gw.upgrade() else { return };
                    while let Some(c) = results.first_child() {
                        results.remove(&c);
                    }
                    match r {
                        Ok(places) if places.is_empty() => {
                            results.append(&w::footnote("No city by that name."))
                        }
                        Ok(places) => {
                            for p in places.into_iter().take(5) {
                                let row =
                                    w::list_button("mark-location-symbolic", &p.name, &p.region);
                                let gw = Rc::downgrade(&g);
                                row.connect_clicked(move |_| {
                                    if let Some(g) = gw.upgrade() {
                                        g.pick(p.clone());
                                    }
                                });
                                results.append(&row);
                            }
                        }
                        Err(e) => results.append(&w::footnote(&e)),
                    }
                }),
            );
        });
        b
    }

    fn pick(self: &Rc<Self>, p: Place) {
        let target = self.target.borrow_mut().take();
        match target {
            Some(id) => {
                (self.actions.set_place)(&id, p.clone());
                self.status.set_label(&format!("Showing {}", p.name));
            }
            None => {
                self.status
                    .set_label(&format!("New Weather widgets show {}", p.name));
                *self.picked.borrow_mut() = Some(p);
            }
        }
        let g = Rc::downgrade(self);
        // Rebuilt outside the click handler of the row being removed.
        glib::idle_add_local_once(move || {
            if let Some(g) = g.upgrade() {
                g.show_page();
            }
        });
    }
}
