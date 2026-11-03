// SPDX-License-Identifier: BSD-2-Clause
//! A native window's menus for the menu bar (the global menu). A window's top-level `MENU` nodes
//! (the direct children of the window, role `MENU`, title = the node's name) and their `MENUITEM`
//! children (label = the node's name, `SEPARATOR` for a rule) are the app's menus; nothing about
//! them is drawn by the shell except the bar's titles and popovers. The app may hide the nodes
//! in its own window: a node that is not shown is still a menu.
//!
//! The menus are published on the session bus as `shared::view::Menu` JSON, bounded like a Kit
//! window's menus: the app's own id is the well-known name, the object path is
//! [`theme::menu_object_path`] (the shell's `ui::transient::object_path`, frozen) and the
//! interface is [`INTERFACE`]:
//!
//! - `Get() -> s`: the menus of the window in front (the one that was active last).
//! - `Activate(u menu, s key)`: click the item. The key is the item node's id as decimal text;
//!   only an entry of the published menus is accepted, and the click goes through the same path
//!   as an assistive technology's default action, so the app sees an ordinary `CLICK`.
//! - `Changed()`: the menus differ from the last `Get`.
//!
//! A process takes the name only once an app has menus, from its own thread, so an engine without
//! a session bus (or without menus) is not slowed down and nothing is claimed that is not used.

use super::theme;
use crate::a11y::hub::{Action, Hub};
use crate::a11y::roles::{self, wk};
use crate::scene::{Idx, NIL, SceneRead, state};
use shared::view::{MAX_LABEL, MAX_MENU_ENTRIES, MAX_MENUS, Menu, MenuEntry};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};

/// The interface the menus are published on (the shell's `appmenu::WOLKIT_MENUS`).
pub const INTERFACE: &str = "org.wolos.Menus";

/// A label from a node's text: control characters become spaces, at most [`MAX_LABEL`] bytes cut
/// at a character boundary.
fn label(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        let c = if c.is_control() { ' ' } else { c };
        if out.len() + c.len_utf8() > MAX_LABEL {
            break;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

/// The node's accessible name, else its own text, else the first non-empty text below it
/// (bounded): what a menu or an item is called.
fn name(scene: &dyn SceneRead, n: Idx) -> String {
    let a = scene.a11y(n).name;
    if !a.trim().is_empty() {
        return label(a);
    }
    let own = label(scene.text(n));
    if !own.is_empty() {
        return own;
    }
    let mut found = String::new();
    let mut seen = 0usize;
    crate::scene::walk(scene, n, &mut |i, _| {
        seen += 1;
        if found.is_empty() && seen < 64 && scene.kind(i) == crate::scene::Kind::Text {
            found = label(scene.text(i));
        }
    });
    found
}

fn role(scene: &dyn SceneRead, n: Idx) -> u32 {
    roles::engine_role(scene.kind(n), scene.a11y(n).role)
}

/// Whether the window has any top-level menu (cheap: the window's direct children only).
pub fn has_menus(scene: &dyn SceneRead) -> bool {
    if !scene.is_live(0) {
        return false;
    }
    let mut c = scene.first_child(0);
    while c != NIL {
        if role(scene, c) == wk::MENU {
            return true;
        }
        c = scene.next_sibling(c);
    }
    false
}

/// The window's menus. Menu ids count from 1; an item's key is its node id in decimal. A menu
/// with no title or no item is left out, as is anything past the bounds of a Kit window's menus.
pub fn collect(scene: &dyn SceneRead) -> Vec<Menu> {
    let mut out: Vec<Menu> = Vec::new();
    if !scene.is_live(0) {
        return out;
    }
    let mut c = scene.first_child(0);
    while c != NIL && out.len() < MAX_MENUS {
        if role(scene, c) == wk::MENU {
            let title = name(scene, c);
            let entries = entries(scene, c);
            if !title.is_empty() && !entries.is_empty() {
                let id = u32::try_from(out.len() + 1).unwrap_or(u32::MAX);
                out.push(Menu { id, title, entries });
            }
        }
        c = scene.next_sibling(c);
    }
    out
}

fn entries(scene: &dyn SceneRead, menu: Idx) -> Vec<MenuEntry> {
    let mut out = Vec::new();
    let mut c = scene.first_child(menu);
    while c != NIL && out.len() < MAX_MENU_ENTRIES {
        match role(scene, c) {
            wk::SEPARATOR => out.push(MenuEntry::Separator),
            wk::MENUITEM => {
                let label = name(scene, c);
                if !label.is_empty() {
                    out.push(MenuEntry::Item {
                        key: scene.id(c).0.to_string(),
                        label,
                        shortcut: String::new(),
                        enabled: !crate::window::route::disabled(scene, c),
                        checked: scene.state(c) & state::CHECKED != 0,
                    });
                }
            }
            _ => {}
        }
        c = scene.next_sibling(c);
    }
    out
}

/// The node id an activation names, when it is an item of `menus`' menu `menu`.
pub fn target(menus: &[Menu], menu: u32, key: &str) -> Option<u32> {
    let m = menus.iter().find(|m| m.id == menu)?;
    m.entries
        .iter()
        .any(|e| matches!(e, MenuEntry::Item { key: k, enabled: true, .. } if k == key))
        .then(|| key.parse().ok())
        .flatten()
}

// ---- the bus side ------------------------------------------------------------------------

/// What the service answers from: every window's last menus and which one is in front.
#[derive(Default)]
struct Book {
    /// `(slot, menus)` per window that has any.
    windows: Vec<(u32, Vec<Menu>)>,
    /// The slot whose menus `Get` answers.
    front: u32,
    /// What the last published state encoded to, to signal only a difference.
    sent: String,
}

impl Book {
    fn current(&self) -> (u32, &[Menu]) {
        match self.windows.iter().find(|w| w.0 == self.front) {
            Some((s, m)) => (*s, m),
            None => (0, &[]),
        }
    }

    fn json(&self) -> String {
        serde_json::to_string(self.current().1).unwrap_or_else(|_| "[]".into())
    }
}

struct Iface {
    book: Arc<Mutex<Book>>,
    hub: Arc<Hub>,
}

fn lock(b: &Mutex<Book>) -> MutexGuard<'_, Book> {
    b.lock().unwrap_or_else(|e| e.into_inner())
}

#[zbus::interface(name = "org.wolos.Menus")]
impl Iface {
    fn get(&self) -> String {
        lock(&self.book).json()
    }

    fn activate(&self, menu: u32, key: String) -> zbus::fdo::Result<()> {
        let (slot, node) = {
            let b = lock(&self.book);
            let (slot, menus) = b.current();
            (slot, target(menus, menu, &key))
        };
        let Some(node) = node else {
            return Err(zbus::fdo::Error::InvalidArgs(
                "That is not an enabled item of the menus.".into(),
            ));
        };
        self.hub.push_action(slot, node, Action::Default);
        Ok(())
    }

    #[zbus(signal)]
    async fn changed(e: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
}

/// The process's menu service: the book the windows write and the thread that owns the bus name.
pub struct Service {
    book: Arc<Mutex<Book>>,
    wake: Sender<()>,
}

/// Where the service connects: the session bus, or an address (tests).
pub enum Bus<'a> {
    Session,
    Address(&'a str),
}

impl Service {
    /// Start the service for `app_id` (its own id: the bus name; the object path follows). The
    /// connection is made on the service's thread; without a bus there is no name and `Get`
    /// answers nobody, which is how an app without a bus has always been.
    pub fn start(app_id: &str, hub: Arc<Hub>, bus: Bus<'_>) -> Option<Service> {
        let book = Arc::new(Mutex::new(Book::default()));
        let (wake, rx) = channel();
        let name = theme::base_app_id(app_id).to_owned();
        let path = theme::menu_object_path(app_id);
        let address = match bus {
            Bus::Session => None,
            Bus::Address(a) => Some(a.to_owned()),
        };
        let iface = Iface {
            book: book.clone(),
            hub,
        };
        std::thread::Builder::new()
            .name("wolkit-menus".into())
            .spawn(move || serve(iface, &name, &path, address.as_deref(), &rx))
            .ok()?;
        Some(Service { book, wake })
    }

    /// Window `slot` now has `menus`; `front` says it is the window in front. Signals `Changed`
    /// when what `Get` would answer is not what it last answered.
    pub fn publish(&self, slot: u32, front: bool, menus: Vec<Menu>) {
        let changed = {
            let mut b = lock(&self.book);
            let none = menus.is_empty();
            b.windows.retain(|w| w.0 != slot);
            if !menus.is_empty() {
                b.windows.push((slot, menus));
            }
            if front && !none {
                b.front = slot;
            }
            if !b.windows.iter().any(|w| w.0 == b.front) {
                b.front = b.windows.last().map_or(0, |w| w.0);
            }
            let json = b.json();
            if json == b.sent {
                false
            } else {
                b.sent = json;
                true
            }
        };
        if changed {
            let _ = self.wake.send(());
        }
    }

    /// The window is gone.
    pub fn remove(&self, slot: u32) {
        self.publish(slot, false, Vec::new());
    }

    /// `Get`'s answer now (tests).
    pub fn json(&self) -> String {
        lock(&self.book).json()
    }
}

static SERVICE: Mutex<Option<Arc<Service>>> = Mutex::new(None);

/// The process's service, started for `app_id` on the session bus by the first window with menus.
pub fn shared(app_id: &str, hub: Arc<Hub>) -> Option<Arc<Service>> {
    shared_on(app_id, hub, Bus::Session)
}

/// The same on a given bus (tests start it on a private one before any window publishes).
pub fn shared_on(app_id: &str, hub: Arc<Hub>, bus: Bus<'_>) -> Option<Arc<Service>> {
    let mut g = SERVICE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = Service::start(app_id, hub, bus).map(Arc::new);
    }
    g.clone()
}

/// The process's service if one was started.
pub fn current() -> Option<Arc<Service>> {
    SERVICE.lock().ok().and_then(|g| g.clone())
}

fn serve(iface: Iface, name: &str, path: &str, address: Option<&str>, rx: &Receiver<()>) {
    use zbus::blocking::connection::Builder;
    let built = match address {
        Some(a) => Builder::address(a),
        None => Builder::session(),
    }
    .and_then(|b| b.name(name.to_owned()))
    .and_then(|b| b.serve_at(path.to_owned(), iface))
    .and_then(|b| b.build());
    let Ok(conn) = built else {
        // No bus, an id that is not a bus name, or the name is taken: no menus in the bar.
        return;
    };
    let Ok(e) = zbus::object_server::SignalEmitter::new(conn.inner(), path.to_owned()) else {
        return;
    };
    // The bar may have asked before the name existed: say so once.
    loop {
        let _ = zbus::block_on(Iface::changed(&e));
        // Wait for a difference; a burst of them is one signal.
        if rx.recv().is_err() {
            return;
        }
        while rx.try_recv().is_ok() {}
    }
}
