//! App Store (`wol-store`): macOS App Store layout on the kit AppWindow. Curated catalog from
//! `wol_core::store`, `pkg search` for the rest, installs through wol-admind `pkg-install` with the
//! Authenticate sheet, updates from `wol update check`. `wol-store <file.wol>` (double-click) opens
//! the install page: wol-admind `wol-install` (ADR 0011). Slow work never runs on the UI thread.
use super::settings::{Ctx, UpdateState, parse_check};
use crate::kit::{self, widgets as w};
use crate::system::{Action, ActionError, System};
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use wol_core::package;
use wol_core::store::{self, App, Category};

pub const PAGES: [&str; 6] = ["Discover", "Create", "Work", "Play", "Develop", "Updates"];

/// Machine facts the store reads (injected so goldens do not depend on the build host).
#[derive(Clone, Debug)]
pub struct Env {
    /// XDG data dirs searched for `applications/<id>.desktop`.
    pub data_dirs: Vec<PathBuf>,
    /// Run `pkg search` for non-catalog results.
    pub pkg_search: bool,
}

impl Env {
    pub fn read() -> Self {
        let mut dirs: Vec<PathBuf> = vec![PathBuf::from(
            std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
                format!("{}/.local/share", std::env::var("HOME").unwrap_or_default())
            }),
        )];
        dirs.extend(
            std::env::var("XDG_DATA_DIRS")
                .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
                .split(':')
                .map(PathBuf::from),
        );
        Env {
            data_dirs: dirs,
            pkg_search: true,
        }
    }
}

pub fn run() -> glib::ExitCode {
    let arg = std::env::args().nth(1);
    // A double-clicked `.wol` opens its own window, even when the App Store is already open.
    let file = arg.as_ref().is_some_and(|a| a.ends_with(".wol"));
    let flags = if file {
        gtk::gio::ApplicationFlags::NON_UNIQUE
    } else {
        gtk::gio::ApplicationFlags::empty()
    };
    let app = gtk::Application::builder()
        .application_id("org.wolos.store")
        .flags(flags)
        .build();
    app.connect_activate(move |app| {
        kit::style::install("");
        let win = gtk::ApplicationWindow::builder()
            .application(app)
            .title("App Store")
            .default_width(w::APP_SIZE.0)
            .default_height(w::APP_SIZE.1)
            .decorated(false)
            .build();
        let s = content(crate::system::default_system(), Env::read());
        match &arg {
            Some(p) if file => s.open_package(
                &std::fs::canonicalize(p).map_or_else(|_| p.clone(), |a| a.display().to_string()),
            ),
            Some(p) => s.aw.select(p),
            None => {}
        }
        win.set_child(Some(&s.aw.root));
        kit::snapshot_on_map(&win, "store");
        win.present();
    });
    app.run_with_args::<&str>(&[])
}

pub struct Store {
    pub aw: w::AppWindow,
    pub search: gtk::SearchEntry,
    show_results: Rc<dyn Fn(&str)>,
    /// The install page: `None` while the file is checked.
    show_package: Rc<dyn Fn(Option<PackageFile>)>,
}

impl Store {
    /// Shows results for `term` now (no typing delay): goldens and `wol-store search:<term>`.
    pub fn search_now(&self, term: &str) {
        self.search.set_text(term);
        (self.show_results)(term);
    }

    /// Install page for the `.wol` at absolute `path` (double-click): checked off the UI thread.
    pub fn open_package(&self, path: &str) {
        (self.show_package)(None);
        let (tx, rx) = async_channel::bounded(1);
        let p = path.to_string();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(PackageFile::read(&p));
        });
        let show = self.show_package.clone();
        glib::spawn_future_local(async move {
            if let Ok(f) = rx.recv().await {
                show(Some(f));
            }
        });
    }

    /// Install page for an already checked file (goldens).
    pub fn show_package(&self, f: PackageFile) {
        (self.show_package)(Some(f));
    }
}

/// A `.wol` file as the install page sees it (ADR 0011).
#[derive(Debug)]
pub struct PackageFile {
    pub path: String,
    /// [`package::verify`] with the trusted keys; the error text is never shown.
    pub verified: Result<package::Verified, String>,
    /// Version of the same app installed now.
    pub installed: Option<String>,
    /// `icon.png` from the file.
    pub icon: Option<Vec<u8>>,
}

impl PackageFile {
    /// Blocking: hashes the whole file.
    pub fn read(path: &str) -> Self {
        let keys = package::read_keys(std::path::Path::new(shared::package::KEYS_DIR));
        Self::read_with(
            path,
            &keys,
            std::path::Path::new(shared::package::RECORDS_DIR),
        )
    }

    pub fn read_with(path: &str, keys: &[[u8; 32]], records: &std::path::Path) -> Self {
        let p = std::path::Path::new(path);
        let verified = package::verify(p, keys);
        let (installed, icon) = match &verified {
            Ok(v) => (
                package::installed(records)
                    .into_iter()
                    .find(|i| i.id == v.info.id)
                    .map(|i| i.version),
                v.icon
                    .as_ref()
                    .and_then(|m| package::read_member(p, m, 4 << 20).ok()),
            ),
            Err(_) => (None, None),
        };
        PackageFile {
            path: path.into(),
            verified,
            installed,
            icon,
        }
    }
}

/// What the install page says about a [`PackageFile`] (no GTK: tested and copy-linted).
#[derive(Debug, PartialEq)]
pub struct PackageSheet {
    pub name: String,
    pub summary: String,
    /// "Version 3.1.0 · com.example.App".
    pub detail: String,
    /// Trust row: (icon, title, subtitle).
    pub trust: (&'static str, &'static str, &'static str),
    /// (icon, label) for each capability the app declares.
    pub capabilities: Vec<(&'static str, &'static str)>,
    /// wol-admind `wol-install` trust argument; `None`: the file can't be installed.
    pub install: Option<&'static str>,
    /// "Install", "Update", or "Open" when this version is installed already.
    pub button: &'static str,
}

/// Trust rows of the install page: signed by a trusted key, not signed, refused.
pub const TRUST_ROWS: [(&str, &str, &str); 3] = [
    (
        "security-high-symbolic",
        "Verified",
        "Signed by a developer WolOS trusts, and unchanged since.",
    ),
    (
        "dialog-warning-symbolic",
        "Not signed",
        "WolOS can't check who made this app. Install it only if you trust where it came from.",
    ),
    (
        "dialog-warning-symbolic",
        "Can't be installed",
        "This file is damaged, was changed after it was signed, or comes from a developer WolOS doesn't know.",
    ),
];

/// Capability rows: icon and label for every `shared::package::CAPABILITIES` id.
pub fn capability_row(id: &str) -> Option<(&'static str, &'static str)> {
    let icon = match id {
        "network" => "network-wireless-symbolic",
        "files" => "folder-symbolic",
        "camera" => "camera-photo-symbolic",
        "microphone" => "microphone-sensitivity-high-symbolic",
        "location" => "preferences-desktop-locale-symbolic",
        "screen-capture" => "wolos-capture-screen-symbolic",
        "notifications" => "preferences-system-notifications-symbolic",
        "background" => "emblem-system-symbolic",
        "devices" => "drive-harddisk-symbolic",
        _ => return None,
    };
    let label = shared::package::CAPABILITIES.iter().find(|c| c.0 == id)?.1;
    Some((icon, label))
}

pub fn package_sheet(f: &PackageFile) -> PackageSheet {
    let file = std::path::Path::new(&f.path);
    let Ok(v) = &f.verified else {
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned());
        return PackageSheet {
            name: stem.unwrap_or_default(),
            summary: String::new(),
            detail: String::new(),
            trust: TRUST_ROWS[2],
            capabilities: vec![],
            install: None,
            button: "Install",
        };
    };
    let i = &v.info;
    let (trust, install) = match v.trust {
        package::Trust::Signed(_) => (TRUST_ROWS[0], "signed"),
        package::Trust::Unsigned => (TRUST_ROWS[1], "unsigned"),
    };
    PackageSheet {
        name: if i.name.is_empty() {
            i.id.clone()
        } else {
            i.name.clone()
        },
        summary: i.summary.clone(),
        detail: format!("Version {} · {}", i.version, i.id),
        trust,
        capabilities: i
            .capabilities
            .iter()
            .filter_map(|c| capability_row(c))
            .collect(),
        install: Some(install),
        button: match &f.installed {
            Some(v) if *v == i.version => "Open",
            Some(_) => "Update",
            None => "Install",
        },
    }
}

struct Shared {
    ctx: Rc<Ctx>,
    env: Env,
    /// One inline error line per window (HIG §3).
    status: gtk::Label,
}

fn say(l: &gtk::Label, text: &str) {
    l.set_label(text);
    l.set_visible(!text.is_empty());
}

fn icon(a: &App, size: i32) -> gtk::Widget {
    w::store_icon(a.id, size)
}

/// Get -> install (Authenticate sheet when needed) -> Open; Open launches the .desktop file.
fn wire(
    sh: &Rc<Shared>,
    target: String,
    name: String,
    desktop: Option<&'static str>,
    set: Rc<dyn Fn(w::TileState)>,
    button: &gtk::Button,
) {
    let installed = Rc::new(std::cell::Cell::new(false));
    if let Some(a) = store::get(&target)
        && store::is_installed(a, &sh.env.data_dirs)
    {
        installed.set(true);
        set(w::TileState::Open);
    }
    let sh = sh.clone();
    button.connect_clicked(move |_| {
        if installed.get() {
            if let Some(d) = desktop {
                kit::spawn(&["gtk4-launch".into(), d.into()]);
            }
            return;
        }
        set(w::TileState::Installing);
        say(&sh.status, "");
        let (set, st, inst, name) = (
            set.clone(),
            sh.status.clone(),
            installed.clone(),
            name.clone(),
        );
        sh.ctx.act(
            Action::PkgInstall(target.clone()),
            Rc::new(move |r: Result<(), ActionError>| match r {
                Ok(()) => {
                    inst.set(true);
                    set(w::TileState::Open);
                }
                Err(e) => {
                    set(w::TileState::Get);
                    say(&st, &crate::copy::install(e, &name));
                }
            }),
        );
    });
}

fn tile(sh: &Rc<Shared>, a: &'static App) -> gtk::Widget {
    let t = w::AppTile::new(&icon(a, w::ICON_L), a.name, a.summary);
    let t2 = t.clone();
    wire(
        sh,
        a.id.into(),
        a.name.into(),
        Some(a.desktop),
        Rc::new(move |s| t2.set_state(s)),
        &t.action,
    );
    t.root.upcast()
}

fn pkg_tile(sh: &Rc<Shared>, h: &store::PkgHit) -> gtk::Widget {
    let t = w::AppTile::new(&w::store_icon(&h.name, w::ICON_L), &h.name, &h.summary);
    let t2 = t.clone();
    wire(
        sh,
        h.name.clone(),
        h.name.clone(),
        None,
        Rc::new(move |s| t2.set_state(s)),
        &t.action,
    );
    t.root.upcast()
}

fn grid(tiles: impl IntoIterator<Item = gtk::Widget>) -> gtk::FlowBox {
    let g = w::wrap_row(w::S4, 3);
    g.set_halign(gtk::Align::Fill);
    for t in tiles {
        g.append(&t);
    }
    g
}

fn shelf(title: &str, g: &gtk::FlowBox) -> gtk::Box {
    let b = w::vbox(w::S2);
    b.append(&w::label(title, "title"));
    b.append(g);
    b
}

/// Page banner for Discover (`None`) or a category, from `store::hero`.
fn hero(sh: &Rc<Shared>, page: Option<Category>) -> Option<gtk::Widget> {
    let h = store::hero(page);
    let a = store::get(h.app)?;
    let get = gtk::Button::with_label("Get");
    get.add_css_class("get-button");
    let card = w::hero_banner(
        h.colors,
        a.name,
        h.headline,
        h.subhead,
        &icon(a, w::HERO_ICON),
        &get,
    );
    let g2 = get.clone();
    wire(
        sh,
        a.id.into(),
        a.name.into(),
        Some(a.desktop),
        Rc::new(move |s| {
            g2.set_sensitive(s != w::TileState::Installing);
            g2.set_label(match s {
                w::TileState::Get => "Get",
                w::TileState::Installing => "Installing…",
                w::TileState::Open => "Open",
            });
        }),
        &get,
    );
    Some(card.upcast())
}

fn discover(sh: &Rc<Shared>) -> gtk::Box {
    let p = w::page("Discover");
    if let Some(h) = hero(sh, None) {
        p.append(&h);
    }
    for (title, ids) in store::ROWS {
        let g = grid(
            ids.iter()
                .filter_map(|id| store::get(id))
                .map(|a| tile(sh, a)),
        );
        p.append(&shelf(title, &g));
    }
    p
}

fn category(sh: &Rc<Shared>, c: Category) -> gtk::Box {
    let p = w::page(c.name());
    if let Some(h) = hero(sh, Some(c)) {
        p.append(&h);
    }
    p.append(&grid(store::in_category(c).map(|a| tile(sh, a))));
    p
}

fn updates(sh: &Rc<Shared>) -> gtk::Box {
    let p = w::page("Updates");
    let ic = w::icon("software-update-available-symbolic", w::ICON_M);
    ic.add_css_class("accent-icon");
    let title = w::label("", "title");
    title.set_wrap(true);
    let detail = w::label("", "secondary");
    detail.set_wrap(true);
    let text = w::vbox(0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&title);
    text.append(&detail);
    let ring = w::progress_ring();
    let action = gtk::Button::with_label("Check");
    action.add_css_class("get-button");
    action.set_valign(gtk::Align::Center);
    let head = w::hbox(w::S3);
    head.add_css_class("list-row");
    head.add_css_class("hero-row");
    head.append(&ic);
    head.append(&text);
    head.append(&ring);
    head.append(&action);
    let list = w::vbox(0);
    p.append(&w::section("", &[head.upcast()]));
    let list_section = w::section("Packages", &[list.clone().upcast()]);
    p.append(&list_section);
    let state = Rc::new(RefCell::new(UpdateState::Idle));
    let show: Rc<dyn Fn(UpdateState)> = {
        let (title, detail, ring, action, list, ls, state) = (
            title.clone(),
            detail.clone(),
            ring.clone(),
            action.clone(),
            list.clone(),
            list_section.clone(),
            state.clone(),
        );
        Rc::new(move |s: UpdateState| {
            let busy = matches!(s, UpdateState::Checking | UpdateState::Installing);
            ring.set_visible(busy);
            action.set_visible(!busy && s != UpdateState::Installed);
            action.set_label(if matches!(s, UpdateState::Available(_)) {
                "Update All"
            } else {
                "Check"
            });
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            ls.set_visible(matches!(s, UpdateState::Available(_)));
            let (t, d) = match &s {
                UpdateState::Idle => (
                    "Check for updates".to_string(),
                    "See what is new for your apps.".to_string(),
                ),
                UpdateState::Checking => ("Checking for updates…".into(), String::new()),
                UpdateState::UpToDate => ("All apps are up to date".into(), String::new()),
                UpdateState::Available(v) => {
                    for n in v.iter().take(50) {
                        list.append(&w::list_row("package-x-generic-symbolic", n, "", None));
                    }
                    (
                        format!(
                            "{} update{} available",
                            v.len(),
                            if v.len() == 1 { "" } else { "s" }
                        ),
                        "WolOS saves a restore point before updating.".into(),
                    )
                }
                UpdateState::Installing => (
                    "Installing updates…".into(),
                    "You can keep using your apps.".into(),
                ),
                UpdateState::Installed => (
                    "Updates installed".into(),
                    "Restart to finish updating.".into(),
                ),
                UpdateState::Failed(e) => ("Updates".into(), e.clone()),
            };
            title.set_label(&t);
            detail.set_label(&d);
            detail.set_visible(!d.is_empty());
            *state.borrow_mut() = s;
        })
    };
    show(UpdateState::Idle);
    {
        let (show, ctx) = (show.clone(), sh.ctx.clone());
        action.connect_clicked(move |_| {
            if matches!(*state.borrow(), UpdateState::Available(_)) {
                show(UpdateState::Installing);
                let show = show.clone();
                return ctx.act(
                    Action::PkgUpgrade,
                    Rc::new(move |r| {
                        show(match r {
                            Ok(()) => UpdateState::Installed,
                            Err(e) => UpdateState::Failed(crate::copy::update(e).into()),
                        })
                    }),
                );
            }
            show(UpdateState::Checking);
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let out = wol_core::system::run_with_timeout(
                    &["wol", "update", "check", "--json"],
                    std::time::Duration::from_secs(120),
                );
                let _ = tx.send_blocking(out);
            });
            let show = show.clone();
            glib::spawn_future_local(async move {
                if let Ok(out) = rx.recv().await {
                    show(parse_check(out.as_deref()));
                }
            });
        });
    }
    p
}

/// Search results: catalog hits now, `pkg search` hits when they arrive.
fn fill_search(sh: &Rc<Shared>, page: &gtk::Box, term: &str) {
    while let Some(c) = page.first_child() {
        page.remove(&c);
    }
    page.append(&w::label(&format!("Results for “{term}”"), "large-title"));
    let hits = store::search(term);
    if !hits.is_empty() {
        page.append(&grid(hits.into_iter().map(|a| tile(sh, a))));
    }
    let more = w::vbox(w::S2);
    page.append(&more);
    let empty = w::empty_state(
        "system-search-symbolic",
        "No results",
        "Try a different word, like “photo” or “music”.",
        None,
    );
    empty.set_visible(store::search(term).is_empty());
    page.append(&empty);
    if !sh.env.pkg_search {
        return;
    }
    let (tx, rx) = async_channel::bounded(1);
    let t = term.to_string();
    std::thread::spawn(move || {
        let out = wol_core::system::run_with_timeout(
            &["pkg", "search", "--", &t],
            std::time::Duration::from_secs(15),
        );
        let _ = tx.send_blocking(out.map(|o| store::parse_pkg_search(&o)).unwrap_or_default());
    });
    let sh = sh.clone();
    glib::spawn_future_local(async move {
        let Ok(found) = rx.recv().await else { return };
        if found.is_empty() || more.parent().is_none() {
            return;
        }
        empty.set_visible(false);
        more.append(&w::label("More packages", "title"));
        more.append(&grid(found.iter().take(30).map(|h| pkg_tile(&sh, h))));
    });
}

/// List row whose subtitle wraps (trust sentences in narrow windows).
fn note_row(icon: &str, title: &str, subtitle: &str) -> gtk::Widget {
    let row = w::list_row(icon, title, "", None);
    if let Some(t) = row
        .first_child()
        .and_then(|c| c.next_sibling())
        .and_downcast::<gtk::Box>()
    {
        let l = w::label(subtitle, "secondary");
        l.set_wrap(true);
        t.append(&l);
    }
    row.upcast()
}

/// The file's own `icon.png`, else its store icon (the generic package icon when none ships).
fn package_icon(f: &PackageFile, size: i32) -> gtk::Widget {
    let png = f
        .icon
        .as_ref()
        .and_then(|b| gtk::gdk::Texture::from_bytes(&glib::Bytes::from(b)).ok());
    if let Some(t) = png {
        let img = gtk::Image::from_paintable(Some(&t));
        img.set_pixel_size(size);
        return img.upcast();
    }
    let name = match &f.verified {
        Ok(v) if !v.info.icon.is_empty() => v.info.icon.as_str(),
        Ok(v) => v.info.id.as_str(),
        Err(_) => "",
    };
    w::store_icon(name, size)
}

/// Install page: icon, name, Install, who made it (trust) and what it uses. Install -> Open.
fn fill_package(sh: &Rc<Shared>, page: &gtk::Box, f: Option<PackageFile>) {
    while let Some(c) = page.first_child() {
        page.remove(&c);
    }
    say(&sh.status, "");
    let Some(f) = f else {
        let b = w::hbox(w::S3);
        b.append(&w::progress_ring());
        b.append(&w::label("Checking the app…", "secondary"));
        page.append(&b);
        return;
    };
    let s = package_sheet(&f);
    let head = w::hbox(w::S6);
    head.append(&package_icon(&f, w::HERO_ICON));
    let text = w::vbox(w::S1);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let name = w::label(&s.name, "large-title");
    name.set_wrap(true);
    text.append(&name);
    for t in [&s.summary, &s.detail] {
        if !t.is_empty() {
            let l = w::label(t, "secondary");
            l.set_wrap(true);
            text.append(&l);
        }
    }
    let get = gtk::Button::with_label(s.button);
    get.add_css_class("get-button");
    get.set_halign(gtk::Align::Start);
    get.set_margin_top(w::S2);
    // A refused file has no button: nothing on the page can install it.
    get.set_visible(s.install.is_some());
    text.append(&get);
    head.append(&text);
    page.append(&head);
    page.append(&w::section(
        "",
        &[note_row(s.trust.0, s.trust.1, s.trust.2)],
    ));
    let (Ok(v), Some(trust)) = (&f.verified, s.install) else {
        return;
    };
    let rows: Vec<gtk::Widget> = if s.capabilities.is_empty() {
        vec![
            w::list_row(
                "object-select-symbolic",
                "Nothing beyond its own windows",
                "",
                None,
            )
            .upcast(),
        ]
    } else {
        s.capabilities
            .iter()
            .map(|(i, l)| w::list_row(i, l, "", None).upcast())
            .collect()
    };
    page.append(&w::section("What it uses", &rows));
    let installed = Rc::new(std::cell::Cell::new(s.button == "Open"));
    let (sh, id, path, label) = (sh.clone(), v.info.id.clone(), f.path.clone(), s.button);
    get.connect_clicked(move |b| {
        if installed.get() {
            kit::spawn(&["gtk4-launch".into(), id.clone()]);
            return;
        }
        b.set_sensitive(false);
        b.set_label("Installing…");
        say(&sh.status, "");
        let (b, st, inst, name) = (
            b.clone(),
            sh.status.clone(),
            installed.clone(),
            s.name.clone(),
        );
        sh.ctx.act(
            Action::Admin(shared::admin::AdminRequest {
                action: shared::admin::AdminAction::WolInstall,
                args: vec![path.clone(), trust.into()],
            }),
            Rc::new(move |r: Result<(), ActionError>| {
                b.set_sensitive(true);
                match r {
                    Ok(()) => {
                        inst.set(true);
                        b.set_label("Open");
                    }
                    Err(e) => {
                        b.set_label(label);
                        say(&st, &crate::copy::install(e, &name));
                    }
                }
            }),
        );
    });
}

pub fn content(sys: Rc<dyn System>, env: Env) -> Store {
    let aw = w::AppWindow::new("App Store", true, false);
    let ctx = Ctx::new(sys, "App Store");
    let status = w::label("", "secondary");
    status.set_wrap(true);
    status.set_visible(false);
    status.add_css_class("store-status");
    let sh = Rc::new(Shared {
        ctx,
        env,
        status: status.clone(),
    });
    let search = aw.sidebar_search("Search");
    aw.source_item("starred-symbolic", "Discover", &discover(&sh));
    for (c, icon) in [
        (Category::Create, "applications-graphics-symbolic"),
        (Category::Work, "applications-office-symbolic"),
        (Category::Play, "applications-games-symbolic"),
        (Category::Develop, "applications-engineering-symbolic"),
    ] {
        aw.source_item(icon, c.name(), &category(&sh, c));
    }
    aw.source_item(
        "software-update-available-symbolic",
        "Updates",
        &updates(&sh),
    );
    let results = w::vbox(w::S6);
    results.add_css_class("page");
    aw.hidden_page("Search", &results);
    let show_results: Rc<dyn Fn(&str)> = {
        let (sh, results, content, side) = (
            sh.clone(),
            results.clone(),
            aw.content.clone(),
            aw.sidebar.clone(),
        );
        Rc::new(move |term: &str| {
            side.unselect_all();
            fill_search(&sh, &results, term);
            content.set_visible_child_name("Search");
        })
    };
    let package_page = w::vbox(w::S6);
    package_page.add_css_class("page");
    aw.hidden_page("Package", &package_page);
    let show_package: Rc<dyn Fn(Option<PackageFile>)> = {
        let (sh, page, content, side) = (
            sh.clone(),
            package_page.clone(),
            aw.content.clone(),
            aw.sidebar.clone(),
        );
        Rc::new(move |f| {
            side.unselect_all();
            fill_package(&sh, &page, f);
            content.set_visible_child_name("Package");
        })
    };
    // Error line pinned under the content (inline, one sentence).
    w::toolbar_place(&aw, &w::sheet(&status));
    sh.ctx.place(&aw);
    {
        let (sh, results, content, sidebar) = (
            sh.clone(),
            results.clone(),
            aw.content.clone(),
            aw.sidebar.clone(),
        );
        let timer: Rc<RefCell<Option<glib::SourceId>>> = Default::default();
        search.connect_search_changed(move |e| {
            if let Some(t) = timer.borrow_mut().take() {
                t.remove();
            }
            let term = e.text().trim().to_string();
            if term.is_empty() {
                match sidebar.selected_row() {
                    Some(r) => content.set_visible_child_name(&r.widget_name()),
                    None => sidebar.select_row(sidebar.row_at_index(1).as_ref()),
                }
                return;
            }
            let (sh, results, content, timer2) =
                (sh.clone(), results.clone(), content.clone(), timer.clone());
            *timer.borrow_mut() = Some(glib::timeout_add_local_once(
                std::time::Duration::from_millis(250),
                move || {
                    timer2.borrow_mut().take();
                    fill_search(&sh, &results, &term);
                    content.set_visible_child_name("Search");
                },
            ));
        });
    }
    // The status sheet shows itself when there is something to say.
    if let Some(r) = status
        .ancestor(gtk::Revealer::static_type())
        .and_downcast::<gtk::Revealer>()
    {
        status.connect_visible_notify(move |l| r.set_reveal_child(l.is_visible()));
    }
    Store {
        aw,
        search,
        show_results,
        show_package,
    }
}
