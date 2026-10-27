//! System Settings on the HIG AppWindow: a searchable sidebar of panes (System Settings order) and
//! grouped Section cards. Every control applies for real or is not shown: theme contract
//! (Appearance), desktop prefs applied live through one `hyprctl` batch and journaled (Desktop &
//! Dock, Effects & Looks, Keyboard, Trackpad & Mouse, Accessibility), shell prefs + `reload`,
//! `System` actions and wol-admind for root changes (the Authenticate sheet asks first). Machine
//! facts load off the UI thread ([`Machine`]); panes are built the first time they are shown.
use crate::kit::{self, form, widgets as w};
use crate::system::{Action, ActionError, System};
use gtk::{gio, glib, prelude::*};
use shared::theme::{Color, Variant};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use wol_core::prefs::{self, ShellPrefs};
use wol_core::theme::accent;

mod about;
mod accessibility;
mod accounts;
mod appearance;
mod assistants;
mod battery;
mod bluetooth;
mod control_center;
mod datetime;
mod desktop_dock;
mod displays;
mod effects;
mod general;
mod hardware;
mod keyboard;
mod language;
mod lock;
mod login_items;
mod machine;
mod network;
mod notifications;
mod printers;
mod privacy;
mod restore;
mod sharing;
mod shortcuts;
mod software_update;
mod sound;
mod storage;
mod trackpad;
mod wallpaper;
mod widgets;
mod wifi;

pub use accounts::{account_name, password_problem};
pub use machine::{Demo, Kernel, Live, Machine, Sharing};
pub use shortcuts::KEY_LEGEND;
pub use software_update::{UpdateState, parse_check};

/// One sidebar pane: its title (also the `wol-settings <title>` name), icon and search words.
pub struct Pane {
    pub title: &'static str,
    pub icon: &'static str,
    pub words: &'static str,
}

const fn pane(title: &'static str, icon: &'static str, words: &'static str) -> Pane {
    Pane { title, icon, words }
}

/// The sidebar, top to bottom; groups are separated by a gap.
pub const SIDEBAR: [&[Pane]; 6] = [
    &[
        pane(
            "Wi-Fi",
            "network-wireless-symbolic",
            "wifi wireless network internet join password hotspot",
        ),
        pane(
            "Bluetooth",
            "bluetooth-active-symbolic",
            "devices headphones airpods keyboard mouse pair connect",
        ),
        pane(
            "Network",
            "network-wired-symbolic",
            "ethernet ip address router dns gateway mac",
        ),
    ],
    &[
        pane(
            "Notifications",
            "preferences-system-notifications-symbolic",
            "focus do not disturb banners alerts quiet",
        ),
        pane(
            "Sound",
            "audio-volume-high-symbolic",
            "volume speakers headphones output mute audio",
        ),
    ],
    &[
        pane(
            "General",
            "emblem-system-symbolic",
            "about version software update storage disk date time zone clock language region locale login items startup sharing computer name remote ssh",
        ),
        pane(
            "Appearance",
            "preferences-desktop-appearance-symbolic",
            "theme dark light auto accent colour color palette",
        ),
        pane(
            "Accessibility",
            "preferences-desktop-accessibility-symbolic",
            "reduce motion transparency contrast zoom cursor size",
        ),
        pane(
            "Control Center",
            "wolos-control-center-symbolic",
            "modules now playing battery keyboard brightness screenshot record",
        ),
        pane(
            "Desktop & Dock",
            "wolos-dock-symbolic",
            "dock menu bar magnification hide tiling tile windows gaps layout",
        ),
        pane(
            "Displays",
            "video-display-symbolic",
            "resolution scale monitor night shift brightness",
        ),
        pane(
            "Wallpaper",
            "preferences-desktop-wallpaper-symbolic",
            "background picture desktop dynamic photo",
        ),
        pane(
            "Effects & Looks",
            "wolos-effects-symbolic",
            "blur transparency opacity glass corners rounding radius borders gradient shadows animations speed gaps dim presets rice",
        ),
        pane(
            "Widgets",
            "wolos-widgets-symbolic",
            "desktop widgets clock weather calendar arrange",
        ),
        pane(
            "Battery",
            "battery-full-symbolic",
            "energy power low power mode health lid sleep",
        ),
    ],
    &[
        pane(
            "Lock Screen",
            "system-lock-screen-symbolic",
            "lock idle screen saver sleep password",
        ),
        pane(
            "Privacy & Security",
            "security-high-symbolic",
            "permissions screen recording firewall consent privacy",
        ),
        pane(
            "Users & Groups",
            "system-users-symbolic",
            "accounts users administrator password shell zsh",
        ),
    ],
    &[
        pane(
            "Keyboard",
            "input-keyboard-symbolic",
            "key repeat input sources layout shortcuts",
        ),
        pane(
            "Trackpad & Mouse",
            "input-touchpad-symbolic",
            "pointer speed scroll natural tap click touchpad",
        ),
        pane(
            "Printers",
            "printer-symbolic",
            "print printer scanner default",
        ),
        pane(
            "Hardware",
            "wolos-chip-symbolic",
            "drivers firmware devices webcam graphics fix",
        ),
    ],
    &[
        pane(
            "Assistants & Automation",
            "wolos-assistant-symbolic",
            "ai helpers mcp scripts automation permissions",
        ),
        pane(
            "Restore",
            "wolos-restore-symbolic",
            "undo history restore points boot environments",
        ),
    ],
];

/// Every sidebar pane title, in order (same as [`SIDEBAR`]).
pub const PAGES: [&str; 24] = [
    "Wi-Fi",
    "Bluetooth",
    "Network",
    "Notifications",
    "Sound",
    "General",
    "Appearance",
    "Accessibility",
    "Control Center",
    "Desktop & Dock",
    "Displays",
    "Wallpaper",
    "Effects & Looks",
    "Widgets",
    "Battery",
    "Lock Screen",
    "Privacy & Security",
    "Users & Groups",
    "Keyboard",
    "Trackpad & Mouse",
    "Printers",
    "Hardware",
    "Assistants & Automation",
    "Restore",
];

/// Pages without a sidebar row, reached from a pane (`wol-settings "Storage"` works too).
pub const SUBPAGES: [&str; 8] = [
    shortcuts::TITLE,
    "About",
    "Software Update",
    "Storage",
    "Date & Time",
    "Language & Region",
    "Login Items",
    "Sharing",
];

/// Older page names (links, scripts) -> today's.
const ALIASES: [(&str, &str); 6] = [
    ("Menu Bar & Dock", "Desktop & Dock"),
    ("Lock & Idle", "Lock Screen"),
    ("Accounts", "Users & Groups"),
    ("Energy", "Battery"),
    ("Effects", "Effects & Looks"),
    ("Shortcuts", shortcuts::TITLE),
];

/// Today's name for `name` (case-insensitive aliases).
pub fn canonical(name: &str) -> &str {
    ALIASES
        .iter()
        .find(|a| a.0.eq_ignore_ascii_case(name))
        .map_or(name, |a| a.1)
}

/// Panes whose title or search words contain every word of `query`.
pub fn search(query: &str) -> Vec<&'static str> {
    let q: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    SIDEBAR
        .iter()
        .flat_map(|g| g.iter())
        .filter(|p| {
            let hay = format!("{} {}", p.title, p.words).to_lowercase();
            q.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|p| p.title)
        .collect()
}

pub fn run() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("org.wolos.settings")
        .build();
    app.connect_activate(|app| {
        kit::style::install("");
        let win = gtk::ApplicationWindow::builder()
            .application(app)
            .title("System Settings")
            .default_width(w::APP_SIZE.0)
            .default_height(w::APP_SIZE.1)
            .decorated(false)
            .build();
        let aw = content(crate::system::default_system(), &Host::read());
        if let Some(p) = std::env::args().nth(1) {
            aw.open(canonical(&p));
        }
        win.set_child(Some(&aw.root));
        kit::snapshot_on_map(&win, "settings");
        win.present();
    });
    app.run_with_args::<&str>(&[])
}

/// Host facts shown by the panes (injected so golden renders do not depend on the build machine).
#[derive(Clone)]
pub struct Host {
    pub hostname: String,
    pub users: Vec<String>,
    pub os: String,
    /// "FreeBSD 15.1-RELEASE-p1" (`kern.ostype` + `kern.osrelease`).
    pub base: String,
    pub processor: String,
    pub memory: String,
    /// Slower facts (storage, hardware, network...), always read off the UI thread.
    pub machine: Arc<dyn Machine>,
}

impl Host {
    pub fn read() -> Self {
        let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
        let shells = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let info = wol_core::system::info::SystemInfo::read(&wol_core::system::PosixHost);
        Host {
            hostname: std::fs::read_to_string("/etc/hostname")
                .ok()
                .or_else(|| {
                    wol_core::system::run_with_timeout(&["hostname"], wol_core::system::RUN_TIMEOUT)
                })
                .unwrap_or_default()
                .trim()
                .to_string(),
            users: crate::login::users(&passwd, &shells),
            os: info.kernel,
            base: info.base,
            processor: info.processor,
            memory: info.memory,
            machine: Arc::new(Live::new()),
        }
    }

    /// The fixed laptop of golden renders and screenshots.
    pub fn demo() -> Self {
        Host {
            hostname: "wonderland".into(),
            users: vec!["alice".into(), "dinah".into()],
            os: "WolKernel".into(),
            base: "FreeBSD 15.1-RELEASE-p1".into(),
            processor: "Intel Core i5-1240P (12 cores)".into(),
            memory: "16 GB".into(),
            machine: Arc::new(Demo),
        }
    }
}

/// `$XDG_CONFIG_HOME/<rel>`.
fn config(rel: &str) -> PathBuf {
    PathBuf::from(kit::config_home()).join(rel)
}

fn read(p: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

/// Atomic write (watchers never see half a file).
/// Settings file change recorded in the journal (`wol undo`, Settings > Restore).
fn save_journaled(what: &str, rel: &str, text: Option<&str>) -> std::io::Result<()> {
    wol_core::journal::Journal::for_config(&PathBuf::from(kit::config_home())).save(
        what,
        "settings",
        &[(rel, text)],
        wol_core::journal::now(),
    )?;
    changed(what);
    Ok(())
}

/// Called with what changed (as in the journal).
type ChangeHook = Rc<dyn Fn(&str)>;

thread_local! {
    /// The Settings window's hook for journaled changes (its Undo toast); unset elsewhere.
    static CHANGED: RefCell<Option<ChangeHook>> = const { RefCell::new(None) };
}

/// Tells the window that a journaled change (`what`, as in the journal) was just saved.
fn changed(what: &str) {
    if let Some(f) = CHANGED.with(|c| c.borrow().clone()) {
        f(what)
    }
}

/// Runs argv lists in order on a worker thread (2 s timeout each); never blocks the UI.
fn background(cmds: Vec<Vec<String>>) {
    std::thread::spawn(move || {
        for c in cmds {
            let argv: Vec<&str> = c.iter().map(String::as_str).collect();
            let _ = wol_core::system::run_with_timeout(&argv, wol_core::system::RUN_TIMEOUT);
        }
    });
}

/// wol-admind request `name args…` (wire name, `shared::admin`), None when this build has no such
/// verb: the control that needs it is not shown.
fn admin(name: &str, args: &[&str]) -> Option<Action> {
    let action = shared::admin::AdminAction::parse(name)?;
    Some(Action::Admin(shared::admin::AdminRequest {
        action,
        args: argv(args),
    }))
}

fn argv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Fills `into` with `render(load())`. `load` runs off the UI thread; a spinner shows only when it
/// takes longer than 150 ms (HIG: never a blank pane, no flash for fast loads).
fn later<T: Send + 'static>(
    into: &gtk::Box,
    load: impl FnOnce() -> T + Send + 'static,
    render: impl FnOnce(&gtk::Box, T) + 'static,
) {
    let spinner = w::spinner();
    spinner.set_visible(false);
    into.append(&spinner);
    let done = Rc::new(Cell::new(false));
    {
        let (s, done) = (spinner.downgrade(), done.clone());
        glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            if let (false, Some(s)) = (done.get(), s.upgrade()) {
                s.set_visible(true);
            }
        });
    }
    let into = into.clone();
    glib::spawn_future_local(async move {
        let r = gio::spawn_blocking(load).await;
        done.set(true);
        into.remove(&spinner);
        if let Ok(v) = r {
            render(&into, v);
        }
    });
}

/// `then(load())`, `load` off the UI thread.
fn off_thread<T: Send + 'static>(
    load: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(T) + 'static,
) {
    glib::spawn_future_local(async move {
        if let Ok(v) = gio::spawn_blocking(load).await {
            then(v)
        }
    });
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// Completion callback for one action.
pub(crate) type Done = Rc<dyn Fn(Result<(), ActionError>)>;

/// Shared page state: the System, the Authenticate sheet and an inline status line per action.
pub(crate) struct Ctx {
    pub(crate) sys: Rc<dyn System>,
    auth: w::AuthSheet,
    pending: RefCell<Option<(Action, Done)>>,
}

impl Ctx {
    /// Wires the Authenticate sheet; place it on `aw` after any other sheets.
    pub(crate) fn new(sys: Rc<dyn System>, who: &str) -> Rc<Self> {
        let ctx = Rc::new(Ctx {
            sys,
            auth: w::authenticate_sheet(who),
            pending: Default::default(),
        });
        let c = ctx.clone();
        ctx.auth.allow.connect_clicked(move |_| c.allow());
        let c = ctx.clone();
        ctx.auth.field.connect_activate(move |_| c.allow());
        let c = ctx.clone();
        ctx.auth.cancel.connect_clicked(move |_| c.cancel());
        ctx
    }
    pub(crate) fn place(&self, aw: &w::AppWindow) {
        w::toolbar_place(aw, &self.auth.revealer);
    }
    /// `a` off the UI thread; root actions ask for the password, then retry; `done` gets the final result.
    pub(crate) fn act(self: &Rc<Self>, a: Action, done: Rc<dyn Fn(Result<(), ActionError>)>) {
        let rx = self.sys.spawn(a.clone());
        let c = self.clone();
        glib::spawn_future_local(async move {
            let Ok(r) = rx.recv().await else { return };
            match r {
                Err(e @ (ActionError::NeedsAuth | ActionError::Denied)) => {
                    c.auth.message.set_label(if e == ActionError::Denied {
                        e.message()
                    } else {
                        ""
                    });
                    *c.pending.borrow_mut() = Some((a, done));
                    c.auth.revealer.set_reveal_child(true);
                    c.auth.field.grab_focus();
                }
                r => done(r),
            }
        });
    }
    /// `a`, then its one-sentence error (if any) on `status`.
    pub(crate) fn act_say(self: &Rc<Self>, a: Action, status: &gtk::Label) {
        let st = status.clone();
        self.act(
            a,
            Rc::new(move |r| say(&st, r.err().map_or("", |e| e.message()))),
        );
    }
    fn allow(self: &Rc<Self>) {
        let pw = self.auth.field.text();
        self.auth.field.set_text("");
        if let Err(e) = self.sys.authorize(&pw) {
            return self.auth.message.set_label(e.message());
        }
        self.auth.revealer.set_reveal_child(false);
        if let Some((a, done)) = self.pending.borrow_mut().take() {
            self.act(a, done);
        }
    }
    fn cancel(&self) {
        if let Some((_, done)) = self.pending.borrow_mut().take() {
            done(Err(ActionError::Denied));
        }
        self.auth.revealer.set_reveal_child(false);
    }
}

/// Inline one-sentence status under a section (HIG §3 errors).
fn status_label() -> gtk::Label {
    let l = w::label("", "secondary");
    l.set_wrap(true);
    l.set_visible(false);
    l.add_css_class("status-line");
    l
}

fn say(l: &gtk::Label, text: &str) {
    l.set_label(text);
    l.set_visible(!text.is_empty());
}

/// Everything a pane needs: actions, shared prefs, host facts, sheets and navigation.
pub(crate) struct Env {
    pub(crate) ctx: Rc<Ctx>,
    pub(crate) desk: Rc<Desk>,
    pub(crate) keys: Rc<Keys>,
    pub(crate) host: Host,
    sheets: Sheets,
    open: Rc<dyn Fn(&str)>,
    /// Keyboard Shortcuts opens on this group (Desktop & Dock > Tiling Shortcuts…).
    pub(crate) shortcut_group: Cell<Option<wol_core::shortcuts::Group>>,
    /// Builds the open pages again from the saved files (after an undo or an outside change).
    refresh: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Env {
    pub(crate) fn sys(&self) -> &Rc<dyn System> {
        &self.ctx.sys
    }
    pub(crate) fn machine(&self) -> Arc<dyn Machine> {
        self.host.machine.clone()
    }
    /// Shows page `name` (subpage links, back buttons).
    pub(crate) fn open(&self, name: &str) {
        (self.open)(name)
    }
    /// Shows every built page again from the saved files.
    pub(crate) fn refresh(&self) {
        let f = self.refresh.borrow().clone();
        if let Some(f) = f {
            f()
        }
    }
    /// A closure that opens `name` (for button callbacks).
    pub(crate) fn opener(self: &Rc<Self>, name: &'static str) -> impl Fn() + 'static {
        let o = self.open.clone();
        move || o(name)
    }
}

type Build = Box<dyn FnOnce() -> gtk::Widget>;

/// Builds `name`'s page into its holder the first time it shows.
fn build_page(env: &Rc<Env>, name: &str) -> Option<Build> {
    let e = env.clone();
    let b: Build = match name {
        "Wi-Fi" => Box::new(move || wifi::page(&e).upcast()),
        "Bluetooth" => Box::new(move || bluetooth::page(&e).upcast()),
        "Network" => Box::new(move || network::page(&e).upcast()),
        "Notifications" => Box::new(move || notifications::page(&e).upcast()),
        "Sound" => Box::new(move || sound::page(&e.ctx).upcast()),
        "General" => Box::new(move || general::page(&e).upcast()),
        "Appearance" => Box::new(move || appearance::page(&e).upcast()),
        "Accessibility" => Box::new(move || accessibility::page(&e).upcast()),
        "Control Center" => Box::new(move || control_center::page(&e).upcast()),
        "Desktop & Dock" => Box::new(move || desktop_dock::page(&e).upcast()),
        "Displays" => Box::new(move || displays::page(&e.ctx).upcast()),
        "Wallpaper" => Box::new(move || wallpaper::page(&e).upcast()),
        "Effects & Looks" => Box::new(move || effects::page(&e).upcast()),
        "Widgets" => Box::new(move || widgets::page(&e).upcast()),
        "Battery" => Box::new(move || battery::page(&e).upcast()),
        "Lock Screen" => Box::new(move || lock::page(&e).upcast()),
        "Privacy & Security" => Box::new(move || privacy::page(&e).upcast()),
        "Users & Groups" => Box::new(move || accounts::page(&e).upcast()),
        "Keyboard" => Box::new(move || keyboard::page(&e).upcast()),
        "Trackpad & Mouse" => Box::new(move || trackpad::page(&e).upcast()),
        "Printers" => Box::new(move || printers::page(&e).upcast()),
        "Hardware" => Box::new(move || hardware::page(&e).upcast()),
        "Assistants & Automation" => Box::new(move || assistants::page(&e).upcast()),
        "Restore" => Box::new(move || restore::page(&e).upcast()),
        shortcuts::TITLE => Box::new(move || shortcuts::page(&e).upcast()),
        "About" => Box::new(move || about::page(&e).upcast()),
        "Software Update" => Box::new(move || software_update::page(&e).upcast()),
        "Storage" => Box::new(move || storage::page(&e).upcast()),
        "Date & Time" => Box::new(move || datetime::page(&e).upcast()),
        "Language & Region" => Box::new(move || language::page(&e).upcast()),
        "Login Items" => Box::new(move || login_items::page(&e).upcast()),
        "Sharing" => Box::new(move || sharing::page(&e).upcast()),
        _ => return None,
    };
    Some(b)
}

pub fn content(sys: Rc<dyn System>, host: &Host) -> w::AppWindow {
    let aw = w::AppWindow::new("System Settings", true, false);
    let ctx = Ctx::new(sys.clone(), "System Settings");
    let sheets = Sheets {
        join: w::toolbar_sheet(&aw, &w::vbox(0)),
        user: w::toolbar_sheet(&aw, &w::vbox(0)),
    };
    // Added last: the Authenticate sheet sits above the others.
    ctx.place(&aw);
    let open: Rc<dyn Fn(&str)> = {
        let (side, stack) = (aw.sidebar.downgrade(), aw.content.downgrade());
        Rc::new(move |name: &str| {
            let (Some(sidebar), Some(content)) = (side.upgrade(), stack.upgrade()) else {
                return;
            };
            let name = canonical(name);
            let mut i = 0;
            while let Some(r) = sidebar.row_at_index(i) {
                if r.widget_name() == name {
                    sidebar.select_row(Some(&r));
                    content.set_visible_child_name(name);
                    return;
                }
                i += 1;
            }
            if content.child_by_name(name).is_some() {
                sidebar.unselect_all();
                content.set_visible_child_name(name);
            }
        })
    };
    let env = Rc::new(Env {
        desk: Desk::load(&ctx),
        keys: Keys::load(&ctx),
        ctx,
        host: host.clone(),
        sheets,
        open,
        shortcut_group: Cell::new(None),
        refresh: RefCell::default(),
    });

    let field = aw.sidebar_search("Search");
    let pending: Rc<RefCell<HashMap<String, (gtk::Box, Build)>>> = Default::default();
    let holders: Rc<RefCell<Vec<(&'static str, gtk::Box)>>> = Default::default();
    let laptop = sys.battery().is_some();
    for (i, group) in SIDEBAR.iter().enumerate() {
        if i > 0 {
            aw.source_gap();
        }
        for p in group.iter() {
            let holder = w::vbox(0);
            aw.source_item(p.icon, p.title, &holder);
            if p.title == "Battery" && !laptop {
                // Desktops: the same pane, named like the Mac's.
                retitle(&aw, p.title, "Energy", "power-profile-balanced-symbolic");
            }
            holders.borrow_mut().push((p.title, holder.clone()));
            if let Some(b) = build_page(&env, p.title) {
                pending.borrow_mut().insert(p.title.into(), (holder, b));
            }
        }
    }
    for name in SUBPAGES {
        let holder = w::vbox(0);
        aw.hidden_page(name, &holder);
        holders.borrow_mut().push((name, holder.clone()));
        if let Some(b) = build_page(&env, name) {
            pending.borrow_mut().insert(name.into(), (holder, b));
        }
    }
    let ensure = {
        let pending = pending.clone();
        move |stack: &gtk::Stack| {
            let Some(name) = stack.visible_child_name() else {
                return;
            };
            let next = pending.borrow_mut().remove(name.as_str());
            if let Some((holder, build)) = next {
                holder.append(&build());
            }
        }
    };
    ensure(&aw.content);
    // After an undo every built page is built again from the restored files.
    let rebuild = {
        let (env, pending, holders, stack, ensure) = (
            Rc::downgrade(&env),
            pending.clone(),
            holders.clone(),
            aw.content.downgrade(),
            ensure.clone(),
        );
        move || {
            let (Some(stack), Some(env)) = (stack.upgrade(), env.upgrade()) else {
                return;
            };
            env.desk.reload();
            env.keys.reload();
            for (name, holder) in holders.borrow().iter() {
                if pending.borrow().contains_key(*name) {
                    continue;
                }
                if let Some(b) = build_page(&env, name) {
                    clear(holder);
                    pending
                        .borrow_mut()
                        .insert((*name).into(), (holder.clone(), b));
                }
            }
            ensure(&stack);
        }
    };
    let rebuild: Rc<dyn Fn()> = Rc::new(rebuild);
    *env.refresh.borrow_mut() = Some(rebuild);
    undo_toast(&aw, &env);
    aw.content
        .connect_visible_child_name_notify(move |s| ensure(s));

    // Search: rows whose title or words match; gaps hide while searching; Enter opens the first.
    let query: Rc<RefCell<String>> = Default::default();
    {
        let q = query.clone();
        aw.sidebar.set_filter_func(move |row| {
            let q = q.borrow();
            if q.trim().is_empty() || row.has_css_class("sidebar-search") {
                return true;
            }
            let name = row.widget_name();
            !name.is_empty() && search(&q).contains(&name.as_str())
        });
    }
    {
        let (q, side) = (query.clone(), aw.sidebar.downgrade());
        field.connect_search_changed(move |s| {
            *q.borrow_mut() = s.text().to_string();
            if let Some(side) = side.upgrade() {
                side.invalidate_filter();
            }
        });
    }
    {
        let env = env.clone();
        field.connect_activate(move |s| {
            if let Some(first) = search(&s.text()).first() {
                env.open(first);
            }
        });
    }
    aw
}

/// After each journaled change: "Wallpaper changed · Undo". Undo reverts every change made while
/// the toast was up (a slider drag is many saves), newest first, then refreshes the pages.
fn undo_toast(aw: &w::AppWindow, env: &Rc<Env>) {
    let toast = kit::toast::Toast::new(aw);
    let burst: Rc<RefCell<Vec<u64>>> = Rc::default();
    let rebuild = Rc::downgrade(env);
    let sys = env.sys().clone();
    let hook: ChangeHook = {
        let toast = Rc::downgrade(&toast);
        Rc::new(move |what: &str| {
            let Some(t) = toast.upgrade() else { return };
            if !t.revealer.reveals_child() {
                burst.borrow_mut().clear();
            }
            if let Some(id) = sys.history().first().map(|e| e.id)
                && !burst.borrow().contains(&id)
            {
                burst.borrow_mut().push(id);
            }
            let (sys, burst, rebuild, toast) = (
                sys.clone(),
                burst.clone(),
                rebuild.clone(),
                Rc::downgrade(&t),
            );
            t.show(&changed_line(what), "Undo", move || {
                let ids: Vec<u64> = burst.borrow_mut().drain(..).rev().collect();
                let failed = ids.into_iter().find_map(|id| sys.undo(Some(id)).err());
                if let Some(env) = rebuild.upgrade() {
                    env.refresh();
                }
                if let Some(t) = toast.upgrade() {
                    t.show(failed.as_deref().unwrap_or("Change undone"), "", || {});
                }
            });
        })
    };
    CHANGED.with(|c| *c.borrow_mut() = Some(hook));
    // The toast lives as long as the window's widgets.
    let keep = toast.clone();
    aw.root.connect_destroy(move |_| {
        let _ = &keep;
        CHANGED.with(|c| c.borrow_mut().take());
    });
}

/// "Wallpaper changed", "Keyboard shortcuts changed".
fn changed_line(what: &str) -> String {
    format!("{} changed", restore::topic(what))
}

/// Sidebar row `title` shows `label` with `icon` (Battery on desktops).
fn retitle(aw: &w::AppWindow, title: &str, label: &str, icon: &str) {
    let mut i = 0;
    while let Some(r) = aw.sidebar.row_at_index(i) {
        i += 1;
        if r.widget_name() != title {
            continue;
        }
        let Some(row) = r.child().and_downcast::<gtk::Box>() else {
            return;
        };
        if let Some(img) = row.first_child().and_downcast::<gtk::Image>() {
            img.set_icon_name(Some(icon));
        }
        if let Some(l) = row.last_child().and_downcast::<gtk::Label>() {
            l.set_label(label);
        }
    }
}

/// Desktop & Dock > Windows on its own with `prefs` (golden renders of the tiling section).
pub fn windows_section(
    sys: Rc<dyn System>,
    prefs: wol_core::hyprprefs::DesktopPrefs,
) -> gtk::Widget {
    let aw = w::AppWindow::new("", false, false);
    let ctx = Ctx::new(sys, "System Settings");
    let desk = Rc::new(Desk {
        ctx: ctx.clone(),
        cur: RefCell::new(prefs),
    });
    let env = Rc::new(Env {
        keys: Keys::load(&ctx),
        desk,
        ctx,
        host: Host::demo(),
        sheets: Sheets {
            join: w::toolbar_sheet(&aw, &w::vbox(0)),
            user: w::toolbar_sheet(&aw, &w::vbox(0)),
        },
        open: Rc::new(|_| {}),
        shortcut_group: Cell::new(None),
        refresh: RefCell::default(),
    });
    desktop_dock::windows(&env)
}

/// Desktop prefs shared by every pane that edits them: one saved file, one apply path.
pub(crate) struct Desk {
    ctx: Rc<Ctx>,
    cur: RefCell<wol_core::hyprprefs::DesktopPrefs>,
}

impl Desk {
    fn load(ctx: &Rc<Ctx>) -> Rc<Self> {
        Rc::new(Desk {
            ctx: ctx.clone(),
            cur: RefCell::new(wol_core::hyprprefs::load_desktop(&config(""))),
        })
    }
    pub(crate) fn get(&self) -> wol_core::hyprprefs::DesktopPrefs {
        self.cur.borrow().clone()
    }
    /// Changes the prefs, saves them and applies the difference live; a failure shows on `status`.
    pub(crate) fn set(
        &self,
        status: &gtk::Label,
        f: impl FnOnce(&mut wol_core::hyprprefs::DesktopPrefs),
    ) {
        f(&mut self.cur.borrow_mut());
        let st = status.clone();
        self.ctx.act(
            Action::Desktop(Box::new(self.get())),
            Rc::new(move |r| {
                say(&st, if r.is_ok() { "" } else { SAVE_FAILED });
                if r.is_ok() {
                    changed("desktop");
                }
            }),
        );
    }
    /// Reads the saved prefs again (after an undo).
    fn reload(&self) {
        *self.cur.borrow_mut() = wol_core::hyprprefs::load_desktop(&config(""));
    }
}

const SAVE_FAILED: &str =
    "This change couldn’t be saved. Check that your home folder has free space.";

/// Keyboard shortcuts: shipped defaults (single source: the shipped hyprland.conf) and the user's changes.
pub(crate) struct Keys {
    ctx: Rc<Ctx>,
    pub(crate) defaults: Vec<wol_core::shortcuts::Shortcut>,
    over: RefCell<wol_core::shortcuts::Overrides>,
}

impl Keys {
    fn load(ctx: &Rc<Ctx>) -> Rc<Self> {
        Rc::new(Keys {
            ctx: ctx.clone(),
            defaults: wol_core::shortcuts::load_defaults(|p| std::fs::read_to_string(p).ok()),
            over: RefCell::new(wol_core::hyprprefs::load_overrides(&config(""))),
        })
    }
    /// Every shortcut as it is now (defaults with changes, then custom ones).
    pub(crate) fn list(&self) -> Vec<wol_core::shortcuts::Shortcut> {
        wol_core::shortcuts::effective(&self.defaults, &self.over.borrow())
    }
    pub(crate) fn overrides(&self) -> wol_core::shortcuts::Overrides {
        self.over.borrow().clone()
    }
    /// Applies `f` to a copy of the overrides; on success saves and rebinds live.
    pub(crate) fn edit(
        &self,
        status: &gtk::Label,
        f: impl FnOnce(&mut wol_core::shortcuts::Overrides) -> Result<(), wol_core::shortcuts::Refused>,
    ) -> Result<(), wol_core::shortcuts::Refused> {
        let mut o = self.overrides();
        f(&mut o)?;
        *self.over.borrow_mut() = o.clone();
        let st = status.clone();
        self.ctx.act(
            Action::Shortcuts(Box::new(o)),
            Rc::new(move |r| {
                say(&st, if r.is_ok() { "" } else { SAVE_FAILED });
                if r.is_ok() {
                    changed("shortcuts");
                }
            }),
        );
        Ok(())
    }
    /// Reads the saved changes again (after an undo).
    fn reload(&self) {
        *self.over.borrow_mut() = wol_core::hyprprefs::load_overrides(&config(""));
    }
    /// WolOS shortcuts pause while one is being recorded.
    pub(crate) fn capture(&self, on: bool) {
        self.ctx.act(Action::ShortcutCapture(on), Rc::new(|_| {}));
    }
}

struct Sheets {
    join: gtk::Revealer,
    user: gtk::Revealer,
}

/// Replace a toolbar sheet's body.
fn sheet_body(r: &gtk::Revealer, body: &impl IsA<gtk::Widget>) {
    if let Some(card) = r.child().and_downcast::<gtk::Box>() {
        clear(&card);
        card.append(body);
    }
}

fn row(title: &str, control: &impl IsA<gtk::Widget>) -> gtk::Widget {
    form::row(title, "", control)
}

fn row_sub(title: &str, subtitle: &str, control: &impl IsA<gtk::Widget>) -> gtk::Widget {
    form::row(title, subtitle, control)
}

fn info(title: &str, value: &str) -> gtk::Widget {
    form::info_row(title, value)
}

fn dropdown(options: &[&str], selected: usize, changed: impl Fn(usize) + 'static) -> gtk::DropDown {
    let d = w::dropdown(options, selected as u32);
    d.connect_selected_notify(move |d| changed(d.selected() as usize));
    d
}

/// Shell prefs file (shell.json), written then announced to the running shell (`reload`).
fn save_shell(p: ShellPrefs) {
    let _ = save_journaled("shell", prefs::SHELL_FILE, Some(&p.to_json()));
    reload_shell();
}

/// Tells the running shell to re-read its files (`reload` on wol-shell.sock).
fn reload_shell() {
    let sock = crate::protocol::socket_path(&kit::runtime_dir());
    std::thread::spawn(move || {
        use std::io::Write;
        if let Ok(mut s) = std::os::unix::net::UnixStream::connect(sock) {
            let _ = s.write_all(b"reload\n");
        }
    });
}

fn load_shell() -> ShellPrefs {
    ShellPrefs::parse(read(&config(prefs::SHELL_FILE)).as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_list_the_sidebar_once_each() {
        let side: Vec<&str> = SIDEBAR
            .iter()
            .flat_map(|g| g.iter().map(|p| p.title))
            .collect();
        assert_eq!(side, PAGES, "PAGES follows the sidebar");
        let mut uniq = PAGES.to_vec();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), PAGES.len(), "each pane once");
        for s in SUBPAGES {
            assert!(!PAGES.contains(&s), "{s} is a subpage");
        }
    }

    #[test]
    fn search_matches_titles_and_words() {
        for (q, want) in [
            ("blur", "Effects & Looks"),
            ("wifi", "Wi-Fi"),
            ("zsh", "Users & Groups"),
            ("time zone", "General"),
            ("tile", "Desktop & Dock"),
            ("firewall", "Privacy & Security"),
            ("UNDO", "Restore"),
            ("driver", "Hardware"),
        ] {
            assert!(search(q).contains(&want), "{q}: {:?}", search(q));
        }
        assert!(search("no such thing").is_empty());
        assert_eq!(search("").len(), 24, "empty query lists every pane");
    }

    #[test]
    fn old_names_still_open() {
        assert_eq!(canonical("lock & idle"), "Lock Screen");
        assert_eq!(canonical("Accounts"), "Users & Groups");
        assert_eq!(canonical("Wi-Fi"), "Wi-Fi");
    }
}
