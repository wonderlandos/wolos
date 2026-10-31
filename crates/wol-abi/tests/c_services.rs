// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! The service families from C (ADR 0020): each `tests/c/<family>.c` runs against a fake wold that
//! answers on the real `shared::portal` wire. wold's own decisions are tested in `api::portal`.
use serde_json::json;
use shared::portal::{Answer, Call, Code};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod common;
mod cprog;

type Calls = Arc<Mutex<Vec<Call>>>;

/// Every line must be a valid call; `f` answers it, given how many came so far.
fn fake_wold(path: &Path, f: fn(&Call, usize) -> Answer) -> Calls {
    let calls: Calls = Arc::default();
    let l = UnixListener::bind(path).unwrap();
    let path = path.to_owned();
    let c = calls.clone();
    std::thread::spawn(move || {
        let mut listener = None;
        for conn in l.incoming().flatten() {
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            if line.trim() == "subscribe" {
                listener = Some(conn);
                continue;
            }
            let json = line.trim().strip_prefix("call ").unwrap();
            let call = Call::decode(json.as_bytes()).unwrap();
            let event = common::services::search_event(&call);
            let a = {
                let mut all = c.lock().unwrap();
                all.push(call.clone());
                f(&call, all.len())
            };
            if a.data.as_ref().is_some_and(|d| d.get("name").is_some()) {
                let file = std::fs::File::options()
                    .read(true)
                    .write(true)
                    .open(path.with_file_name("picked.txt"))
                    .unwrap();
                shared::sys::send_with_fd(&conn, a.to_line().as_bytes(), file.as_fd()).unwrap();
                continue;
            }
            (&conn).write_all(a.to_line().as_bytes()).unwrap();
            if let (Some(e), Some(mut l)) = (event, listener.as_ref()) {
                l.write_all(e.to_line().as_bytes()).unwrap();
            }
        }
    });
    calls
}

/// Build and run `tests/c/<name>.c` against `f`.
fn run(name: &str, f: fn(&Call, usize) -> Answer) -> (Vec<String>, Vec<Call>) {
    let dir = cprog::temp(name);
    let exe = cprog::build(name, &dir, common::lib_dir());
    std::fs::write(dir.join("picked.txt"), "from the panel").unwrap();
    let calls = fake_wold(&dir.join("wold.sock"), f);
    let out = cprog::run(&exe, &dir);
    let _ = std::fs::remove_dir_all(dir);
    let c = calls.lock().unwrap().clone();
    (out, c)
}

#[test]
fn clipboard() {
    let (out, calls) = run("clipboard", |c, n| match (c, n) {
        (Call::ClipboardRead {}, _) => Answer::with(json!({"text": "copied"})),
        (_, 1) => Answer::ok(),
        _ => Answer::err(Code::PermissionDenied, "No clipboard for you."),
    });
    assert_eq!(
        out,
        [
            "text: copied",
            "null out: 1 out must not be NULL.",
            "too long refused",
            "denied: 2 No clipboard for you."
        ]
    );
    assert_eq!(
        calls,
        [
            Call::ClipboardWrite {
                text: "hello".into()
            },
            Call::ClipboardRead {},
            Call::ClipboardWrite {
                text: "denied".into()
            },
        ],
        "invalid calls never left the library"
    );
}

#[test]
fn prefs() {
    let (out, calls) = run("prefs", |c, _| match c {
        Call::PrefsGet { key } if key == "width" => Answer::with(json!({"value": "640"})),
        Call::PrefsGet { .. } => Answer::with(json!({})),
        Call::PrefsKeys {} => Answer::with(json!(["height", "width"])),
        _ => Answer::ok(),
    });
    assert_eq!(
        out,
        [
            "width: 640",
            "missing: null",
            "keys: 2 height width [\"height\",\"width\"]",
            "bad key: 1",
            "long value: 1"
        ]
    );
    assert_eq!(
        calls,
        [
            Call::PrefsSet {
                key: "width".into(),
                value: "640".into()
            },
            Call::PrefsGet {
                key: "width".into()
            },
            Call::PrefsGet {
                key: "nothing".into()
            },
            Call::PrefsKeys {},
            Call::PrefsRemove {
                key: "width".into()
            },
        ]
    );
}

#[test]
fn panels() {
    let (out, calls) = run("panels", |c, _| match c {
        Call::PanelOpen { title } if title == "Open" => Answer::with(json!({"name": "picked.txt"})),
        Call::PanelOpen { .. } => Answer::with(json!({"cancelled": true})),
        _ => Answer::with(json!({"name": "picked.txt"})),
    });
    assert_eq!(
        out,
        [
            "name: picked.txt",
            "read: from the panel",
            "cancelled: null",
            "wrote: picked.txt",
            "bad name: 1"
        ]
    );
    assert_eq!(calls.len(), 3);
}

#[test]
fn power() {
    let (out, calls) = run("power", |c, _| match c {
        Call::PowerInfo {} => Answer::with(json!({
            "on_battery": true, "low_power": false, "battery": {"percent": 71, "charging": false}
        })),
        _ => Answer::ok(),
    });
    assert_eq!(out, ["battery: 1 71 1", "held: 1", "no reason: 1"]);
    assert_eq!(
        calls,
        [
            Call::PowerInfo {},
            Call::PowerKeepAwake {
                reason: "Exporting a video".into()
            }
        ]
    );
}

#[test]
fn network() {
    let (out, calls) = run("network", |_, _| {
        Answer::with(json!({
            "online": true, "interfaces": [{"name": "wlan0", "kind": "wifi", "up": true}]
        }))
    });
    assert_eq!(out, ["online: 1 1", "wlan0 wifi", "null out: 1"]);
    assert_eq!(calls, [Call::NetworkInfo {}]);
}

#[test]
fn audio() {
    let (out, calls) = run("audio", |c, _| match c {
        Call::AudioInfo {} => Answer::with(json!({"volume": 55, "muted": true})),
        _ => Answer::ok(),
    });
    assert_eq!(out, ["volume: 55 1 0", "set", "too loud: 1"]);
    assert_eq!(
        calls,
        [
            Call::AudioInfo {},
            Call::AudioSetVolume { percent: 20 },
            Call::AudioSetMuted { muted: false },
        ]
    );
}

#[test]
fn search() {
    let (out, calls) = run("search", |c, _| match c {
        Call::SearchQuery { .. } => {
            Answer::with(json!({"results": [{"title": "Notes", "kind": "Application"}]}))
        }
        _ => Answer::ok(),
    });
    assert_eq!(
        out,
        [
            "found: 1 Notes",
            "twice: 1",
            "asked: term",
            "no title: 1",
            "open: note-1"
        ]
    );
    assert_eq!(
        calls,
        [
            Call::SearchQuery {
                text: "term".into()
            },
            Call::SearchProvide {},
            Call::SearchReply {
                id: 5,
                results: vec![shared::portal::SearchResult {
                    key: "note-1".into(),
                    title: "A note".into(),
                    detail: "Notes".into()
                }]
            },
        ]
    );
}

#[test]
fn account() {
    let (out, calls) = run("account", |_, _| {
        Answer::with(json!({"name": "alice", "full_name": "Alice Liddell"}))
    });
    assert_eq!(out, ["Alice Liddell (alice)", "null out: 1"]);
    assert_eq!(calls, [Call::AccountInfo {}]);
}

#[test]
fn message() {
    let (out, calls) = run("message", |c, _| match c {
        Call::IpcSend { .. } => Answer::with(json!({"body": "pong"})),
        _ => Answer::ok(),
    });
    assert_eq!(out, ["reply: pong", "got hello from /usr/bin/sender"]);
    assert_eq!(
        calls,
        [
            Call::IpcListen {
                name: "greet".into()
            },
            Call::IpcSend {
                peer: "/usr/bin/peer".into(),
                name: "greet".into(),
                body: "hello".into()
            },
            Call::IpcReply {
                id: 9,
                body: "pong".into()
            },
            Call::IpcPublish {
                topic: "news".into(),
                body: "item".into()
            },
        ]
    );
}

#[test]
fn device() {
    let (out, calls) = run("device", |c, _| match c {
        Call::DeviceList {} => {
            Answer::with(json!({"devices": [{"name": "Scanner", "vendor": 1133}]}))
        }
        _ => Answer::with(json!({"name": "Scanner"})),
    });
    assert_eq!(
        out,
        [
            "Scanner 1133",
            "opened Scanner: from the panel",
            "bad id: 1"
        ]
    );
    assert_eq!(
        calls,
        [
            Call::DeviceList {},
            Call::DeviceOpen {
                vendor: 1133,
                product: 2
            }
        ]
    );
}

#[test]
fn share() {
    let (out, calls) = run("share", |_, _| Answer::with(json!({"done": "copied"})));
    assert_eq!(out, ["done: copied", "no title: 1"]);
    assert_eq!(
        calls,
        [Call::ShareText {
            title: "A note".into(),
            text: "two\nlines".into()
        }]
    );
}
