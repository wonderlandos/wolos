// SPDX-License-Identifier: BSD-2-Clause
//! The accessibility export on the wire (`wolkit::a11y::bridge`, WK-W4): a private message bus
//! (the system's bus daemon, started for the test with its own socket) stands in for the
//! accessibility bus, the hub serves the Todos window on it, and a client walks it the way a
//! screen reader does (children, role, name, states, extents, actions, text, value). The walk is
//! printed in the dump format and must equal the golden dump of the same window
//! (`tests/a11y_golden/todos.txt`): what the bus shows is what the tree holds. Actions and change
//! signals are checked too. Skipped, with a note, where no bus daemon is installed.

#[path = "a11y_golden/todos.rs"]
mod todos;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use todos::{Win, place, todos};
use wolkit::a11y::bridge::{self, ROOT};
use wolkit::a11y::hub::{Action, Hub};
use wolkit::a11y::roles::st;
use wolkit::scene::Kind;
use wolkit::scene::prop::id;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

type ObjRef = (String, OwnedObjectPath);

struct Bus {
    child: Child,
    address: String,
}

// Stops the private bus daemon.
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bus(tag: &str) -> Option<Bus> {
    let dir = std::env::temp_dir().join(format!("wolkit-atspi-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let sock = dir.join("bus");
    let _ = std::fs::remove_file(&sock);
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
        .arg(format!("--address=unix:path={}", sock.display()))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let out = child.stdout.take()?;
    let mut line = String::new();
    BufReader::new(out.take(4096)).read_line(&mut line).ok()?;
    let address = line.trim().to_owned();
    (!address.is_empty()).then_some(Bus { child, address })
}

struct Rig {
    hub: Arc<Hub>,
    slot: u32,
    client: Connection,
    server: String,
    _bus: Bus,
}

fn rig(tag: &str, win: &Win) -> Option<Rig> {
    let Some(b) = bus(tag) else {
        println!(
            "no message bus daemon here: the wire test of the accessibility export is skipped"
        );
        return None;
    };
    let hub = Arc::new(Hub::new("org.example.todos").unwrap());
    let slot = hub.register();
    bridge::serve_on(&hub, &b.address, false).unwrap();
    assert!(hub.active());
    hub.publish(slot, win.tree());
    let server = hub
        .lock()
        .conn
        .as_ref()
        .unwrap()
        .unique_name()
        .unwrap()
        .to_string();
    let client = zbus::blocking::connection::Builder::address(b.address.as_str())
        .unwrap()
        .method_timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Some(Rig {
        hub,
        slot,
        client,
        server,
        _bus: b,
    })
}

impl Rig {
    fn call<B, R>(&self, path: &str, iface: &str, member: &str, body: &B) -> R
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> zbus::export::serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        let m = self
            .client
            .call_method(Some(self.server.as_str()), path, Some(iface), member, body)
            .unwrap_or_else(|e| panic!("{iface}.{member} on {path}: {e}"));
        m.body().deserialize::<R>().unwrap()
    }

    fn prop(&self, path: &str, iface: &str, name: &str) -> OwnedValue {
        self.call(
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            &(iface, name),
        )
    }

    fn string(&self, path: &str, name: &str) -> String {
        String::try_from(self.prop(path, "org.a11y.atspi.Accessible", name)).unwrap()
    }
}

/// Walk from the root the way a screen reader does and print what the bus says, in the dump format.
fn walk(r: &Rig) -> String {
    let frames: Vec<ObjRef> = r.call(ROOT, "org.a11y.atspi.Accessible", "GetChildren", &());
    assert_eq!(frames.len(), 1);
    let nodes = std::cell::Cell::new(0usize);
    let mut body = String::new();
    let mut stack = vec![(frames[0].1.as_str().to_owned(), 0usize)];
    let acc = "org.a11y.atspi.Accessible";
    while let Some((p, depth)) = stack.pop() {
        nodes.set(nodes.get() + 1);
        let id = p.rsplit('n').next().unwrap().to_owned();
        let role: String = r.call(&p, acc, "GetRoleName", &());
        let name = r.string(&p, "Name");
        let state: Vec<u32> = r.call(&p, acc, "GetState", &());
        let bits = u64::from(state[0]) | (u64::from(state[1]) << 32);
        let names: Vec<&str> = st::NAMES
            .iter()
            .filter(|(b, _)| bits & (1 << b) != 0)
            .map(|(_, n)| *n)
            .collect();
        let (x, y, w, h): (i32, i32, i32, i32) =
            r.call(&p, "org.a11y.atspi.Component", "GetExtents", &1u32);
        let _ = write!(
            body,
            "{}{id} {} {name:?} [{}] {x},{y} {w}x{h}",
            "  ".repeat(depth),
            role.replace(' ', "-"),
            names.join(",")
        );
        let desc = r.string(&p, "Description");
        if !desc.is_empty() {
            let _ = write!(body, " description={desc:?}");
        }
        let ifaces: Vec<String> = r.call(&p, acc, "GetInterfaces", &());
        let has = |i: &str| ifaces.iter().any(|x| x == i);
        if has("org.a11y.atspi.Action") {
            let a: Vec<(String, String, String)> =
                r.call(&p, "org.a11y.atspi.Action", "GetActions", &());
            let _ = write!(body, " action={}", a[0].0);
        }
        if has("org.a11y.atspi.Text") {
            let t = "org.a11y.atspi.Text";
            let text: String = r.call(&p, t, "GetText", &(0i32, -1i32));
            let caret = i32::try_from(r.prop(&p, t, "CaretOffset")).unwrap();
            let (a, b): (i32, i32) = r.call(&p, t, "GetSelection", &0i32);
            let (a, b) = if (a, b) == (0, 0) && caret != 0 {
                (caret, caret)
            } else {
                (a, b)
            };
            let _ = write!(body, " text={text:?} caret={caret} selection={a}..{b}");
        }
        if has("org.a11y.atspi.Value") {
            let v = |n: &str| f64::try_from(r.prop(&p, "org.a11y.atspi.Value", n)).unwrap();
            let _ = write!(
                body,
                " value={} min={} max={} step={}",
                v("CurrentValue"),
                v("MinimumValue"),
                v("MaximumValue"),
                v("MinimumIncrement")
            );
        }
        body.push('\n');
        let kids: Vec<ObjRef> = r.call(&p, acc, "GetChildren", &());
        let count = i32::try_from(r.prop(&p, acc, "ChildCount")).unwrap();
        assert_eq!(usize::try_from(count).unwrap(), kids.len());
        for k in kids.iter().rev() {
            stack.push((k.1.as_str().to_owned(), depth + 1));
        }
    }
    format!("a11y window=1 nodes={}\n{body}", nodes.get())
}

#[test]
fn the_bus_shows_exactly_the_golden_tree() {
    let w = todos();
    let Some(r) = rig("golden", &w) else {
        return;
    };
    let want = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/a11y_golden/todos.txt"),
    )
    .unwrap();
    assert_eq!(walk(&r), want);
    // The root is the application, its child the window's frame.
    let role: u32 = r.call(ROOT, "org.a11y.atspi.Accessible", "GetRole", &());
    assert_eq!(role, wolkit::a11y::roles::role::APPLICATION);
    let tk = String::try_from(r.prop(ROOT, "org.a11y.atspi.Application", "ToolkitName")).unwrap();
    assert_eq!(tk, "libWolkit");
}

#[test]
fn actions_from_the_bus_are_queued_for_the_engine() {
    let w = todos();
    let Some(r) = rig("actions", &w) else {
        return;
    };
    let p = |id: u32| bridge::node_path(r.slot, id);
    let ok: bool = r.call(&p(4), "org.a11y.atspi.Action", "DoAction", &0i32);
    assert!(ok);
    let ok: bool = r.call(&p(4), "org.a11y.atspi.Component", "GrabFocus", &());
    assert!(ok);
    let ok: bool = r.call(&p(3), "org.a11y.atspi.Text", "SetCaretOffset", &2i32);
    assert!(ok);
    r.client
        .call_method(
            Some(r.server.as_str()),
            p(9).as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Set",
            &(
                "org.a11y.atspi.Value",
                "CurrentValue",
                zbus::zvariant::Value::from(3.0f64),
            ),
        )
        .unwrap();
    let got = r.hub.take_actions(r.slot);
    assert_eq!(
        got,
        vec![
            (4, Action::Default),
            (4, Action::Focus),
            (3, Action::Caret(2)),
            (9, Action::Value(3.0))
        ]
    );
    let ok: bool = r.call(&p(2), "org.a11y.atspi.Component", "GrabFocus", &());
    assert!(!ok, "a heading does not take focus");
    let at: ObjRef = r.call(
        &p(0),
        "org.a11y.atspi.Component",
        "GetAccessibleAtPoint",
        &(240i32, 50i32, 1u32),
    );
    assert_eq!(at.1.as_str(), p(4), "the point is on the Add button");
}

#[test]
fn focus_and_text_changes_are_signalled() {
    let mut w = todos();
    let Some(r) = rig("signals", &w) else {
        return;
    };
    let rule = "type='signal',interface='org.a11y.atspi.Event.Object'";
    let mut it =
        zbus::blocking::MessageIterator::for_match_rule(rule, &r.client, Some(64)).unwrap();
    let old = w.tree();
    w.apply(|p| {
        p.set_text(3, "milk and eggs");
        p.focus(4);
        place(p, 20, Kind::Button, 1, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(20, 0, "New", "");
        p.set_prop(20, id::TOOLTIP, &[0]);
    });
    let new = w.tree();
    r.hub.publish(r.slot, new.clone());
    bridge::changes(&r.hub, r.slot, &old, &new);
    let mut seen: HashMap<(String, String, String), i32> = HashMap::new();
    let end = Instant::now() + Duration::from_secs(5);
    let key = |a: &str, b: &str, c: &str| (a.to_owned(), b.to_owned(), c.to_owned());
    let wanted = [
        key("4", "StateChanged", "focused"),
        key("3", "StateChanged", "focused"),
        key("3", "TextChanged", "insert"),
        key("1", "ChildrenChanged", "add"),
    ];
    while Instant::now() < end && !wanted.iter().all(|k| seen.contains_key(k)) {
        let Some(Ok(m)) = it.next() else { break };
        let h = m.header();
        let member = h.member().map(|x| x.to_string()).unwrap_or_default();
        let path = h.path().map(|x| x.to_string()).unwrap_or_default();
        let (detail, d1, _d2, _any, _props): (
            String,
            i32,
            i32,
            OwnedValue,
            HashMap<String, OwnedValue>,
        ) = m.body().deserialize().unwrap();
        seen.insert(
            (
                path.rsplit('n').next().unwrap_or("").to_owned(),
                member,
                detail,
            ),
            d1,
        );
    }
    let get = |id: &str, m: &str, d: &str| {
        seen.get(&(id.to_owned(), m.to_owned(), d.to_owned()))
            .copied()
    };
    assert_eq!(get("4", "StateChanged", "focused"), Some(1), "{seen:?}");
    assert_eq!(get("3", "StateChanged", "focused"), Some(0));
    assert!(get("3", "TextChanged", "insert").is_some());
    assert!(get("1", "ChildrenChanged", "add").is_some());
    assert!(new.nodes.iter().any(|n| n.id == 20 && n.has(st::FOCUSABLE)));
}
