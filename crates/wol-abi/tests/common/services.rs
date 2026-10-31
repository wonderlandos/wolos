// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! The `services` program every language binding must pass: each call added since 1.2 against a
//! fake wold on the real `shared::portal` wire.
#![allow(dead_code)]
use serde_json::json;
use shared::portal::{Answer, Call};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

fn answer(c: &Call) -> Answer {
    match c {
        Call::NotificationPost { .. } => {
            let mut a = Answer::ok();
            a.id = Some(7);
            a
        }
        Call::ClipboardRead {} => Answer::with(json!({"text": "copied"})),
        Call::PrefsGet { .. } => Answer::with(json!({"value": "640"})),
        Call::PrefsKeys {} => Answer::with(json!(["height", "width"])),
        Call::PanelOpen { .. } | Call::PanelSave { .. } => Answer::with(json!({"name": "a.txt"})),
        Call::PowerInfo {} => Answer::with(json!({
            "on_battery": true, "low_power": false, "battery": {"percent": 71, "charging": false}
        })),
        _ => Answer::ok(),
    }
}

/// Run `cmd`, check what it prints and that each call reached wold in order.
pub fn run_services(mut cmd: Command, dir: &Path) {
    let picked = dir.join("picked.txt");
    std::fs::write(&picked, "from the panel").unwrap();
    let calls: Arc<Mutex<Vec<Call>>> = Arc::default();
    let l = UnixListener::bind(dir.join("wold.sock")).unwrap();
    let seen = calls.clone();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            let json = line.trim().strip_prefix("call ").unwrap();
            let call = Call::decode(json.as_bytes()).unwrap();
            let a = answer(&call);
            seen.lock().unwrap().push(call);
            if a.data.as_ref().is_some_and(|d| d.get("name").is_some()) {
                let f = std::fs::File::options()
                    .read(true)
                    .write(true)
                    .open(&picked)
                    .unwrap();
                shared::sys::send_with_fd(&conn, a.to_line().as_bytes(), f.as_fd()).unwrap();
                continue;
            }
            (&conn).write_all(a.to_line().as_bytes()).unwrap();
        }
    });
    let out = cmd
        .env("WOL_SOCKET", dir.join("wold.sock"))
        .env("WOL_KIT_SOCKET", dir.join("kit.sock"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "menu",
            "toolbar",
            "note",
            "clipboard: copied",
            "pref: 640",
            "keys: 2 width",
            "panel: a.txt from the panel",
            "battery: 71 1",
            "held",
        ]
    );
    let names: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| {
            serde_json::from_str::<serde_json::Value>(&c.encode()).unwrap()["call"].to_string()
        })
        .map(|s| s.trim_matches('"').to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "notification.post",
            "notification.replace",
            "notification.withdraw",
            "clipboard.write",
            "clipboard.read",
            "prefs.set",
            "prefs.get",
            "prefs.keys",
            "prefs.remove",
            "panel.open",
            "panel.save",
            "power.info",
            "power.keep-awake",
        ]
    );
}
