// SPDX-License-Identifier: BSD-2-Clause
//! Export over the desktop accessibility bus (plan section 2.7), only while an assistive
//! technology is attached: a status thread asks the session bus whether accessibility is on
//! (`org.a11y.Status`: `IsEnabled` or `ScreenReaderEnabled`) and follows its changes; when it is,
//! it connects to the accessibility bus (`org.a11y.Bus.GetAddress`), embeds the app's root with
//! the registry and a server thread answers the standard interfaces from the hub's snapshots:
//! Accessible, Component (bounds, hit test, focus), Action (the default action), Text (content,
//! caret, selection of a text field), Value (sliders) and Application. When it is turned off the
//! connection is dropped and nothing more is built.
//!
//! The engine's thread never waits here: actions are queued on the hub, change signals
//! (state, name, children, text, caret) are emitted from the engine thread on the connection,
//! which only queues them. Coordinates are window coordinates (the compositor does not tell a
//! client where its window is).

use super::hub::{Action, Exported, Hub};
use super::roles::{self, role, st};
use super::tree::{A11yNode, Tree};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::{Header, Message, Type};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

pub const ROOT: &str = "/org/a11y/atspi/accessible/root";
const PREFIX: &str = "/org/a11y/atspi/accessible/";
const NULL: &str = "/org/a11y/atspi/null";
const TOOLKIT: &str = "libWolkit";
/// D-Bus calls to the session bus give up after this.
const TIMEOUT: Duration = Duration::from_secs(2);

type ObjRef = (String, OwnedObjectPath);
/// Sets a property from a `Properties.Set` call; true when it did.
type Setter<'a> = Option<&'a dyn Fn(&str, &Value) -> bool>;

fn path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p)
        .unwrap_or_else(|_| OwnedObjectPath::from(ObjectPath::from_static_str_unchecked(NULL)))
}

pub fn node_path(slot: u32, id: u32) -> String {
    format!("{PREFIX}w{slot}n{id}")
}

/// What an object path names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Obj {
    Root,
    Node(u32, u32),
}

pub fn parse(p: &str) -> Option<Obj> {
    if p == ROOT {
        return Some(Obj::Root);
    }
    let rest = p.strip_prefix(PREFIX)?.strip_prefix('w')?;
    let (slot, id) = rest.split_once('n')?;
    Some(Obj::Node(slot.parse().ok()?, id.parse().ok()?))
}

/// Start watching whether an assistive technology listens (once per process).
pub fn start(hub: &Arc<Hub>) {
    let h = hub.clone();
    let spawned = std::thread::Builder::new()
        .name("wolkit-a11y-status".into())
        .spawn(move || status_loop(&h));
    // Without the thread nothing is exported, which is the state with no assistive technology.
    let _ = spawned;
}

fn status_bool(session: &Connection, prop: &str) -> bool {
    let reply = session.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &("org.a11y.Status", prop),
    );
    reply
        .ok()
        .and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedValue>().ok())
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

fn status_loop(hub: &Arc<Hub>) {
    let Ok(session) = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.method_timeout(TIMEOUT).build())
    else {
        return;
    };
    let rule = "type='signal',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged',path='/org/a11y/bus'";
    let mut changes = MessageIterator::for_match_rule(rule, &session, Some(16)).ok();
    loop {
        let on = status_bool(&session, "IsEnabled") || status_bool(&session, "ScreenReaderEnabled");
        if on && !hub.active() {
            let addr = session
                .call_method(
                    Some("org.a11y.Bus"),
                    "/org/a11y/bus",
                    Some("org.a11y.Bus"),
                    "GetAddress",
                    &(),
                )
                .ok()
                .and_then(|m| m.body().deserialize::<String>().ok());
            if let Some(a) = addr {
                // A bus that cannot be reached leaves export off until the next change.
                let _ = serve_on(hub, &a, true);
            }
        } else if !on && hub.active() {
            hub.set_connection(None);
        }
        // Wait for the next change of the status (or stop when the session bus goes away).
        match changes.as_mut().map(Iterator::next) {
            Some(Some(_)) => {}
            _ => return,
        }
    }
}

/// Connect to the accessibility bus at `address`, answer queries there and, with `embed`,
/// register the root with the registry. Public for tests (a private bus, no registry).
pub fn serve_on(hub: &Arc<Hub>, address: &str, embed: bool) -> zbus::Result<()> {
    let conn = zbus::blocking::connection::Builder::address(address)?
        .method_timeout(TIMEOUT)
        .build()?;
    let iter = MessageIterator::from(&conn);
    let (h, c) = (hub.clone(), conn.clone());
    std::thread::Builder::new()
        .name("wolkit-a11y".into())
        .spawn(move || {
            for m in iter {
                let Ok(m) = m else { break };
                serve(&h, &c, &m);
            }
        })
        .map_err(|e| zbus::Error::InputOutput(Arc::new(e)))?;
    hub.set_connection(Some(conn.clone()));
    if embed {
        let me = conn
            .unique_name()
            .map(|n| n.to_string())
            .unwrap_or_default();
        let parent = conn
            .call_method(
                Some("org.a11y.atspi.Registry"),
                ROOT,
                Some("org.a11y.atspi.Socket"),
                "Embed",
                &((me, path(ROOT)),),
            )
            .ok()
            .and_then(|m| m.body().deserialize::<ObjRef>().ok());
        if let Some((name, p)) = parent {
            hub.lock().parent = Some((name, p.as_str().to_owned()));
        }
    }
    Ok(())
}

fn me(conn: &Connection) -> String {
    conn.unique_name()
        .map(|n| n.to_string())
        .unwrap_or_default()
}

fn null() -> ObjRef {
    (String::new(), path(NULL))
}

/// A node, its window's slot and what is needed to answer about it.
struct Found {
    slot: u32,
    node: A11yNode,
    parent: ObjRef,
    children: Vec<ObjRef>,
    index_in_parent: i32,
    /// Bounds of the parent (for parent-relative extents).
    parent_bounds: crate::scene::Rect,
}

fn found(w: &Exported, conn_name: &str, id: u32) -> Option<Found> {
    let i = *w.by_id.get(&id)?;
    let n = w.tree.nodes.get(i)?.clone();
    let r = |j: usize| {
        (
            conn_name.to_owned(),
            path(&node_path(w.slot, w.tree.nodes[j].id)),
        )
    };
    let (parent, index_in_parent, parent_bounds) = if n.parent == usize::MAX {
        let at = 0;
        (
            (conn_name.to_owned(), path(ROOT)),
            at,
            crate::scene::Rect::default(),
        )
    } else {
        let p = &w.tree.nodes[n.parent];
        let at = p.children.iter().position(|c| *c == i).unwrap_or(0);
        (r(n.parent), i32::try_from(at).unwrap_or(0), p.bounds)
    };
    let children = n.children.iter().map(|c| r(*c)).collect();
    Some(Found {
        slot: w.slot,
        node: n,
        parent,
        children,
        index_in_parent,
        parent_bounds,
    })
}

fn reply<B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType>(
    c: &Connection,
    h: &Header<'_>,
    b: &B,
) {
    // A caller that went away does not need the answer.
    let _ = c.reply(h, b);
}

fn unknown(c: &Connection, h: &Header<'_>) {
    let m = h.member().map(|m| m.to_string()).unwrap_or_default();
    let _ = c.reply_dbus_error(
        h,
        zbus::fdo::Error::UnknownMethod(format!("{m} is not answered here.")),
    );
}

fn serve(hub: &Hub, conn: &Connection, msg: &Message) {
    let h = msg.header();
    if h.message_type() != Type::MethodCall {
        return;
    }
    let (Some(p), Some(member)) = (h.path(), h.member()) else {
        return;
    };
    let iface = h.interface().map(|i| i.to_string()).unwrap_or_default();
    let member = member.to_string();
    let Some(obj) = parse(p.as_str()) else {
        let _ = conn.reply_dbus_error(
            &h,
            zbus::fdo::Error::UnknownObject(format!("{p} is not an object here.")),
        );
        return;
    };
    let name = me(conn);
    match obj {
        Obj::Root => root(hub, conn, &h, msg, &iface, &member, &name),
        Obj::Node(slot, id) => {
            let f = {
                let s = hub.lock();
                s.windows
                    .iter()
                    .find(|w| w.slot == slot)
                    .and_then(|w| found(w, &name, id))
            };
            match f {
                Some(f) => node(hub, conn, &h, msg, &iface, &member, &f),
                None => {
                    let _ = conn.reply_dbus_error(
                        &h,
                        zbus::fdo::Error::UnknownObject(format!("{p} is gone.")),
                    );
                }
            }
        }
    }
}

fn root(
    hub: &Hub,
    c: &Connection,
    h: &Header<'_>,
    msg: &Message,
    iface: &str,
    member: &str,
    name: &str,
) {
    let (frames, title, parent, app_id, number) = {
        let s = hub.lock();
        let frames: Vec<ObjRef> = s
            .windows
            .iter()
            .filter(|w| !w.tree.nodes.is_empty())
            .map(|w| (name.to_owned(), path(&node_path(w.slot, 0))))
            .collect();
        let title = s
            .windows
            .iter()
            .find_map(|w| w.tree.nodes.first().map(|n| n.name.clone()));
        let parent = s.parent.clone().map_or_else(null, |(n, p)| (n, path(&p)));
        (frames, title, parent, s.app_id.clone(), s.app_number)
    };
    let app_name = title.filter(|t| !t.is_empty()).unwrap_or(app_id);
    match (iface, member) {
        ("org.freedesktop.DBus.Properties", "Get" | "GetAll" | "Set") => {
            let mut props: HashMap<&str, Value> = HashMap::new();
            props.insert("Name", Value::from(app_name));
            props.insert("Description", Value::from(""));
            props.insert("Parent", Value::from(parent));
            props.insert(
                "ChildCount",
                Value::from(i32::try_from(frames.len()).unwrap_or(0)),
            );
            props.insert("Locale", Value::from(""));
            props.insert("AccessibleId", Value::from(""));
            props.insert("HelpText", Value::from(""));
            props.insert("ToolkitName", Value::from(TOOLKIT));
            props.insert("Version", Value::from("1.0.0"));
            props.insert("AtspiVersion", Value::from("2.1"));
            props.insert("Id", Value::from(number));
            properties(hub, c, h, msg, member, props, None);
        }
        (_, "GetChildren") => reply(c, h, &frames),
        (_, "GetChildAtIndex") => {
            let i = msg.body().deserialize::<i32>().unwrap_or(-1);
            let r = usize::try_from(i)
                .ok()
                .and_then(|i| frames.get(i).cloned())
                .unwrap_or_else(null);
            reply(c, h, &r);
        }
        (_, "GetIndexInParent") => reply(c, h, &-1i32),
        (_, "GetRelationSet") => reply(c, h, &Vec::<(u32, Vec<ObjRef>)>::new()),
        (_, "GetRole") => reply(c, h, &role::APPLICATION),
        (_, "GetRoleName" | "GetLocalizedRoleName") => {
            reply(c, h, &roles::role_name(role::APPLICATION))
        }
        (_, "GetState") => reply(c, h, &vec![0u32, 0u32]),
        (_, "GetAttributes") => reply(
            c,
            h,
            &HashMap::<String, String>::from([("toolkit".into(), TOOLKIT.into())]),
        ),
        (_, "GetApplication") => reply(c, h, &(name.to_owned(), path(ROOT))),
        (_, "GetInterfaces") => reply(
            c,
            h,
            &vec!["org.a11y.atspi.Accessible", "org.a11y.atspi.Application"],
        ),
        (_, "GetLocale") => reply(c, h, &""),
        (_, "GetApplicationBusAddress") => reply(c, h, &""),
        _ => unknown(c, h),
    }
}

/// `Properties.Get`, `GetAll` and `Set` over a table of values; `Set` goes to `set`.
fn properties(
    hub: &Hub,
    c: &Connection,
    h: &Header<'_>,
    msg: &Message,
    member: &str,
    props: HashMap<&str, Value>,
    set: Setter<'_>,
) {
    match member {
        "Get" => {
            let Ok((_iface, prop)) = msg.body().deserialize::<(String, String)>() else {
                return unknown(c, h);
            };
            match props.get(prop.as_str()) {
                Some(v) => reply(c, h, v),
                None => {
                    let _ = c.reply_dbus_error(
                        h,
                        zbus::fdo::Error::UnknownProperty(format!(
                            "{prop} is not a property here."
                        )),
                    );
                }
            }
        }
        "GetAll" => reply(c, h, &props),
        "Set" => {
            let ok = msg
                .body()
                .deserialize::<(String, String, zbus::zvariant::OwnedValue)>()
                .ok()
                .is_some_and(|(_, prop, v)| {
                    if prop == "Id" {
                        if let Ok(n) = i32::try_from(&*v) {
                            hub.lock().app_number = n;
                        }
                        return true;
                    }
                    set.is_some_and(|f| f(&prop, &v))
                });
            if ok {
                reply(c, h, &());
            } else {
                let _ = c.reply_dbus_error(
                    h,
                    zbus::fdo::Error::PropertyReadOnly("That property cannot be set.".into()),
                );
            }
        }
        _ => unknown(c, h),
    }
}

fn r32(v: f32) -> i32 {
    crate::num::round_i32(v)
}

/// Extents in the asked coordinates: screen and window are the same here, parent is relative.
fn extents(f: &Found, coord: u32) -> (i32, i32, i32, i32) {
    let b = f.node.bounds;
    let (ox, oy) = if coord == 2 {
        (f.parent_bounds.x, f.parent_bounds.y)
    } else {
        (0.0, 0.0)
    };
    (r32(b.x - ox), r32(b.y - oy), r32(b.w), r32(b.h))
}

fn interfaces(n: &A11yNode) -> Vec<&'static str> {
    let mut v = vec!["org.a11y.atspi.Accessible", "org.a11y.atspi.Component"];
    if n.action.is_some() {
        v.push("org.a11y.atspi.Action");
    }
    if n.text.is_some() {
        v.push("org.a11y.atspi.Text");
    }
    if n.value.is_some() {
        v.push("org.a11y.atspi.Value");
    }
    v
}

fn states(n: &A11yNode) -> Vec<u32> {
    let lo = u32::try_from(n.states & 0xffff_ffff).unwrap_or(0);
    let hi = u32::try_from(n.states >> 32).unwrap_or(0);
    vec![lo, hi]
}

fn node(
    hub: &Hub,
    c: &Connection,
    h: &Header<'_>,
    msg: &Message,
    iface: &str,
    member: &str,
    f: &Found,
) {
    let n = &f.node;
    let act = |a: Action| hub.push_action(f.slot, n.id, a);
    match iface {
        "org.freedesktop.DBus.Properties" => {
            let mut p: HashMap<&str, Value> = HashMap::new();
            p.insert("Name", Value::from(n.name.clone()));
            p.insert("Description", Value::from(n.description.clone()));
            p.insert("Parent", Value::from(f.parent.clone()));
            p.insert(
                "ChildCount",
                Value::from(i32::try_from(f.children.len()).unwrap_or(0)),
            );
            p.insert("Locale", Value::from(""));
            p.insert("AccessibleId", Value::from(format!("w{}n{}", f.slot, n.id)));
            p.insert("HelpText", Value::from(""));
            p.insert("NActions", Value::from(i32::from(n.action.is_some())));
            if let Some(t) = &n.text {
                p.insert(
                    "CharacterCount",
                    Value::from(i32::try_from(t.text.chars().count()).unwrap_or(0)),
                );
                p.insert(
                    "CaretOffset",
                    Value::from(i32::try_from(t.caret).unwrap_or(0)),
                );
            }
            if let Some(v) = n.value {
                p.insert("MinimumValue", Value::from(f64::from(v.min)));
                p.insert("MaximumValue", Value::from(f64::from(v.max)));
                p.insert("MinimumIncrement", Value::from(f64::from(v.step)));
                p.insert("CurrentValue", Value::from(f64::from(v.current)));
                p.insert("Text", Value::from(""));
            }
            let set = |prop: &str, v: &Value| -> bool {
                match (prop, f64::try_from(v)) {
                    ("CurrentValue", Ok(x)) if n.value.is_some() => {
                        act(Action::Value(x));
                        true
                    }
                    _ => false,
                }
            };
            properties(hub, c, h, msg, member, p, Some(&set));
        }
        "org.a11y.atspi.Accessible" => accessible(c, h, msg, member, f),
        "org.a11y.atspi.Component" => component(hub, c, h, msg, member, f),
        "org.a11y.atspi.Action" => match member {
            "GetName" | "GetLocalizedName" => reply(c, h, &n.action.unwrap_or("")),
            "GetDescription" | "GetKeyBinding" => reply(c, h, &""),
            "GetActions" => {
                let v: Vec<(String, String, String)> = n
                    .action
                    .iter()
                    .map(|a| ((*a).to_owned(), String::new(), String::new()))
                    .collect();
                reply(c, h, &v);
            }
            "DoAction" => {
                let ok =
                    n.action.is_some() && msg.body().deserialize::<i32>().is_ok_and(|i| i == 0);
                if ok {
                    act(Action::Default);
                }
                reply(c, h, &ok);
            }
            _ => unknown(c, h),
        },
        "org.a11y.atspi.Text" if n.text.is_some() => text(hub, c, h, msg, member, f),
        "org.a11y.atspi.Value" => unknown(c, h),
        _ => unknown(c, h),
    }
}

fn accessible(c: &Connection, h: &Header<'_>, msg: &Message, member: &str, f: &Found) {
    let n = &f.node;
    match member {
        "GetChildren" => reply(c, h, &f.children),
        "GetChildAtIndex" => {
            let i = msg.body().deserialize::<i32>().unwrap_or(-1);
            let r = usize::try_from(i)
                .ok()
                .and_then(|i| f.children.get(i).cloned())
                .unwrap_or_else(null);
            reply(c, h, &r);
        }
        "GetIndexInParent" => reply(c, h, &f.index_in_parent),
        "GetRelationSet" => reply(c, h, &Vec::<(u32, Vec<ObjRef>)>::new()),
        "GetRole" => reply(c, h, &n.wire_role),
        "GetRoleName" | "GetLocalizedRoleName" => reply(c, h, &roles::role_name(n.wire_role)),
        "GetState" => reply(c, h, &states(n)),
        "GetAttributes" => reply(c, h, &HashMap::<String, String>::new()),
        "GetApplication" => reply(c, h, &(f.parent.0.clone(), path(ROOT))),
        "GetInterfaces" => reply(c, h, &interfaces(n)),
        _ => unknown(c, h),
    }
}

fn component(hub: &Hub, c: &Connection, h: &Header<'_>, msg: &Message, member: &str, f: &Found) {
    let n = &f.node;
    match member {
        "GetExtents" => {
            let coord = msg.body().deserialize::<u32>().unwrap_or(1);
            reply(c, h, &extents(f, coord));
        }
        "GetPosition" => {
            let coord = msg.body().deserialize::<u32>().unwrap_or(1);
            let e = extents(f, coord);
            reply(c, h, &(e.0, e.1));
        }
        "GetSize" => reply(c, h, &(r32(n.bounds.w), r32(n.bounds.h))),
        "Contains" => {
            let (x, y, _) = msg
                .body()
                .deserialize::<(i32, i32, u32)>()
                .unwrap_or((-1, -1, 0));
            reply(c, h, &n.bounds.contains(x as f32, y as f32));
        }
        "GetAccessibleAtPoint" => {
            let (x, y, _) = msg
                .body()
                .deserialize::<(i32, i32, u32)>()
                .unwrap_or((-1, -1, 0));
            let s = hub.lock();
            let hit = s.windows.iter().find(|w| w.slot == f.slot).and_then(|w| {
                let start = *w.by_id.get(&n.id)?;
                let mut at = start;
                'down: loop {
                    for ch in w.tree.nodes[at].children.iter().rev() {
                        if w.tree.nodes[*ch].bounds.contains(x as f32, y as f32) {
                            at = *ch;
                            continue 'down;
                        }
                    }
                    break;
                }
                (w.tree.nodes[at].bounds.contains(x as f32, y as f32)).then(|| {
                    (
                        f.parent.0.clone(),
                        path(&node_path(w.slot, w.tree.nodes[at].id)),
                    )
                })
            });
            drop(s);
            reply(c, h, &hit.unwrap_or_else(null));
        }
        "GetLayer" => reply(
            c,
            h,
            &if n.wire_role == role::FRAME {
                7u32
            } else {
                3u32
            },
        ),
        "GetMDIZOrder" => reply(c, h, &0i16),
        "GetAlpha" => reply(c, h, &1.0f64),
        "GrabFocus" => {
            let ok = n.has(st::FOCUSABLE);
            if ok {
                hub.push_action(f.slot, n.id, Action::Focus);
            }
            reply(c, h, &ok);
        }
        "ScrollTo" | "ScrollToPoint" => {
            hub.push_action(f.slot, n.id, Action::ScrollTo);
            reply(c, h, &true);
        }
        _ => unknown(c, h),
    }
}

fn chars(s: &str, a: usize, b: usize) -> String {
    s.chars().skip(a).take(b.saturating_sub(a)).collect()
}

/// The word around character offset `at`: (text, start, end) in characters.
fn word_at(s: &str, at: usize) -> (String, usize, usize) {
    let v: Vec<char> = s.chars().collect();
    let at = at.min(v.len());
    let is_w = |c: char| c.is_alphanumeric() || c == '_';
    let mut a = at;
    while a > 0 && is_w(v[a - 1]) {
        a -= 1;
    }
    let mut b = at;
    while b < v.len() && is_w(v[b]) {
        b += 1;
    }
    (v[a..b].iter().collect(), a, b)
}

fn text(hub: &Hub, c: &Connection, h: &Header<'_>, msg: &Message, member: &str, f: &Found) {
    let Some(t) = &f.node.text else {
        return unknown(c, h);
    };
    let count = t.text.chars().count();
    let off = |v: i32| usize::try_from(v).map_or(count, |v| v.min(count));
    let i = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
    match member {
        "GetText" => {
            let (a, b) = msg.body().deserialize::<(i32, i32)>().unwrap_or((0, -1));
            reply(c, h, &chars(&t.text, off(a), off(b)));
        }
        "SetCaretOffset" => {
            let o = msg.body().deserialize::<i32>().unwrap_or(0);
            hub.push_action(f.slot, f.node.id, Action::Caret(off(o)));
            reply(c, h, &true);
        }
        "GetNSelections" => reply(c, h, &i32::from(t.selection.0 != t.selection.1)),
        "GetSelection" => reply(c, h, &(i(t.selection.0), i(t.selection.1))),
        "AddSelection" | "SetSelection" => {
            let (a, b) = if member == "AddSelection" {
                msg.body().deserialize::<(i32, i32)>().unwrap_or((0, 0))
            } else {
                msg.body()
                    .deserialize::<(i32, i32, i32)>()
                    .map(|x| (x.1, x.2))
                    .unwrap_or((0, 0))
            };
            hub.push_action(f.slot, f.node.id, Action::Select(off(a), off(b)));
            reply(c, h, &true);
        }
        "RemoveSelection" => {
            hub.push_action(f.slot, f.node.id, Action::Caret(t.caret));
            reply(c, h, &true);
        }
        "GetCharacterAtOffset" => {
            let o = off(msg.body().deserialize::<i32>().unwrap_or(0));
            let ch = t.text.chars().nth(o).map_or(0, u32::from);
            reply(c, h, &i32::try_from(ch).unwrap_or(0));
        }
        "GetStringAtOffset" | "GetTextAtOffset" | "GetTextBeforeOffset" | "GetTextAfterOffset" => {
            let (o, g) = msg.body().deserialize::<(i32, u32)>().unwrap_or((0, 0));
            let o = off(o);
            // Granularity 0 is a character; 1 (and the word boundaries) a word; the rest the line,
            // which is the whole of a one-line field.
            let (s, a, b) = match (member, g) {
                ("GetStringAtOffset", 0) => (chars(&t.text, o, o + 1), o, (o + 1).min(count)),
                ("GetStringAtOffset", 1) | (_, 1 | 2) => word_at(&t.text, o),
                (_, 0) => (chars(&t.text, o, o + 1), o, (o + 1).min(count)),
                _ => (t.text.clone(), 0, count),
            };
            reply(c, h, &(s, i(a), i(b)));
        }
        "GetCharacterExtents" | "GetRangeExtents" => reply(c, h, &extents(f, 1)),
        "GetOffsetAtPoint" => reply(c, h, &i(t.caret)),
        "GetDefaultAttributes" | "GetDefaultAttributeSet" => {
            reply(c, h, &HashMap::<String, String>::new())
        }
        "GetAttributeRun" | "GetAttributes" => {
            reply(c, h, &(HashMap::<String, String>::new(), 0i32, i(count)))
        }
        _ => unknown(c, h),
    }
}

// ---- change signals (engine thread) ---------------------------------------------------------

/// `d` is the event's detail string and its two numbers.
fn signal(
    conn: &Connection,
    p: &str,
    iface: &str,
    member: &str,
    d: (&str, i32, i32),
    any: Value<'_>,
) {
    let (detail, d1, d2) = d;
    let props: HashMap<&str, Value> = HashMap::new();
    // A listener that went away misses one event; the next query reads the current tree.
    let _ = conn.emit_signal(
        Option::<&str>::None,
        p,
        iface,
        member,
        &(detail, d1, d2, any, props),
    );
}

const EVENT: &str = "org.a11y.atspi.Event.Object";

/// Tell listeners what changed between `old` and `new` (the trees of window `slot`).
pub fn changes(hub: &Hub, slot: u32, old: &Tree, new: &Tree) {
    let Some(conn) = hub.lock().conn.clone() else {
        return;
    };
    let name = me(&conn);
    let before: HashMap<u32, &A11yNode> = old.nodes.iter().map(|n| (n.id, n)).collect();
    let watched = [
        (st::FOCUSED, "focused"),
        (st::CHECKED, "checked"),
        (st::PRESSED, "pressed"),
        (st::SELECTED, "selected"),
        (st::EXPANDED, "expanded"),
        (st::SENSITIVE, "sensitive"),
        (st::ENABLED, "enabled"),
        (st::BUSY, "busy"),
    ];
    for n in &new.nodes {
        let p = node_path(slot, n.id);
        let Some(o) = before.get(&n.id) else {
            continue;
        };
        if n.id == 0 && o.has(st::ACTIVE) != n.has(st::ACTIVE) {
            let m = if n.has(st::ACTIVE) {
                "Activate"
            } else {
                "Deactivate"
            };
            signal(
                &conn,
                &p,
                "org.a11y.atspi.Event.Window",
                m,
                ("", 0, 0),
                Value::from(""),
            );
        }
        for (bit, label) in watched {
            if o.has(bit) != n.has(bit) {
                signal(
                    &conn,
                    &p,
                    EVENT,
                    "StateChanged",
                    (label, i32::from(n.has(bit)), 0),
                    Value::from(0i32),
                );
            }
        }
        if o.name != n.name {
            signal(
                &conn,
                &p,
                EVENT,
                "PropertyChange",
                ("accessible-name", 0, 0),
                Value::from(n.name.clone()),
            );
        }
        if o.description != n.description {
            let v = Value::from(n.description.clone());
            signal(
                &conn,
                &p,
                EVENT,
                "PropertyChange",
                ("accessible-description", 0, 0),
                v,
            );
        }
        if let (Some(a), Some(b)) = (&o.text, &n.text) {
            if a.text != b.text {
                let (la, lb) = (a.text.chars().count(), b.text.chars().count());
                let i = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
                signal(
                    &conn,
                    &p,
                    EVENT,
                    "TextChanged",
                    ("delete", 0, i(la)),
                    Value::from(a.text.clone()),
                );
                signal(
                    &conn,
                    &p,
                    EVENT,
                    "TextChanged",
                    ("insert", 0, i(lb)),
                    Value::from(b.text.clone()),
                );
            }
            if a.caret != b.caret || a.text != b.text {
                signal(
                    &conn,
                    &p,
                    EVENT,
                    "TextCaretMoved",
                    ("", i32::try_from(b.caret).unwrap_or(0), 0),
                    Value::from(0i32),
                );
            }
        }
        if o.value.map(|v| v.current) != n.value.map(|v| v.current) {
            signal(
                &conn,
                &p,
                EVENT,
                "PropertyChange",
                ("accessible-value", 0, 0),
                Value::from(0i32),
            );
        }
        // Children: ids in order; a change is reported per child added or removed.
        let ids = |t: &Tree, x: &A11yNode| -> Vec<u32> {
            x.children.iter().map(|c| t.nodes[*c].id).collect()
        };
        let (ca, cb) = (ids(old, o), ids(new, n));
        if ca != cb {
            for (k, id) in ca.iter().enumerate().filter(|(_, id)| !cb.contains(id)) {
                let r: ObjRef = (name.clone(), path(&node_path(slot, *id)));
                signal(
                    &conn,
                    &p,
                    EVENT,
                    "ChildrenChanged",
                    ("remove", i32::try_from(k).unwrap_or(0), 0),
                    Value::from(r),
                );
            }
            for (k, id) in cb.iter().enumerate().filter(|(_, id)| !ca.contains(id)) {
                let r: ObjRef = (name.clone(), path(&node_path(slot, *id)));
                signal(
                    &conn,
                    &p,
                    EVENT,
                    "ChildrenChanged",
                    ("add", i32::try_from(k).unwrap_or(0), 0),
                    Value::from(r),
                );
            }
        }
    }
}
