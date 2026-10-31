// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
//! The service families from C (ADR 0020): each `tests/c/<family>.c` runs against a fake wold that
//! answers on the real `shared::portal` wire. wold's own decisions are tested in `api::portal`.
use serde_json::json;
use shared::portal::{Answer, Call, Code};
use std::io::{BufRead, BufReader, Write};
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
    let c = calls.clone();
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            let json = line.trim().strip_prefix("call ").unwrap();
            let call = Call::decode(json.as_bytes()).unwrap();
            let a = {
                let mut all = c.lock().unwrap();
                all.push(call.clone());
                f(&call, all.len())
            };
            (&conn).write_all(a.to_line().as_bytes()).unwrap();
        }
    });
    calls
}

/// Build and run `tests/c/<name>.c` against `f`.
fn run(name: &str, f: fn(&Call, usize) -> Answer) -> (Vec<String>, Vec<Call>) {
    let dir = cprog::temp(name);
    let exe = cprog::build(name, &dir, common::lib_dir());
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
