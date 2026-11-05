// SPDX-License-Identifier: GPL-2.0-or-later
//! A test double of the browser's end of the pipe: answers the CDP methods the verbs use from
//! fixtures and records every command, so the daemon, the socket and the verbs run without a browser.
#![allow(dead_code)]
use atlas_drive::browser::Browser;
use atlas_drive::cdp::Framer;
use atlas_drive::daemon::Daemon;
use atlas_drive::server::{self, ServerCfg};
use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

pub fn ax_fixture() -> Value {
    let n = |id: &str,
             role: &str,
             name: &str,
             parent: &str,
             kids: &[&str],
             bk: i64,
             props: Value| {
        json!({"nodeId": id, "ignored": false, "role": {"type":"role","value":role}, "name": {"type":"computedString","value":name}, "parentId": parent, "childIds": kids, "backendDOMNodeId": bk, "properties": props})
    };
    json!({"nodes": [
        n("1", "RootWebArea", "Demo page", "", &["2", "3"], 1, json!([])),
        n("2", "heading", "Profile", "1", &[], 2, json!([{"name":"level","value":{"type":"integer","value":1}}])),
        n("3", "button", "Save", "1", &[], 3, json!([])),
    ]})
}

fn tiny_png() -> String {
    let mut b = vec![
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
    ];
    b.extend_from_slice(&4u32.to_be_bytes());
    b.extend_from_slice(&3u32.to_be_bytes());
    b.extend_from_slice(&[8, 2, 0, 0, 0]);
    atlas_drive::util::b64_encode(&b)
}

/// Start the double; returns the wired Browser and the log of commands.
pub fn fake_browser() -> (Browser, Seen) {
    let (cmd_r, cmd_w) = std::io::pipe().unwrap();
    let (ev_r, mut ev_w) = std::io::pipe().unwrap();
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        let mut f = Framer::new(cmd_r, 0, 64 << 20);
        let mut send = move |v: Value| {
            let mut t = v.to_string().into_bytes();
            t.push(0);
            ev_w.write_all(&t).is_ok()
        };
        while let Ok(Some(raw)) = f.next_msg() {
            let v: Value = serde_json::from_slice(&raw).unwrap();
            let (id, method) = (
                v["id"].clone(),
                v["method"].as_str().unwrap_or("").to_string(),
            );
            let sess = v["sessionId"].as_str().unwrap_or("").to_string();
            log.lock()
                .unwrap()
                .push((sess.clone(), method.clone(), v["params"].clone()));
            let reply = |r: Value| {
                let mut o = json!({"id": id, "result": r});
                if !sess.is_empty() {
                    o["sessionId"] = sess.clone().into();
                }
                o
            };
            match method.as_str() {
                "Target.setDiscoverTargets" => {
                    send(reply(json!({})));
                    send(
                        json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"T1","type":"page","title":"","url":"about:blank"}}}),
                    );
                }
                "Target.attachToTarget" => {
                    send(reply(json!({"sessionId":"S1"})));
                }
                "Page.navigate" => {
                    send(reply(json!({"frameId":"F1","loaderId":"L1"})));
                    send(
                        json!({"sessionId":"S1","method":"Page.frameNavigated","params":{"frame":{"id":"F1","loaderId":"L1","url":v["params"]["url"]}}}),
                    );
                    send(
                        json!({"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"T1","type":"page","title":"Demo page","url":v["params"]["url"]}}}),
                    );
                    send(
                        json!({"sessionId":"S1","method":"Page.lifecycleEvent","params":{"frameId":"F1","loaderId":"L1","name":"load"}}),
                    );
                }
                "Accessibility.getFullAXTree" => {
                    send(reply(ax_fixture()));
                }
                "DOM.getContentQuads" => {
                    send(reply(
                        json!({"quads":[[10.0,10.0,50.0,10.0,50.0,30.0,10.0,30.0]]}),
                    ));
                }
                "Runtime.evaluate" => {
                    let expr = v["params"]["expression"].as_str().unwrap_or("");
                    let val = if expr.contains("INJ=") {
                        json!({"found": [], "scanned": 3})
                    } else {
                        json!("")
                    };
                    send(reply(json!({"result": {"type": "object", "value": val}})));
                }
                "Page.captureScreenshot" => {
                    send(reply(json!({"data": tiny_png()})));
                }
                "Browser.close" => {
                    send(reply(json!({})));
                    return;
                }
                _ => {
                    send(reply(json!({})));
                }
            }
        }
    });
    let b = Browser::from_streams(Box::new(ev_r), Box::new(cmd_w), None, None, false, None, 0);
    b.call(None, "Target.setDiscoverTargets", json!({"discover": true}))
        .unwrap();
    b.wait_first_page(std::time::Duration::from_secs(5))
        .unwrap();
    (b, seen)
}

pub struct Server {
    pub socket: PathBuf,
    pub dir: PathBuf,
    pub seen: Seen,
    pub grants: PathBuf,
}

/// A daemon on a temp socket, wired to the double. `ci` skips consent.
pub fn start(name: &str, ci: bool) -> Server {
    let dir = std::env::temp_dir().join(format!("ad-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("atlas/drive.sock");
    let listener = server::bind(&socket).unwrap();
    let (b, seen) = fake_browser();
    let d = Daemon::new(
        atlas_drive::browser::LaunchConfig::default(),
        dir.join("state"),
    );
    d.attach(b);
    let grants = dir.join("grants.json");
    let cfg = ServerCfg {
        grants_path: grants.clone(),
        ci,
        consent_cmd: None,
        own_exe: None,
        attached_file: Some(dir.join("drive.attached")),
    };
    std::thread::spawn(move || {
        let _ = server::serve(listener, d, cfg);
    });
    Server {
        socket,
        dir,
        seen,
        grants,
    }
}
