// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! libwol 1.2 from C: `tests/c/menus.c` against fake wold and Kit hosts on the real `shared` wires.
use shared::portal::{Answer, Call};
use shared::view::{FromHost, Menu, MenuEntry, Node, ToHost, Tool};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod common;
mod cprog;

fn fake_wold(path: &Path, calls: Arc<Mutex<Vec<Call>>>) {
    let l = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            let json = line.trim().strip_prefix("call ").unwrap();
            calls
                .lock()
                .unwrap()
                .push(Call::decode(json.as_bytes()).unwrap());
            let mut a = Answer::ok();
            a.id = Some(9);
            (&conn).write_all(a.to_line().as_bytes()).unwrap();
        }
    });
}

/// Records every message; once the window is up, chooses a menu item, a forged key, a key on a
/// forged source and a toolbar button.
fn fake_kit(path: &Path, shown: Arc<Mutex<Vec<ToHost>>>) {
    let l = UnixListener::bind(path).unwrap();
    std::thread::spawn(move || {
        let conn = l.incoming().next().unwrap().unwrap();
        let mut r = BufReader::new(&conn);
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let m = ToHost::parse(&line).unwrap_or_else(|| panic!("{line}"));
            let first = shown.lock().unwrap().is_empty();
            shown.lock().unwrap().push(m.clone());
            if let (
                ToHost::Show {
                    window,
                    menus,
                    toolbar: Some(t),
                    ..
                },
                false,
            ) = (&m, first)
            {
                if shown.lock().unwrap().len() != 2 {
                    continue;
                }
                let window = *window;
                let chose = |source, key: &str| FromHost::Chose {
                    window,
                    source,
                    key: key.into(),
                };
                for e in [
                    chose(menus[0].id, "save"),
                    chose(menus[0].id, "forged"),
                    chose(menus[0].id, "open"),
                    chose(99, "open"),
                    chose(t.id, "share"),
                ] {
                    (&conn).write_all(e.to_line().as_bytes()).unwrap();
                }
            }
        }
    });
}

#[test]
fn menus_toolbars_and_notifications_from_c() {
    let dir = cprog::temp("menus");
    let exe = cprog::build("menus", &dir, common::lib_dir());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let shown = Arc::new(Mutex::new(Vec::new()));
    fake_wold(&dir.join("wold.sock"), calls.clone());
    fake_kit(&dir.join("kit.sock"), shown.clone());
    let got = cprog::run(&exe, &dir);
    assert_eq!(
        got,
        [
            "bad key refused",
            "null refused",
            "withdrawn",
            "menu: open",
            "tool: share ud",
            "chosen: 2"
        ],
        "a forged key, a disabled item and a forged source run nothing"
    );

    let body = |c: &Call| match c {
        Call::NotificationPost { body, .. } => format!("post {body}"),
        Call::NotificationReplace { id, body, .. } => format!("replace {id} {body}"),
        Call::NotificationWithdraw { id } => format!("withdraw {id}"),
        c => panic!("{c:?}"),
    };
    assert_eq!(
        calls.lock().unwrap().iter().map(body).collect::<Vec<_>>(),
        ["post 0 of 5", "replace 9 2 of 5", "withdraw 9"],
        "a text change before the send stays local; a second withdraw does nothing"
    );

    let shown = shown.lock().unwrap();
    let ToHost::Show {
        title,
        content,
        menus,
        toolbar,
        ..
    } = &shown[1]
    else {
        panic!("{:?}", shown[1])
    };
    assert_eq!(title, "Editor");
    assert!(matches!(content, Node::Label { text, .. } if text == "Hello"));
    let Menu { entries, .. } = &menus[0];
    assert_eq!(menus[0].title, "File");
    assert!(matches!(
        &entries[0],
        MenuEntry::Item { shortcut, checked: true, .. } if shortcut == "cmd+o"
    ));
    assert_eq!(entries[1], MenuEntry::Separator);
    assert!(matches!(
        &entries[2],
        MenuEntry::Item { enabled: false, .. }
    ));
    let t = toolbar.as_ref().unwrap();
    assert!(matches!(
        &t.tools[0],
        Tool::Button { icon, .. } if icon == "go-previous-symbolic"
    ));
    assert_eq!(t.tools[1], Tool::Spacer);
    assert!(matches!(&t.tools[2], Tool::Button { icon, .. } if icon.is_empty()));
    let last = shown.last().unwrap();
    assert!(
        matches!(last, ToHost::Show { menus, .. }
            if matches!(&menus[0].entries[2], MenuEntry::Item { enabled: true, .. })),
        "the callback's change redrew the window: {last:?}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
