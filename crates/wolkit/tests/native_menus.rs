// SPDX-License-Identifier: BSD-2-Clause
//! A native window's menus for the menu bar (`wolkit::window::native::menus`, DT-3): which scene
//! nodes are the menus, that what is published passes the checks the shell makes on a Kit
//! window's menus, and, on a private message bus, that `Get`, `Activate` and `Changed` do what the
//! shell relies on (`Activate` becomes the same default action an assistive technology asks for,
//! and only an enabled item of the published menus is accepted). The wire part is skipped, with a
//! note, where no bus daemon is installed.

#[path = "a11y_golden/todos.rs"]
mod todos;

use shared::view::{Menu, MenuEntry};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use todos::{Win, place};
use wolkit::a11y::hub::{Action, Hub};
use wolkit::a11y::roles::wk;
use wolkit::scene::prop::id;
use wolkit::scene::{Kind, SceneRead, state};
use wolkit::window::native::menus::{self, Bus, Service};

/// File and Edit as top-level `MENU` nodes hidden inside the window (the app draws its own bar
/// elsewhere or none), a `MENU` nested in a panel that must not count, and one with no items.
fn app() -> Win {
    Win::new(|p| {
        place(p, 1, Kind::Box, 0, [0.0, 0.0, 320.0, 240.0]);
        // File: New, Open (disabled), a rule, Word wrap (checked), and a node that is no item.
        place(p, 2, Kind::Box, 0, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(2, wk::MENU, "File", "");
        p.set_prop(2, id::DISPLAY, &[1]);
        place(p, 3, Kind::Button, 2, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(3, wk::MENUITEM, "New", "");
        place(p, 4, Kind::Button, 2, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(4, wk::MENUITEM, "Open", "");
        p.set_state(4, state::DISABLED);
        place(p, 5, Kind::Divider, 2, [0.0, 0.0, 10.0, 1.0]);
        place(p, 6, Kind::Toggle, 2, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(6, wk::MENUITEM, "", "");
        p.set_text(6, "Word wrap");
        p.set_state(6, state::CHECKED);
        place(p, 7, Kind::Text, 2, [0.0, 0.0, 10.0, 10.0]);
        p.set_text(7, "not an item");
        // Edit: its title is its content.
        place(p, 8, Kind::Box, 0, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(8, wk::MENU, "", "");
        place(p, 9, Kind::Text, 8, [0.0, 0.0, 10.0, 10.0]);
        p.set_text(9, "Edit");
        place(p, 10, Kind::Button, 8, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(10, wk::MENUITEM, "Undo", "");
        // A menu with no items and one with no title are left out.
        place(p, 11, Kind::Box, 0, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(11, wk::MENU, "Empty", "");
        place(p, 12, Kind::Box, 0, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(12, wk::MENU, "", "");
        place(p, 13, Kind::Button, 12, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(13, wk::MENUITEM, "Orphan", "");
        // A MENU below the window's top level (a context menu in a panel) is not a window menu.
        place(p, 14, Kind::Box, 1, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(14, wk::MENU, "Context", "");
        place(p, 15, Kind::Button, 14, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(15, wk::MENUITEM, "Copy", "");
    })
}

fn item(key: &str, label: &str, enabled: bool, checked: bool) -> MenuEntry {
    MenuEntry::Item {
        key: key.into(),
        label: label.into(),
        shortcut: String::new(),
        enabled,
        checked,
    }
}

#[test]
fn the_top_level_menu_nodes_are_the_menus() {
    let w = app();
    assert!(menus::has_menus(&w.core.scene));
    let m = menus::collect(&w.core.scene);
    let key = |slot: u32| w.core.scene.id(slot).0.to_string();
    assert_eq!(
        m,
        vec![
            Menu {
                id: 1,
                title: "File".into(),
                entries: vec![
                    item(&key(3), "New", true, false),
                    item(&key(4), "Open", false, false),
                    MenuEntry::Separator,
                    item(&key(6), "Word wrap", true, true),
                ],
            },
            Menu {
                id: 2,
                title: "Edit".into(),
                entries: vec![item(&key(10), "Undo", true, false)],
            },
        ],
        "hidden menus count, an empty or untitled one and a nested one do not"
    );
    assert!(
        shared::view::validate_chrome(&m, None).is_ok(),
        "the shell accepts what is published"
    );
}

#[test]
fn a_window_without_menus_has_none() {
    let w = todos::todos();
    assert!(!w.tree().nodes.is_empty(), "the window has a tree");
    assert!(!menus::has_menus(&w.core.scene));
    assert!(menus::collect(&w.core.scene).is_empty());
}

#[test]
fn long_and_odd_labels_are_cut_to_what_the_shell_accepts() {
    let w = Win::new(|p| {
        place(p, 1, Kind::Box, 0, [0.0, 0.0, 10.0, 10.0]);
        p.set_a11y(1, wk::MENU, "Fi\nle", "");
        for i in 0..60u32 {
            place(p, 2 + i, Kind::Button, 1, [0.0, 0.0, 10.0, 10.0]);
            p.set_a11y(2 + i, wk::MENUITEM, &"\u{e9}".repeat(100), "");
        }
    });
    let m = menus::collect(&w.core.scene);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].title, "Fi le");
    assert_eq!(m[0].entries.len(), shared::view::MAX_MENU_ENTRIES);
    assert!(shared::view::validate_chrome(&m, None).is_ok());
}

#[test]
fn only_an_enabled_listed_item_can_be_activated() {
    let w = app();
    let m = menus::collect(&w.core.scene);
    let new = w.core.scene.id(3).0;
    let open = w.core.scene.id(4).0;
    assert_eq!(menus::target(&m, 1, &new.to_string()), Some(new));
    assert_eq!(menus::target(&m, 1, &open.to_string()), None, "disabled");
    assert_eq!(menus::target(&m, 2, &new.to_string()), None, "wrong menu");
    assert_eq!(menus::target(&m, 3, &new.to_string()), None, "no such menu");
    assert_eq!(menus::target(&m, 1, "999999"), None, "not listed");
    assert_eq!(menus::target(&m, 1, "x"), None);
}

#[test]
fn the_interface_and_path_are_the_shell_s() {
    assert_eq!(menus::INTERFACE, "org.wolos.Menus");
    let appmenu = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../shell/src/appmenu.rs"
    ))
    .unwrap();
    assert!(
        appmenu.contains(&format!(
            "pub const WOLKIT_MENUS: &str = \"{}\";",
            menus::INTERFACE
        )),
        "the shell reads the menus from another interface"
    );
}

// ---- on the wire -------------------------------------------------------------------------

struct Daemon {
    child: Child,
    address: String,
}

// Stops the private bus daemon.
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn daemon(tag: &str) -> Option<Daemon> {
    let dir = std::env::temp_dir().join(format!("wolkit-menus-{tag}-{}", std::process::id()));
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
    (!address.is_empty()).then_some(Daemon { child, address })
}

const NAME: &str = "org.example.menus";
const PATH: &str = "/org/example/menus";

fn get(c: &zbus::blocking::Connection) -> Option<String> {
    c.call_method(Some(NAME), PATH, Some(menus::INTERFACE), "Get", &())
        .ok()?
        .body()
        .deserialize::<String>()
        .ok()
}

fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        wolkit::loop_::wait(&[], 20, &mut Vec::new());
    }
}

#[test]
fn the_bus_serves_get_activate_and_changed() {
    let Some(d) = daemon("wire") else {
        println!("no message bus daemon here: the wire test of the menus is skipped");
        return;
    };
    let hub = Arc::new(Hub::new(NAME).unwrap());
    let slot = hub.register();
    let svc = Service::start(NAME, hub.clone(), Bus::Address(&d.address)).unwrap();
    let client = zbus::blocking::connection::Builder::address(d.address.as_str())
        .unwrap()
        .method_timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    // Before any window has menus the name may not exist yet or answers an empty list.
    let w = app();
    let m = menus::collect(&w.core.scene);
    let want = serde_json::to_string(&m).unwrap();
    // The change signal reaches a client that subscribed before the menus came.
    let rule = format!(
        "type='signal',interface='{}',member='Changed'",
        menus::INTERFACE
    );
    let mut changes =
        zbus::blocking::MessageIterator::for_match_rule(rule.as_str(), &client, Some(8)).unwrap();
    wait("the name", || get(&client).map(|_| ()));
    svc.publish(slot, true, m.clone());
    wait("the menus", || {
        (get(&client).as_deref() == Some(&want)).then_some(())
    });
    assert_eq!(svc.json(), want);
    let sig = changes.next().and_then(Result::ok);
    assert!(sig.is_some(), "Changed was signalled");

    // The shell parses what it gets with its own checks: the same JSON round trip.
    let back: Vec<Menu> = serde_json::from_str(&want).unwrap();
    assert_eq!(back, m);

    // Activating an enabled item queues the default action on the node of that window.
    let new = w.core.scene.id(3).0;
    let args = (1u32, new.to_string());
    client
        .call_method(Some(NAME), PATH, Some(menus::INTERFACE), "Activate", &args)
        .unwrap();
    assert_eq!(
        hub.take_actions(slot),
        vec![(new, Action::Default)],
        "the click reaches the window as an ordinary default action"
    );
    // A disabled, unknown or mistyped item is refused and queues nothing.
    for (menu, key) in [
        (1u32, w.core.scene.id(4).0.to_string()),
        (2, new.to_string()),
        (1, "12345678".to_string()),
        (1, "no".to_string()),
    ] {
        let r = client.call_method(
            Some(NAME),
            PATH,
            Some(menus::INTERFACE),
            "Activate",
            &(menu, key.as_str()),
        );
        assert!(r.is_err(), "{menu} {key} is refused");
    }
    assert!(hub.take_actions(slot).is_empty());

    // The window goes away: the list is empty again.
    svc.remove(slot);
    wait("no menus", || {
        (get(&client).as_deref() == Some("[]")).then_some(())
    });
}

#[test]
fn the_window_in_front_answers() {
    let Some(d) = daemon("front") else {
        println!("no message bus daemon here: the wire test of the menus is skipped");
        return;
    };
    let hub = Arc::new(Hub::new(NAME).unwrap());
    let (a, b) = (hub.register(), hub.register());
    let svc = Service::start(NAME, hub, Bus::Address(&d.address)).unwrap();
    let w = app();
    let all = menus::collect(&w.core.scene);
    let first = all[..1].to_vec();
    svc.publish(a, true, first.clone());
    svc.publish(b, false, all.clone());
    assert_eq!(
        svc.json(),
        serde_json::to_string(&first).unwrap(),
        "a is in front"
    );
    svc.publish(b, true, all.clone());
    assert_eq!(
        svc.json(),
        serde_json::to_string(&all).unwrap(),
        "b came to the front"
    );
    svc.remove(b);
    assert_eq!(
        svc.json(),
        serde_json::to_string(&first).unwrap(),
        "back to a"
    );
    svc.remove(a);
    assert_eq!(svc.json(), "[]");
}
