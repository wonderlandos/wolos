//! Desktop widgets (ADR 0010): a layer surface above the wallpaper and below windows, inside the
//! menu bar and Dock zones. Cards sit on a snapping grid saved in `widgets.json`; Edit Widgets
//! (desktop right-click, a card's menu, `wol shell show widgets`) raises the surface, shows remove
//! buttons and the gallery, and lets cards be dragged. Calm: one coalesced timer for every
//! widget, music and batteries are event-driven, and nothing runs while a fullscreen window
//! covers the desktop.
use super::widgets_builtin::{self as builtin, Data, SystemView, WeatherView};
use super::widgets_gallery::{Actions, Entry, Gallery};
use super::widgets_layout as lay;
use crate::kit::data as d;
use crate::kit::surface;
use crate::kit::widgets as w;
use crate::widgets::{
    self as wd, Now, calendar, grid::Grid, media, power, schedule::Schedule, weather,
};
use gtk::{gio, glib, prelude::*};
use serde_json::Value;
use shared::widgets::manifest::{Manifest, Source};
use shared::widgets::{Builtin, Desktop, Face, Kind, Options, Place, Placed, Size, Units};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use wol_core::widgets::Package;

/// What a card shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Builtin(Data),
    Custom {
        manifest: Rc<Manifest>,
        dir: PathBuf,
        data: Value,
    },
    /// A sentence from `crate::copy`; `details` (paths, exit codes) only behind Show Details.
    Error {
        name: String,
        message: String,
        details: Vec<String>,
    },
    /// An http source the user has not allowed yet.
    Consent {
        name: String,
        host: String,
    },
}

pub type Listener = Rc<dyn Fn(Change)>;
/// A card menu action, run with the board.
type BoardFn = Box<dyn Fn(&Rc<Board>)>;

/// Board -> provider notifications.
pub enum Change {
    Added(String),
    Removed(Placed),
    /// Size or options changed.
    Updated(String),
    /// The user allowed a widget's network host.
    Granted(String),
    Edit(bool),
}

struct Drag {
    id: String,
    x0: f64,
    y0: f64,
    target: (u16, u16),
}

struct St {
    file: Option<PathBuf>,
    desk: Desktop,
    grid: Grid,
    content: HashMap<String, Content>,
    cards: HashMap<String, gtk::Box>,
    removes: HashMap<String, gtk::Button>,
    packages: Vec<Rc<Package>>,
    edit: bool,
    drag: Option<Drag>,
    mark: Option<gtk::Box>,
}

/// The desktop: canvas of cards, drag and drop, menus, Edit Widgets and its gallery.
pub struct Board {
    pub root: gtk::Overlay,
    canvas: gtk::Fixed,
    pub gallery: Rc<Gallery>,
    st: RefCell<St>,
    listener: RefCell<Option<Listener>>,
    win: RefCell<Option<gtk::Window>>,
}

fn entries(packages: &[Rc<Package>]) -> Vec<Entry> {
    Builtin::ALL
        .into_iter()
        .map(Entry::Builtin)
        .chain(packages.iter().cloned().map(Entry::Package))
        .collect()
}

/// Sizes a placed widget may take.
fn sizes_of(kind: &Kind, packages: &[Rc<Package>], current: Size) -> Vec<Size> {
    match kind {
        Kind::Builtin(b) => builtin::sizes(*b).to_vec(),
        Kind::Custom(id) => packages
            .iter()
            .find(|p| &p.id == id)
            .and_then(|p| p.manifest.as_ref().ok())
            .map(|m| m.sizes.clone())
            .unwrap_or_else(|| vec![current]),
        Kind::Unknown(_) => vec![current],
    }
}

impl Board {
    pub fn new(desk: Desktop, file: Option<PathBuf>, packages: Vec<Package>) -> Rc<Board> {
        let packages: Vec<Rc<Package>> = packages.into_iter().map(Rc::new).collect();
        Rc::new_cyclic(|me: &Weak<Board>| {
            let m = me.clone();
            let probe = d::size_probe(move |wd, ht| {
                if let Some(b) = m.upgrade() {
                    b.resize(wd, ht);
                }
            });
            let root = d::layers(&probe);
            let canvas = d::canvas();
            canvas.set_halign(gtk::Align::Fill);
            canvas.set_valign(gtk::Align::Fill);
            root.add_overlay(&canvas);
            let (m1, m2, m3, m4) = (me.clone(), me.clone(), me.clone(), me.clone());
            let gallery = Gallery::new(
                entries(&packages),
                Actions {
                    add: Box::new(move |k, s, p| m1.upgrade().map_or(Ok(()), |b| b.add(k, s, p))),
                    set_place: Box::new(move |id, p| {
                        if let Some(b) = m2.upgrade() {
                            b.set_place(id, p);
                        }
                    }),
                    done: Box::new(move || {
                        if let Some(b) = m3.upgrade() {
                            b.set_edit(false);
                        }
                    }),
                    search: Box::new(move |q, done| {
                        if m4.upgrade().is_some() {
                            search_places(q, done);
                        }
                    }),
                },
            );
            gallery.root.set_visible(false);
            root.add_overlay(&gallery.root);
            let grid = Grid::new(1, 1, d::card_metrics());
            Board {
                root,
                canvas,
                gallery,
                st: RefCell::new(St {
                    file,
                    desk,
                    grid,
                    content: HashMap::new(),
                    cards: HashMap::new(),
                    removes: HashMap::new(),
                    packages,
                    edit: false,
                    drag: None,
                    mark: None,
                }),
                listener: RefCell::new(None),
                win: RefCell::new(None),
            }
        })
        .wired()
    }

    fn wired(self: Rc<Self>) -> Rc<Self> {
        // Right-click on the bare desktop.
        let menu = gtk::GestureClick::new();
        menu.set_button(gtk::gdk::BUTTON_SECONDARY);
        let me = Rc::downgrade(&self);
        menu.connect_pressed(move |_, _, x, y| {
            let Some(b) = me.upgrade() else { return };
            let edit = b.st.borrow().edit;
            let m2 = Rc::downgrade(&b);
            d::popup_menu(
                &b.canvas,
                x,
                y,
                vec![
                    (
                        if edit {
                            "Done Editing"
                        } else {
                            "Edit Widgets…"
                        }
                        .to_string(),
                        None,
                        Box::new(move || {
                            if let Some(b) = m2.upgrade() {
                                b.set_edit(!edit);
                            }
                        }),
                    ),
                    (
                        "Change Wallpaper…".to_string(),
                        None,
                        Box::new(|| {
                            crate::kit::spawn(&["wol-settings".into(), "Wallpaper".into()])
                        }),
                    ),
                ],
            );
        });
        self.canvas.add_controller(menu);
        // Edit Widgets: drag a card to another cell.
        let drag = gtk::GestureDrag::new();
        let me = Rc::downgrade(&self);
        drag.connect_drag_begin(move |g, x, y| {
            if let Some(b) = me.upgrade()
                && !b.drag_begin(x, y)
            {
                g.set_state(gtk::EventSequenceState::Denied);
            }
        });
        let me = Rc::downgrade(&self);
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(b) = me.upgrade() {
                b.drag_update(dx, dy);
            }
        });
        let me = Rc::downgrade(&self);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(b) = me.upgrade() {
                b.drag_end();
            }
        });
        self.canvas.add_controller(drag);
        self
    }

    pub fn set_listener(&self, f: Listener) {
        *self.listener.borrow_mut() = Some(f);
    }

    pub fn set_window(&self, win: &gtk::Window) {
        *self.win.borrow_mut() = Some(win.clone());
    }

    fn notify(&self, c: Change) {
        let l = self.listener.borrow().clone();
        if let Some(l) = l {
            l(c);
        }
    }

    pub fn widgets(&self) -> Vec<Placed> {
        self.st.borrow().desk.widgets.clone()
    }

    pub fn desktop(&self) -> Desktop {
        self.st.borrow().desk.clone()
    }

    pub fn packages(&self) -> Vec<Rc<Package>> {
        self.st.borrow().packages.clone()
    }

    pub fn is_editing(&self) -> bool {
        self.st.borrow().edit
    }

    fn save(&self) {
        let st = self.st.borrow();
        if let Some(f) = &st.file
            && let Err(e) = wd::save(f, &st.desk)
        {
            eprintln!("wol-shell: {}: {e}", f.display());
        }
    }

    /// The work area changed size: recompute the grid and place every card.
    pub fn resize(self: &Rc<Self>, wd: i32, ht: i32) {
        {
            let mut st = self.st.borrow_mut();
            let g = Grid::new(wd, ht, d::card_metrics());
            if g == st.grid {
                return;
            }
            st.grid = g;
        }
        self.layout();
    }

    /// Display positions (the saved layout, moved inward on a smaller screen).
    fn shown(&self) -> Vec<Placed> {
        let st = self.st.borrow();
        let at = st.grid.arrange(&st.desk.widgets);
        st.desk
            .widgets
            .iter()
            .zip(at)
            .map(|(p, (col, row))| Placed {
                col,
                row,
                ..p.clone()
            })
            .collect()
    }

    /// Puts every card at its cell; builds cards that do not exist yet.
    fn layout(self: &Rc<Self>) {
        let shown = self.shown();
        let ids: HashSet<String> = shown.iter().map(|p| p.id.clone()).collect();
        let stale: Vec<String> = self
            .st
            .borrow()
            .cards
            .keys()
            .filter(|k| !ids.contains(*k))
            .cloned()
            .collect();
        for id in stale {
            self.drop_card(&id);
        }
        for p in &shown {
            if !self.st.borrow().cards.contains_key(&p.id) {
                self.build_card(p);
            }
            self.place(p);
        }
        self.clear_gallery(&shown);
    }

    /// Keeps the gallery in the free side of the desktop, so it never covers a card.
    fn clear_gallery(&self, shown: &[Placed]) {
        let g = self.st.borrow().grid;
        let total = 2 * g.m.margin + g.cols as i32 * g.m.pitch() - g.m.gap;
        let spans = shown.iter().map(|p| {
            let (x, _) = g.m.origin(p.col, p.row);
            (x, x + g.m.size(p.size).0)
        });
        let (start, end) = gallery_margins(spans, total, g.m.margin);
        self.gallery.root.set_margin_start(start);
        self.gallery.root.set_margin_end(end);
    }

    fn origin(&self, p: &Placed) -> (f64, f64) {
        let (x, y) = self.st.borrow().grid.m.origin(p.col, p.row);
        (x as f64, y as f64)
    }

    fn place(&self, p: &Placed) {
        let (x, y) = self.origin(p);
        let st = self.st.borrow();
        if let Some(c) = st.cards.get(&p.id) {
            if c.parent().is_some() {
                self.canvas.move_(c, x, y);
            } else {
                self.canvas.put(c, x, y);
            }
        }
        if let Some(r) = st.removes.get(&p.id) {
            let off = (w::S2 as f64) - (w::S4 as f64);
            if r.parent().is_some() {
                self.canvas.move_(r, x + off, y + off);
            } else {
                self.canvas.put(r, x + off, y + off);
            }
            r.insert_before(&self.canvas, None::<&gtk::Widget>);
        }
    }

    fn drop_card(&self, id: &str) {
        let mut st = self.st.borrow_mut();
        if let Some(c) = st.cards.remove(id) {
            self.canvas.remove(&c);
        }
        if let Some(r) = st.removes.remove(id) {
            self.canvas.remove(&r);
        }
    }

    fn render(self: &Rc<Self>, p: &Placed) -> gtk::Box {
        let st = self.st.borrow();
        let c = match (st.content.get(&p.id), &p.kind) {
            (Some(Content::Builtin(data)), _) => {
                let me = Rc::downgrade(self);
                let id = p.id.clone();
                let hooks = builtin::Hooks {
                    choose_city: Rc::new(move || {
                        if let Some(b) = me.upgrade() {
                            b.choose_city(&id);
                        }
                    }),
                };
                builtin::card(data, p.size, &hooks)
            }
            (
                Some(Content::Custom {
                    manifest,
                    dir,
                    data,
                }),
                Kind::Custom(pkg),
            ) => {
                let cache = wd::cache_home().join("wolos/widgets").join(pkg);
                let cx = lay::Ctx {
                    dir,
                    cache: &cache,
                    interactive: !st.edit,
                };
                lay::card(manifest, p.size, data, &cx)
            }
            (
                Some(Content::Error {
                    name,
                    message,
                    details,
                }),
                _,
            ) => lay::error_card(p.size, name, message, details),
            (Some(Content::Consent { name, host }), _) => {
                let me = Rc::downgrade(self);
                let id = p.id.clone();
                lay::consent_card(p.size, name, host, move || {
                    if let Some(b) = me.upgrade() {
                        b.grant(&id);
                    }
                })
            }
            // Loading: the empty glass, so nothing jumps when data arrives.
            _ => d::card(p.size, &d::fill()),
        };
        if st.edit
            && let Some(body) = c.first_child()
        {
            body.set_can_target(false);
        }
        c
    }

    fn build_card(self: &Rc<Self>, p: &Placed) {
        let c = self.render(p);
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_SECONDARY);
        let (me, id) = (Rc::downgrade(self), p.id.clone());
        click.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(b) = me.upgrade() {
                b.card_menu(&id, x, y);
            }
        });
        c.add_controller(click);
        let edit = self.st.borrow().edit;
        let mut st = self.st.borrow_mut();
        st.cards.insert(p.id.clone(), c);
        if edit && !st.removes.contains_key(&p.id) {
            let r = d::corner_button("list-remove-symbolic", "Remove Widget");
            let (me, id) = (Rc::downgrade(self), p.id.clone());
            r.connect_clicked(move |_| {
                if let Some(b) = me.upgrade() {
                    b.remove(&id);
                }
            });
            st.removes.insert(p.id.clone(), r);
        }
    }

    /// Rebuilds one card (new data, size or mode) where it stands.
    fn refresh_card(self: &Rc<Self>, id: &str) {
        let Some(p) = self.shown().into_iter().find(|p| p.id == id) else {
            return;
        };
        if let Some(old) = self.st.borrow_mut().cards.remove(id) {
            self.canvas.remove(&old);
        }
        self.build_card(&p);
        self.place(&p);
    }

    /// New content for a widget; unchanged content changes nothing on screen.
    pub fn set_content(self: &Rc<Self>, id: &str, c: Content) {
        {
            let mut st = self.st.borrow_mut();
            if st.content.get(id) == Some(&c) {
                return;
            }
            st.content.insert(id.to_string(), c);
        }
        if self.st.borrow().drag.as_ref().is_some_and(|d| d.id == id) {
            return;
        }
        self.refresh_card(id);
    }

    pub fn content(&self, id: &str) -> Option<Content> {
        self.st.borrow().content.get(id).cloned()
    }

    pub fn add(
        self: &Rc<Self>,
        kind: Kind,
        size: Size,
        place: Option<Place>,
    ) -> Result<(), String> {
        let shown = self.shown();
        let id = {
            let mut st = self.st.borrow_mut();
            let Some((col, row)) = st.grid.free_spot(&shown, None, size, (0, 0)) else {
                return Err("No room. Remove a widget or pick a smaller size.".into());
            };
            let id = st.desk.new_id(&kind);
            let mut options = Options::default();
            if kind == Kind::Builtin(Builtin::Weather) {
                options.place = place;
            }
            st.desk.widgets.push(Placed {
                id: id.clone(),
                kind,
                size,
                col,
                row,
                options,
            });
            id
        };
        self.save();
        self.layout();
        self.notify(Change::Added(id));
        Ok(())
    }

    pub fn remove(self: &Rc<Self>, id: &str) {
        let removed = {
            let mut st = self.st.borrow_mut();
            st.content.remove(id);
            st.desk.remove(id)
        };
        self.drop_card(id);
        self.save();
        if let Some(p) = removed {
            self.notify(Change::Removed(p));
        }
    }

    fn update(self: &Rc<Self>, id: &str, f: impl FnOnce(&mut Placed, &Grid, &[Placed])) {
        let shown = self.shown();
        {
            let mut st = self.st.borrow_mut();
            let grid = st.grid;
            let Some(p) = st.desk.get_mut(id) else { return };
            f(p, &grid, &shown);
        }
        self.save();
        self.layout();
        self.refresh_card(id);
        self.notify(Change::Updated(id.to_string()));
    }

    fn set_size(self: &Rc<Self>, id: &str, size: Size) {
        self.update(id, |p, grid, shown| {
            let at = shown
                .iter()
                .find(|s| s.id == p.id)
                .map_or((p.col, p.row), |s| (s.col, s.row));
            let spot = if grid.is_free(shown, Some(&p.id), at.0, at.1, size) {
                Some(at)
            } else {
                grid.free_spot(shown, Some(&p.id), size, at)
            };
            if let Some((c, r)) = spot {
                p.size = size;
                (p.col, p.row) = (c, r);
            }
        });
    }

    pub fn set_place(self: &Rc<Self>, id: &str, place: Place) {
        self.update(id, |p, _, _| p.options.place = Some(place));
    }

    fn set_options(self: &Rc<Self>, id: &str, f: impl FnOnce(&mut Options)) {
        self.update(id, |p, _, _| f(&mut p.options));
    }

    fn grant(self: &Rc<Self>, id: &str) {
        let host = match self.content(id) {
            Some(Content::Consent { host, .. }) => host,
            _ => return,
        };
        {
            let mut st = self.st.borrow_mut();
            let Some(Kind::Custom(pkg)) = st.desk.get(id).map(|p| p.kind.clone()) else {
                return;
            };
            st.desk.grant(&pkg, &host);
        }
        self.save();
        self.notify(Change::Granted(id.to_string()));
    }

    /// Edit Widgets on or off.
    pub fn set_edit(self: &Rc<Self>, on: bool) {
        if self.st.borrow().edit == on {
            return;
        }
        self.st.borrow_mut().edit = on;
        if !on {
            let rs: Vec<gtk::Button> = self
                .st
                .borrow_mut()
                .removes
                .drain()
                .map(|(_, r)| r)
                .collect();
            for r in rs {
                self.canvas.remove(&r);
            }
        }
        let ids: Vec<String> = self.widgets().into_iter().map(|p| p.id).collect();
        for id in ids {
            self.refresh_card(&id);
        }
        self.gallery.root.set_visible(on);
        if let Some(win) = self.win.borrow().as_ref() {
            if on {
                surface::set_layer(win, surface::Layer::Top, surface::Keyboard::OnDemand);
            } else {
                surface::set_layer(win, surface::Layer::Bottom, surface::Keyboard::None);
            }
        }
        self.notify(Change::Edit(on));
    }

    fn choose_city(self: &Rc<Self>, id: &str) {
        self.set_edit(true);
        self.gallery
            .select(&Kind::Builtin(Builtin::Weather), Some(id.to_string()));
    }

    /// Socket: `show|hide|toggle widgets` (Edit Widgets).
    pub fn command(self: &Rc<Self>, c: &crate::protocol::Command) -> bool {
        if c.target != "widgets" {
            return false;
        }
        let on = crate::protocol::apply(c.verb, self.is_editing());
        self.set_edit(on);
        true
    }

    /// widgets.json or the package folders changed (`wol shell reload`).
    pub fn replace(self: &Rc<Self>, desk: Desktop, packages: Vec<Package>) {
        let packages: Vec<Rc<Package>> = packages.into_iter().map(Rc::new).collect();
        let ids: Vec<String> = self.st.borrow().cards.keys().cloned().collect();
        for id in ids {
            self.drop_card(&id);
        }
        {
            let mut st = self.st.borrow_mut();
            st.desk = desk;
            st.packages = packages.clone();
            st.content.clear();
        }
        self.gallery.set_entries(entries(&packages));
        self.layout();
    }

    fn card_menu(self: &Rc<Self>, id: &str, x: f64, y: f64) {
        let Some(p) = self.st.borrow().desk.get(id).cloned() else {
            return;
        };
        let packages = self.packages();
        let mut rows: Vec<w::MenuRow> = Vec::new();
        let me = Rc::downgrade(self);
        let act = |f: BoardFn| -> Box<dyn Fn()> {
            let me = me.clone();
            Box::new(move || {
                if let Some(b) = me.upgrade() {
                    f(&b);
                }
            })
        };
        let tick = |on: bool| on.then_some("✓");
        for s in sizes_of(&p.kind, &packages, p.size) {
            let id = p.id.clone();
            rows.push((
                s.label().to_string(),
                tick(s == p.size),
                act(Box::new(move |b| b.set_size(&id, s))),
            ));
        }
        match &p.kind {
            Kind::Builtin(Builtin::Clock) => {
                rows.push((String::new(), None, Box::new(|| {})));
                let face = p.options.face.unwrap_or(Face::Analog);
                for (label, f) in [("Analog", Face::Analog), ("Digital", Face::Digital)] {
                    let id = p.id.clone();
                    rows.push((
                        label.into(),
                        tick(face == f),
                        act(Box::new(move |b| b.set_options(&id, |o| o.face = Some(f)))),
                    ));
                }
            }
            Kind::Builtin(Builtin::Weather) => {
                rows.push((String::new(), None, Box::new(|| {})));
                let id = p.id.clone();
                rows.push((
                    "Choose City…".into(),
                    None,
                    act(Box::new(move |b| b.choose_city(&id))),
                ));
                let units = p.options.units.unwrap_or_else(local_units);
                for (label, u) in [
                    ("Celsius", Units::Celsius),
                    ("Fahrenheit", Units::Fahrenheit),
                ] {
                    let id = p.id.clone();
                    rows.push((
                        label.into(),
                        tick(units == u),
                        act(Box::new(move |b| b.set_options(&id, |o| o.units = Some(u)))),
                    ));
                }
            }
            _ => {}
        }
        rows.push((String::new(), None, Box::new(|| {})));
        if !self.is_editing() {
            rows.push((
                "Edit Widgets…".into(),
                None,
                act(Box::new(|b| b.set_edit(true))),
            ));
        }
        let id = p.id.clone();
        rows.push((
            "Remove Widget".into(),
            None,
            act(Box::new(move |b| b.remove(&id))),
        ));
        let card = self.st.borrow().cards.get(&p.id).cloned();
        if let Some(c) = card {
            d::popup_menu(&c, x, y, rows);
        }
    }

    // ---- Drag and drop (Edit Widgets) ----

    fn card_at(&self, x: f64, y: f64) -> Option<String> {
        let mut hit = self.canvas.pick(x, y, gtk::PickFlags::DEFAULT);
        let st = self.st.borrow();
        while let Some(wg) = hit {
            if let Some((id, _)) = st
                .cards
                .iter()
                .find(|(_, c)| c.upcast_ref::<gtk::Widget>() == &wg)
            {
                return Some(id.clone());
            }
            if st
                .removes
                .values()
                .any(|r| r.upcast_ref::<gtk::Widget>() == &wg)
            {
                return None;
            }
            hit = wg.parent();
        }
        None
    }

    fn drag_begin(self: &Rc<Self>, x: f64, y: f64) -> bool {
        if !self.is_editing() {
            return false;
        }
        let Some(id) = self.card_at(x, y) else {
            return false;
        };
        let Some(p) = self.shown().into_iter().find(|p| p.id == id) else {
            return false;
        };
        let (x0, y0) = self.origin(&p);
        let mark = d::drop_mark(p.size);
        self.canvas.put(&mark, x0, y0);
        let mut st = self.st.borrow_mut();
        if let Some(c) = st.cards.get(&id) {
            mark.insert_before(&self.canvas, Some(c));
            c.add_css_class("dragging");
            c.insert_before(&self.canvas, None::<&gtk::Widget>);
        }
        if let Some(r) = st.removes.get(&id) {
            r.set_visible(false);
        }
        st.mark = Some(mark);
        st.drag = Some(Drag {
            id,
            x0,
            y0,
            target: (p.col, p.row),
        });
        true
    }

    fn drag_update(&self, dx: f64, dy: f64) {
        let shown = self.shown();
        let mut st = self.st.borrow_mut();
        let grid = st.grid;
        let (cw, ch) = (self.canvas.width() as f64, self.canvas.height() as f64);
        let Some(dr) = st.drag.as_ref() else { return };
        let id = dr.id.clone();
        let Some(p) = shown.iter().find(|p| p.id == id) else {
            return;
        };
        let (pw, ph) = grid.m.size(p.size);
        let nx = (dr.x0 + dx).clamp(0.0, (cw - pw as f64).max(0.0));
        let ny = (dr.y0 + dy).clamp(0.0, (ch - ph as f64).max(0.0));
        if let Some(c) = st.cards.get(&id) {
            self.canvas.move_(c, nx, ny);
        }
        let (c, r) = grid.snap(nx, ny, p.size);
        let target = if grid.is_free(&shown, Some(&id), c, r, p.size) {
            (c, r)
        } else {
            grid.drop_at(&shown, &id, c, r)
        };
        let (mx, my) = grid.m.origin(target.0, target.1);
        if let Some(m) = st.mark.as_ref() {
            self.canvas.move_(m, mx as f64, my as f64);
        }
        if let Some(dr) = st.drag.as_mut() {
            dr.target = target;
        }
    }

    fn drag_end(self: &Rc<Self>) {
        let (dr, mark) = {
            let mut st = self.st.borrow_mut();
            (st.drag.take(), st.mark.take())
        };
        if let Some(m) = mark {
            self.canvas.remove(&m);
        }
        let Some(dr) = dr else { return };
        {
            let mut st = self.st.borrow_mut();
            if let Some(c) = st.cards.get(&dr.id) {
                c.remove_css_class("dragging");
            }
            if let Some(r) = st.removes.get(&dr.id) {
                r.set_visible(true);
            }
            if let Some(p) = st.desk.get_mut(&dr.id) {
                (p.col, p.row) = dr.target;
            }
        }
        self.save();
        self.layout();
    }
}

fn local_units() -> Units {
    let l = ["LC_ALL", "LC_MEASUREMENT", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    weather::units_for_locale(&l)
}

/// Geocoding off the main loop; `done` runs on it.
/// Gallery margins (start, end): the gallery sits on the wider side clear of every card span.
fn gallery_margins(spans: impl Iterator<Item = (i32, i32)>, total: i32, pad: i32) -> (i32, i32) {
    let (lo, hi) = spans.fold((i32::MAX, i32::MIN), |(lo, hi), (a, b)| {
        (lo.min(a), hi.max(b))
    });
    if lo > hi {
        return (0, 0);
    }
    if total - hi >= lo {
        (hi + pad, 0)
    } else {
        (0, total - lo + pad)
    }
}

fn search_places(q: String, done: super::widgets_gallery::Found) {
    glib::spawn_future_local(async move {
        let r = gio::spawn_blocking(move || {
            wol_core::widgets::http_get(
                &weather::geocode_url(&q),
                wol_core::widgets::HTTP_TIMEOUT,
                wol_core::widgets::MAX_OUTPUT,
            )
            .map(|b| weather::parse_places(&String::from_utf8_lossy(&b)))
            .map_err(|e| e.message())
        })
        .await
        .unwrap_or_else(|_| Err(String::new()))
        .map_err(|why| {
            eprintln!("wol-shell: city search: {why}");
            crate::copy::CITY_SEARCH.to_string()
        });
        done(r);
    });
}

// ---- Live data ----

/// Local date and time (`WOL_FAKE_NOW`: the golden clock).
pub fn local_now() -> Now {
    if std::env::var_os("WOL_FAKE_NOW").is_some() {
        return builtin::sample_now();
    }
    let Ok(t) = glib::DateTime::now_local() else {
        return builtin::sample_now();
    };
    Now {
        y: t.year(),
        month: t.month() as u32,
        day: t.day_of_month() as u32,
        weekday: (t.day_of_week() as u32).saturating_sub(1),
        hour: t.hour() as u32,
        minute: t.minute() as u32,
    }
}

/// An event time as local wall-clock time.
fn to_local(wh: &calendar::When) -> Option<calendar::DateTime> {
    use calendar::When;
    let t = match wh {
        When::Date { .. } => return None,
        When::Floating(at) => return Some(*at),
        When::Utc(a) => {
            glib::DateTime::from_utc(a.y, a.m as i32, a.d as i32, a.hh as i32, a.mm as i32, 0.0)
                .ok()?
        }
        When::Zoned { tzid, at: a } => {
            let tz = glib::TimeZone::new(Some(tzid));
            glib::DateTime::new(
                &tz,
                a.y,
                a.m as i32,
                a.d as i32,
                a.hh as i32,
                a.mm as i32,
                0.0,
            )
            .ok()?
        }
    }
    .to_local()
    .ok()?;
    Some(calendar::DateTime {
        y: t.year(),
        m: t.month() as u32,
        d: t.day_of_month() as u32,
        hh: t.hour() as u32,
        mm: t.minute() as u32,
    })
}

/// Today in UTC `YYYYMMDDTHHMMSSZ` (EDS range).
fn today_utc(n: &Now) -> Option<(String, String)> {
    let start = glib::DateTime::new(
        &glib::TimeZone::local(),
        n.y,
        n.month as i32,
        n.day as i32,
        0,
        0,
        0.0,
    )
    .ok()?;
    let end = start.add_days(1).ok()?;
    let f = |t: &glib::DateTime| {
        t.to_utc()
            .ok()
            .and_then(|u| u.format("%Y%m%dT%H%M%SZ").ok())
            .map(|s| s.to_string())
    };
    Some((f(&start)?, f(&end)?))
}

/// Seconds between refreshes.
const MINUTE: u64 = 60;
const SYSTEM_EVERY: u64 = 10;
const EVENTS_EVERY: u64 = 15 * 60;
const BATTERY_EVERY: u64 = 2 * 60;
/// CPU samples a System chart shows (5 minutes).
const CPU_SAMPLES: usize = 30;

struct Live {
    board: Rc<Board>,
    sys: Rc<dyn crate::system::System>,
    schedule: RefCell<Schedule>,
    scheduled: RefCell<HashSet<String>>,
    timer: RefCell<Option<glib::SourceId>>,
    paused: Cell<bool>,
    wakeups: Cell<u64>,
    started: std::time::Instant,
    stats: RefCell<Option<std::sync::Arc<std::sync::Mutex<wol_core::system::stats::StatsSource>>>>,
    cpu: RefCell<wd::History>,
    system: RefCell<Option<SystemView>>,
    /// None = no calendar service.
    events: RefCell<Option<Vec<calendar::RawEvent>>>,
    events_day: Cell<u32>,
    music: RefCell<Option<media::Playing>>,
    music_on: Cell<bool>,
    /// None = UPower not asked yet.
    upower: Cell<Option<bool>>,
    batteries: RefCell<Vec<power::Device>>,
    weather: RefCell<HashMap<String, WeatherView>>,
    custom: RefCell<HashMap<String, Value>>,
}

fn timing() -> bool {
    std::env::var_os("WOL_SHELL_TIMING").is_some()
}

fn place_key(p: &Place, u: Units) -> String {
    format!("{:.3},{:.3},{u:?}", p.lat, p.lon)
}

impl Live {
    fn units(p: &Placed) -> Units {
        p.options.units.unwrap_or_else(local_units)
    }

    /// Content for one placed widget from what is known now.
    fn content(&self, p: &Placed, now: &Now) -> Option<Content> {
        Some(match &p.kind {
            Kind::Builtin(b) => Content::Builtin(match b {
                Builtin::Clock => Data::Clock {
                    now: *now,
                    face: p.options.face.unwrap_or(Face::Analog),
                },
                Builtin::Calendar => Data::Calendar {
                    now: *now,
                    events: self
                        .events
                        .borrow()
                        .as_ref()
                        .map(|e| calendar::upcoming(e, now.at(), to_local)),
                },
                Builtin::Weather => Data::Weather(match &p.options.place {
                    None => WeatherView::NoPlace,
                    Some(pl) => self
                        .weather
                        .borrow()
                        .get(&place_key(pl, Self::units(p)))
                        .cloned()
                        .unwrap_or_else(|| WeatherView::Loading(pl.clone())),
                }),
                Builtin::Music => Data::Music(self.music.borrow().clone()),
                Builtin::Battery => Data::Batteries {
                    devices: self.batteries.borrow().clone(),
                    minutes: self.sys.battery_time(),
                },
                Builtin::System => Data::System(self.system.borrow().clone()?),
            }),
            Kind::Custom(id) => {
                let pkg = self.board.packages().into_iter().find(|k| &k.id == id);
                let Some(pkg) = pkg else {
                    return Some(Content::Error {
                        name: id.clone(),
                        message: crate::copy::WIDGET_MISSING.into(),
                        details: vec![format!(
                            "Package folder: ~/.local/share/{}/{id}",
                            shared::widgets::USER_PACKAGES_DIR
                        )],
                    });
                };
                let m = match &pkg.manifest {
                    Err(problems) => {
                        return Some(Content::Error {
                            name: pkg.name().into(),
                            message: crate::copy::widget_problems(pkg.name()),
                            details: lay::problem_lines(problems),
                        });
                    }
                    Ok(m) => m,
                };
                if let Some(host) = m.source.as_ref().and_then(Source::host)
                    && !self.board.desktop().allows(id, &host)
                {
                    return Some(Content::Consent {
                        name: m.name.clone(),
                        host,
                    });
                }
                let data = match (&m.source, self.custom.borrow().get(&p.id)) {
                    (None, _) => Value::Null,
                    (Some(_), Some(v)) => v.clone(),
                    (Some(_), None) => return None,
                };
                Content::Custom {
                    manifest: Rc::new(m.clone()),
                    dir: pkg.dir.clone(),
                    data,
                }
            }
            Kind::Unknown(k) => Content::Error {
                name: k.clone(),
                message: crate::copy::WIDGET_NEWER.into(),
                details: vec![format!("Widget kind: {k}")],
            },
        })
    }

    /// Refreshes the cards of widgets matching `which`.
    fn show(&self, which: impl Fn(&Placed) -> bool) {
        let now = local_now();
        for p in self.board.widgets().iter().filter(|p| which(p)) {
            if let Some(c) = self.content(p, &now) {
                self.board.set_content(&p.id, c);
            }
        }
    }

    fn show_kind(&self, b: Builtin) {
        self.show(|p| p.kind == Kind::Builtin(b));
    }

    fn has(&self, b: Builtin) -> bool {
        self.board
            .widgets()
            .iter()
            .any(|p| p.kind == Kind::Builtin(b))
    }

    fn schedule(&self, id: &str, period: u64) {
        if self.scheduled.borrow_mut().insert(id.to_string()) {
            self.schedule.borrow_mut().set(id, period, wd::unix_now());
        }
    }

    fn unschedule(&self, id: &str) {
        if self.scheduled.borrow_mut().remove(id) {
            self.schedule.borrow_mut().remove(id);
        }
    }

    /// Starts what the widgets on the desktop need and stops what nothing needs any more.
    fn sync(self: &Rc<Self>) {
        let ws = self.board.widgets();
        let has = |b: Builtin| ws.iter().any(|p| p.kind == Kind::Builtin(b));
        if has(Builtin::Clock) || has(Builtin::Calendar) {
            self.schedule("minute", MINUTE);
        } else {
            self.unschedule("minute");
        }
        if has(Builtin::Calendar) {
            if !self.scheduled.borrow().contains("events") {
                self.schedule("events", EVENTS_EVERY);
                self.read_events();
            }
        } else {
            self.unschedule("events");
        }
        if has(Builtin::System) {
            if !self.scheduled.borrow().contains("system") {
                self.schedule("system", SYSTEM_EVERY);
                self.read_system();
            }
        } else {
            self.unschedule("system");
            *self.stats.borrow_mut() = None;
        }
        if has(Builtin::Weather) {
            self.schedule("weather", weather::REFRESH_SECS);
        } else {
            self.unschedule("weather");
        }
        if has(Builtin::Music) && !self.music_on.replace(true) {
            self.watch_music();
        }
        if has(Builtin::Battery) {
            match self.upower.get() {
                None => self.watch_batteries(),
                Some(false) => self.schedule("battery", BATTERY_EVERY),
                Some(true) => {}
            }
        } else {
            self.unschedule("battery");
        }
        let custom: HashSet<String> = ws
            .iter()
            .filter(|p| matches!(p.kind, Kind::Custom(_)))
            .map(|p| format!("custom:{}", p.id))
            .collect();
        let gone: Vec<String> = self
            .scheduled
            .borrow()
            .iter()
            .filter(|k| k.starts_with("custom:") && !custom.contains(*k))
            .cloned()
            .collect();
        for k in gone {
            self.unschedule(&k);
        }
        self.arm();
    }

    fn arm(self: &Rc<Self>) {
        if let Some(t) = self.timer.borrow_mut().take() {
            t.remove();
        }
        if self.paused.get() {
            return;
        }
        let now_ms = glib::real_time() / 1000;
        let now = (now_ms / 1000) as u64;
        let Some(wait) = self.schedule.borrow().wait(now) else {
            return;
        };
        // Just past the deadline: a clock never shows the minute that just ended.
        let due_ms = (now + wait) as i64 * 1000 + 50;
        let delay = (due_ms - now_ms).max(0) as u64;
        let me = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
            if let Some(l) = me.upgrade() {
                *l.timer.borrow_mut() = None;
                l.wake();
            }
        });
        *self.timer.borrow_mut() = Some(id);
    }

    fn wake(self: &Rc<Self>) {
        let due = self.schedule.borrow_mut().take_due(wd::unix_now());
        if !due.is_empty() {
            let n = self.wakeups.get() + 1;
            self.wakeups.set(n);
            if timing() {
                let mins = self.started.elapsed().as_secs_f64() / 60.0;
                eprintln!(
                    "wol-shell: widgets wake {due:?} ({n} in {mins:.1} min, {:.1}/h)",
                    n as f64 / mins.max(1.0 / 60.0) * 60.0
                );
            }
        }
        for id in due {
            self.run(&id);
        }
        self.arm();
    }

    fn run(self: &Rc<Self>, id: &str) {
        match id {
            "minute" => {
                let now = local_now();
                if self.events_day.get() != now.day && self.has(Builtin::Calendar) {
                    self.read_events();
                }
                self.show(|p| matches!(p.kind, Kind::Builtin(Builtin::Clock | Builtin::Calendar)));
            }
            "system" => self.read_system(),
            "events" => self.read_events(),
            "weather" => self.refresh_weather(None, false),
            "battery" => {
                *self.batteries.borrow_mut() = power::builtin(&*self.sys);
                self.show_kind(Builtin::Battery);
            }
            c if c.starts_with("custom:") => self.run_custom(&c["custom:".len()..]),
            _ => {}
        }
    }

    fn set_paused(self: &Rc<Self>, on: bool) {
        if self.paused.replace(on) == on {
            return;
        }
        if timing() {
            eprintln!(
                "wol-shell: widgets {}",
                if on { "paused (fullscreen)" } else { "resumed" }
            );
        }
        if on {
            if let Some(t) = self.timer.borrow_mut().take() {
                t.remove();
            }
        } else {
            // What came due while covered runs now.
            self.wake();
        }
    }

    fn watch_fullscreen(self: &Rc<Self>) {
        let Some(path) = crate::hypr::socket(".socket2.sock") else {
            return;
        };
        let check = {
            let me = Rc::downgrade(self);
            move || {
                if let Some(l) = me.upgrade() {
                    let full = crate::hypr::request("j/activeworkspace")
                        .is_some_and(|j| wd::has_fullscreen(&j));
                    l.set_paused(full);
                }
            }
        };
        check();
        glib::spawn_future_local(async move {
            let Ok(conn) = gio::SocketClient::new()
                .connect_future(&gio::UnixSocketAddress::new(&path))
                .await
            else {
                return;
            };
            let input = gio::DataInputStream::new(&conn.input_stream());
            while let Ok(Some(line)) = input
                .read_line_utf8_future(glib::Priority::DEFAULT_IDLE)
                .await
            {
                if wd::fullscreen_may_change(&line) {
                    check();
                }
            }
        });
    }

    fn read_system(self: &Rc<Self>) {
        let src = self
            .stats
            .borrow_mut()
            .get_or_insert_with(|| {
                std::sync::Arc::new(std::sync::Mutex::new(
                    wol_core::system::stats::StatsSource::new(std::sync::Arc::new(
                        wol_core::system::PosixHost,
                    )),
                ))
            })
            .clone();
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let s = if cfg!(target_os = "freebsd") {
                // swapinfo(8) runs at most every 30 s: never on the main loop.
                match gio::spawn_blocking(move || src.lock().map(|mut s| s.read()).ok()).await {
                    Ok(Some(s)) => s,
                    _ => return,
                }
            } else {
                match src.lock() {
                    Ok(mut s) => s.read(),
                    Err(_) => return,
                }
            };
            let Some(l) = me.upgrade() else { return };
            let mut cpu = l.cpu.borrow_mut();
            cpu.push(s.cpu_percent);
            *l.system.borrow_mut() = Some(SystemView {
                cpu: cpu.values(),
                mem_used: s.memory.used,
                mem_total: s.memory.total,
                load: s.load.map(|x| (x.one, x.five, x.fifteen)),
                uptime: s.uptime_secs,
            });
            drop(cpu);
            l.show_kind(Builtin::System);
        });
    }

    fn read_events(self: &Rc<Self>) {
        let now = local_now();
        self.events_day.set(now.day);
        let Some((start, end)) = today_utc(&now) else {
            return;
        };
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let ev = gio::spawn_blocking(move || crate::widgets::eds::read(&start, &end))
                .await
                .ok()
                .flatten();
            if let Some(l) = me.upgrade() {
                *l.events.borrow_mut() = ev;
                l.show_kind(Builtin::Calendar);
            }
        });
    }

    fn watch_music(self: &Rc<Self>) {
        let (tx, rx) = async_channel::unbounded();
        media::watch(tx);
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(p) = rx.recv().await {
                let Some(l) = me.upgrade() else { break };
                *l.music.borrow_mut() = p;
                l.show_kind(Builtin::Music);
            }
        });
    }

    fn watch_batteries(self: &Rc<Self>) {
        self.upower.set(Some(true));
        let (tx, rx) = async_channel::unbounded();
        power::watch(tx);
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(v) = rx.recv().await {
                let Some(l) = me.upgrade() else { break };
                match v {
                    Some(devs) => *l.batteries.borrow_mut() = devs,
                    None => {
                        // No UPower: the built-in battery, on the widget schedule.
                        l.upower.set(Some(false));
                        *l.batteries.borrow_mut() = power::builtin(&*l.sys);
                        l.sync();
                    }
                }
                l.show_kind(Builtin::Battery);
            }
        });
    }

    /// Weather for one widget's place (`only`), or every place on the desktop.
    fn refresh_weather(self: &Rc<Self>, only: Option<&str>, force: bool) {
        let mut places: Vec<(Place, Units)> = Vec::new();
        for p in self.board.widgets() {
            if p.kind != Kind::Builtin(Builtin::Weather) || only.is_some_and(|o| o != p.id) {
                continue;
            }
            if let Some(pl) = &p.options.place {
                let u = Self::units(&p);
                if !places
                    .iter()
                    .any(|(q, v)| place_key(q, *v) == place_key(pl, u))
                {
                    places.push((pl.clone(), u));
                }
            }
        }
        for (pl, u) in places {
            let me = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                let cache = wd::cache_home();
                let p2 = pl.clone();
                let out = gio::spawn_blocking(move || {
                    weather::load(&cache, &p2, u, wd::unix_now(), force, |url| {
                        wol_core::widgets::http_get(
                            url,
                            wol_core::widgets::HTTP_TIMEOUT,
                            wol_core::widgets::MAX_OUTPUT,
                        )
                        .map_err(|e| e.message())
                    })
                })
                .await
                .unwrap_or(weather::Outcome::Offline);
                let Some(l) = me.upgrade() else { return };
                let view = match out {
                    weather::Outcome::Fresh(f) => WeatherView::Ready {
                        place: pl.clone(),
                        forecast: f,
                        stale: None,
                    },
                    weather::Outcome::Stale(f, age) => WeatherView::Ready {
                        place: pl.clone(),
                        forecast: f,
                        stale: Some(age),
                    },
                    weather::Outcome::Offline => WeatherView::Offline(pl.clone()),
                };
                l.weather.borrow_mut().insert(place_key(&pl, u), view);
                l.show_kind(Builtin::Weather);
            });
        }
    }

    fn run_custom(self: &Rc<Self>, wid: &str) {
        let Some(p) = self.board.widgets().into_iter().find(|p| p.id == wid) else {
            return;
        };
        let Kind::Custom(pkg_id) = &p.kind else {
            return;
        };
        let Some(pkg) = self.board.packages().into_iter().find(|k| &k.id == pkg_id) else {
            return self.show(|q| q.id == wid);
        };
        let Ok(m) = &pkg.manifest else {
            return self.show(|q| q.id == wid);
        };
        let Some(src) = m.source.clone() else {
            return self.show(|q| q.id == wid);
        };
        let desk = self.board.desktop();
        let allowed: Vec<String> = desk
            .grants
            .iter()
            .filter(|g| &g.widget == pkg_id)
            .map(|g| g.host.clone())
            .collect();
        if let Some(h) = src.host()
            && !allowed.contains(&h)
        {
            return self.show(|q| q.id == wid);
        }
        let (dir, name, wid) = (pkg.dir.clone(), m.name.clone(), wid.to_string());
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let home = wol_core::widgets::home();
            let r = gio::spawn_blocking(move || {
                wol_core::widgets::read_source(&src, &dir, &home, |h| {
                    allowed.iter().any(|a| a == h)
                })
            })
            .await;
            let Some(l) = me.upgrade() else { return };
            match r {
                Ok(Ok(v)) => {
                    l.custom.borrow_mut().insert(wid.clone(), v);
                    l.show(|q| q.id == wid);
                }
                Ok(Err(e)) => {
                    // Keep showing the last good data; without any, say what went wrong.
                    if !l.custom.borrow().contains_key(&wid) {
                        l.board.set_content(
                            &wid,
                            Content::Error {
                                message: crate::copy::widget_source(&e, &name),
                                name,
                                details: vec![e.message()],
                            },
                        );
                    }
                    eprintln!("wol-shell: widget {wid}: {}", e.message());
                }
                Err(_) => {}
            }
        });
    }

    fn on_change(self: &Rc<Self>, c: Change) {
        match c {
            Change::Added(id) | Change::Updated(id) | Change::Granted(id) => {
                self.sync();
                let Some(p) = self.board.widgets().into_iter().find(|p| p.id == id) else {
                    return;
                };
                self.show(|q| q.id == id);
                match &p.kind {
                    Kind::Builtin(Builtin::Weather) => self.refresh_weather(Some(&id), false),
                    Kind::Custom(_) => self.start_custom(&p),
                    _ => {}
                }
            }
            Change::Removed(p) => {
                self.custom.borrow_mut().remove(&p.id);
                self.unschedule(&format!("custom:{}", p.id));
                self.sync();
            }
            Change::Edit(_) => {}
        }
    }

    fn start_custom(self: &Rc<Self>, p: &Placed) {
        let Kind::Custom(pkg_id) = &p.kind else {
            return;
        };
        let refresh = self
            .board
            .packages()
            .into_iter()
            .find(|k| &k.id == pkg_id)
            .and_then(|k| k.manifest.as_ref().ok().map(|m| m.refresh))
            .unwrap_or(0);
        if refresh > 0 {
            self.schedule(&format!("custom:{}", p.id), refresh as u64);
        }
        self.run_custom(&p.id);
        self.arm();
    }

    /// Everything from scratch (start, `wol shell reload`).
    fn start_all(self: &Rc<Self>) {
        self.custom.borrow_mut().clear();
        self.sync();
        self.show(|_| true);
        self.refresh_weather(None, false);
        for p in self.board.widgets() {
            if matches!(p.kind, Kind::Custom(_)) {
                self.start_custom(&p);
            }
        }
    }
}

fn read_disk() -> (Desktop, PathBuf, Vec<Package>) {
    let file = wd::desktop_path(&wol_core::theme::config_home());
    let (desk, err) = wd::load(&file);
    if let Some(e) = err {
        eprintln!("wol-shell: {e} (the file is kept as widgets.json.bad on the next change)");
    }
    let packages = wol_core::widgets::load_packages(&wol_core::widgets::package_dirs(
        &wol_core::widgets::data_home(),
    ));
    (desk, file, packages)
}

/// The desktop widget surface with its live data.
pub fn build(
    app: &gtk::Application,
    sys: Rc<dyn crate::system::System>,
) -> (gtk::ApplicationWindow, Rc<Board>) {
    let (desk, file, packages) = read_disk();
    let board = Board::new(desk, Some(file), packages);
    let win = super::window(app, "desktop", surface::Spec::desktop());
    win.set_child(Some(&board.root));
    board.set_window(win.upcast_ref());
    let keys = gtk::EventControllerKey::new();
    let b2 = Rc::downgrade(&board);
    keys.connect_key_pressed(move |_, k, _, _| match b2.upgrade() {
        Some(b) if k == gtk::gdk::Key::Escape && b.is_editing() => {
            b.set_edit(false);
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    win.add_controller(keys);
    let live = Rc::new(Live {
        board: board.clone(),
        sys,
        schedule: Default::default(),
        scheduled: Default::default(),
        timer: Default::default(),
        paused: Cell::new(false),
        wakeups: Cell::new(0),
        started: std::time::Instant::now(),
        stats: Default::default(),
        cpu: RefCell::new(wd::History::new(CPU_SAMPLES)),
        system: Default::default(),
        events: Default::default(),
        events_day: Cell::new(0),
        music: Default::default(),
        music_on: Cell::new(false),
        upower: Cell::new(None),
        batteries: Default::default(),
        weather: Default::default(),
        custom: Default::default(),
    });
    let l2 = Rc::downgrade(&live);
    board.set_listener(Rc::new(move |c| {
        if let Some(l) = l2.upgrade() {
            l.on_change(c);
        }
    }));
    live.start_all();
    live.watch_fullscreen();
    // `wol shell reload`: widgets.json and the package folders again.
    let l3 = Rc::downgrade(&live);
    let first = Cell::new(true);
    super::ipc::on_reload(move |_| {
        if first.replace(false) {
            return;
        }
        if let Some(l) = l3.upgrade() {
            let (desk, _, packages) = read_disk();
            l.board.replace(desk, packages);
            l.start_all();
        }
    });
    // The providers live as long as the surface.
    win.connect_destroy(move |_| {
        let _ = &live;
    });
    (win, board)
}

#[cfg(test)]
mod gallery_tests {
    use super::gallery_margins;

    #[test]
    fn the_gallery_never_covers_a_card() {
        assert_eq!(gallery_margins([(24, 368)].into_iter(), 1920, 24), (392, 0));
        assert_eq!(
            gallery_margins([(1500, 1896)].into_iter(), 1920, 24),
            (0, 444)
        );
        assert_eq!(gallery_margins(std::iter::empty(), 1920, 24), (0, 0));
    }
}
