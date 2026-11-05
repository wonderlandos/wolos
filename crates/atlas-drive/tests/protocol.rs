// SPDX-License-Identifier: GPL-2.0-or-later
//! The daemon, the socket and the verbs against a test double of the browser's pipe.
mod support;
use atlas_drive::client::Client;
use atlas_drive::perms;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use support::start;

fn methods(s: &support::Seen) -> Vec<String> {
    s.lock()
        .unwrap()
        .iter()
        .map(|(_, m, _)| m.clone())
        .collect()
}

#[test]
fn the_socket_is_private() {
    let s = start("perm", true);
    assert_eq!(
        std::fs::metadata(&s.socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(s.socket.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    // a second daemon refuses a live socket
    assert!(atlas_drive::server::bind(&s.socket).is_err());
}

#[test]
fn goto_snapshot_click_round_trip() {
    let s = start("flow", true);
    let mut c = Client::connect(&s.socket).unwrap();
    let r = c.verb("goto", json!({"url": "http://a.test/"})).unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(r["changed"]["url"], "http://a.test/");
    assert_eq!(r["changed"]["title"], "Demo page");
    assert!(r["tokens"].as_u64().unwrap() > 0);
    let snap = c.verb("snapshot", json!({"safe": false})).unwrap();
    assert_eq!(
        snap["text"],
        "@ http://a.test/ \"Demo page\"\n[e1] h1 \"Profile\"\n[e2] button \"Save\"\n"
    );
    // click by ref: scroll + quads, then move, press, release in order
    let r = c.verb("click", json!({"target": "e2"})).unwrap();
    assert_eq!(r["ok"], true);
    assert!(
        r.get("clicked").is_none(),
        "a ref needs no echo of what it resolved to"
    );
    let m = methods(&s.seen);
    let mouse: Vec<&String> = m.iter().filter(|x| x.starts_with("Input.")).collect();
    assert_eq!(mouse, vec!["Input.dispatchMouseEvent"; 3]);
    let before = s.seen.lock().unwrap().len();
    let _ = before;
    let types: Vec<String> = s
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, m, _)| m == "Input.dispatchMouseEvent")
        .map(|(_, _, p)| p["type"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(types, vec!["mouseMoved", "mousePressed", "mouseReleased"]);
    let pressed = s
        .seen
        .lock()
        .unwrap()
        .iter()
        .find(|(_, _, p)| p["type"] == "mousePressed")
        .map(|(_, _, p)| p.clone())
        .unwrap();
    assert_eq!(
        (pressed["x"].as_f64(), pressed["y"].as_f64()),
        (Some(30.0), Some(20.0)),
        "the centre of the quad"
    );
    // a locator is echoed with the ref it resolved to
    let r = c
        .verb("click", json!({"target": "role=button name=Save"}))
        .unwrap();
    assert!(
        r["clicked"]
            .as_str()
            .unwrap()
            .contains("[e2] button \"Save\""),
        "{r}"
    );
    // a stale ref is one line with a hint
    let e = c.verb("click", json!({"target": "e99"})).unwrap_err();
    assert!(
        e.contains("unknown ref e99") && e.contains("run snapshot"),
        "{e}"
    );
}

#[test]
fn refs_from_an_earlier_page_are_refused() {
    let s = start("stale", true);
    let mut c = Client::connect(&s.socket).unwrap();
    c.verb("goto", json!({"url": "http://a.test/1"})).unwrap();
    c.verb("snapshot", json!({"safe": false})).unwrap();
    c.verb("goto", json!({"url": "http://a.test/2"})).unwrap();
    let e = c.verb("click", json!({"target": "e2"})).unwrap_err();
    assert!(e.contains("earlier page"), "{e}");
}

#[test]
fn raw_cdp_passes_through_with_the_clients_ids() {
    let s = start("raw", true);
    let mut c = Client::connect(&s.socket).unwrap();
    let r = c.cdp("Browser.getVersion", json!({})).unwrap();
    assert_eq!(r, json!({}));
    // ids are the client's own, even when another client uses the same ids
    let mut d = Client::connect(&s.socket).unwrap();
    let a = c
        .request("Runtime.evaluate", json!({"expression": "1"}), &mut |_| {})
        .unwrap();
    let b = d
        .request("Runtime.evaluate", json!({"expression": "1"}), &mut |_| {})
        .unwrap();
    assert_eq!(a["result"]["type"], b["result"]["type"]);
    assert!(methods(&s.seen).iter().any(|m| m == "Browser.getVersion"));
}

#[test]
fn a_budget_cuts_the_snapshot_and_says_so() {
    let s = start("budget", true);
    let mut c = Client::connect(&s.socket).unwrap();
    c.verb("goto", json!({"url": "http://a.test/"})).unwrap();
    let r = c
        .verb("snapshot", json!({"safe": false, "budget": 12}))
        .unwrap();
    assert_eq!(r["truncated"], true);
    assert!(
        r["text"]
            .as_str()
            .unwrap()
            .contains("cut to fit --budget 12")
    );
}

#[test]
fn batch_runs_parallel_branches_and_streams_each_step() {
    let s = start("batch", true);
    let mut c = Client::connect(&s.socket).unwrap();
    c.verb("goto", json!({"url": "http://a.test/"})).unwrap();
    let mut events: Vec<Value> = vec![];
    let r = c
        .request(
            "Drive.batch",
            json!({"parallel": true, "steps": [
                {"id": "shot", "verb": "screenshot", "params": {"out": s.dir.join("a.png").display().to_string()}},
                {"id": "snap", "verb": "snapshot", "params": {"safe": false}},
                {"id": "after", "verb": "tabs", "after": ["shot", "snap"]},
            ]}),
            &mut |v| events.push(v.clone()),
        )
        .unwrap();
    assert_eq!(r["ok"], true);
    assert_eq!(events.len(), 3, "one streamed event per step");
    let order: Vec<&str> = events
        .iter()
        .map(|e| e["event"]["step"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        order.last(),
        Some(&"after"),
        "a dependent step runs after the branches it waits for: {order:?}"
    );
    let png = std::fs::read(s.dir.join("a.png")).unwrap();
    assert_eq!(&png[1..4], b"PNG");
    assert_eq!(r["results"][0]["r"]["size"], json!([4, 3]));
}

#[test]
fn a_failing_step_stops_the_batch() {
    let s = start("fail", true);
    let mut c = Client::connect(&s.socket).unwrap();
    let r = c
        .verb(
            "batch",
            json!({"steps": [{"verb": "click", "params": {"target": "e5"}}, {"verb": "tabs"}]}),
        )
        .unwrap();
    assert_eq!(r["ok"], false);
    assert_eq!(r["failed"], "0");
    assert_eq!(r["results"][1]["skipped"], true);
}

#[test]
fn cdp_events_streams_matching_events_only() {
    let s = start("events", true);
    let mut c = Client::connect(&s.socket).unwrap();
    let mut lines: Vec<Value> = vec![];
    let mut other = Client::connect(&s.socket).unwrap();
    let t = std::thread::spawn({
        let sock = s.socket.clone();
        move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let mut c2 = Client::connect(&sock).unwrap();
            c2.verb("goto", json!({"url": "http://a.test/ev"})).unwrap();
        }
    });
    let r = c
        .request(
            "Drive.cdp_events",
            json!({"events": ["Page.lifecycleEvent"], "ms": 900, "limit": 1}),
            &mut |v| lines.push(v.clone()),
        )
        .unwrap();
    t.join().unwrap();
    let _ = other.verb("status", json!({}));
    assert_eq!(r["events"], 1);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["method"], "Page.lifecycleEvent");
}

#[test]
fn an_unknown_agent_is_refused_until_allowed_then_revoked() {
    let s = start("consent", false);
    let me = std::env::current_exe()
        .unwrap()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();
    let mut c = Client::connect(&s.socket).unwrap();
    let e = c.verb("snapshot", json!({})).unwrap_err();
    assert!(
        e.contains("not allowed") && e.contains("permissions allow"),
        "{e}"
    );
    // the person allows read: reading works, acting does not (a scope not granted is not assumed)
    let mut g = perms::Grants::default();
    g.grant(&me, &["read".into()], 1);
    perms::save(&s.grants, &g).unwrap();
    assert!(c.verb("tabs", json!({})).is_ok());
    let e = c.verb("click", json!({"target": "e1"})).unwrap_err();
    assert!(e.contains("'act'"), "{e}");
    // a read-only agent cannot choose where a file is written
    let e = c
        .verb(
            "screenshot",
            json!({"out": "/tmp/should-not-be-written.png"}),
        )
        .unwrap_err();
    assert!(e.contains("'files'"), "{e}");
    assert!(!std::path::Path::new("/tmp/should-not-be-written.png").exists());
    // raw CDP needs every scope
    let e = c.cdp("Page.reload", json!({})).unwrap_err();
    assert!(e.contains("every scope"), "{e}");
    // AUTO mode stops the prompting; revoking AUTO brings it back
    g.auto = Some(perms::Auto {
        until: None,
        sites: vec![],
    });
    perms::save(&s.grants, &g).unwrap();
    assert!(c.verb("goto", json!({"url": "http://a.test/"})).is_ok());
    g.auto = None;
    g.revoke(&me);
    perms::save(&s.grants, &g).unwrap();
    assert!(c.verb("tabs", json!({})).is_err());
}

#[test]
fn an_attached_agent_is_published_for_the_indicator() {
    let s = start("attached", false);
    let me = std::env::current_exe()
        .unwrap()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();
    let mut g = perms::Grants::default();
    g.grant(&me, &["read".into()], 1);
    perms::save(&s.grants, &g).unwrap();
    let mut c = Client::connect(&s.socket).unwrap();
    c.verb("tabs", json!({})).unwrap();
    let f = s.dir.join("drive.attached");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
    assert_eq!(v["attached"][0]["agent"], me.as_str());
    drop(c);
    for _ in 0..50 {
        if !f.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!f.exists(), "the indicator file goes with the last agent");
}

#[test]
fn bad_input_is_a_json_error_not_a_hang() {
    let s = start("bad", true);
    use std::io::{BufRead, BufReader, Write};
    let mut st = std::os::unix::net::UnixStream::connect(&s.socket).unwrap();
    st.write_all(b"this is not json\n{\"id\":4}\n{\"id\":5,\"method\":\"Drive.nope\"}\n")
        .unwrap();
    let mut r = BufReader::new(st);
    let mut l = String::new();
    r.read_line(&mut l).unwrap();
    assert!(l.contains("not JSON"), "{l}");
    l.clear();
    r.read_line(&mut l).unwrap();
    assert!(l.contains("needs a method"), "{l}");
    l.clear();
    r.read_line(&mut l).unwrap();
    assert!(l.contains("unknown verb nope"), "{l}");
}

#[test]
fn auto_mode_for_a_site_covers_where_a_navigation_goes() {
    let s = start("autosite", false);
    let mut g = perms::Grants {
        auto: Some(perms::Auto {
            until: None,
            sites: vec!["a.test".into()],
        }),
        ..Default::default()
    };
    perms::save(&s.grants, &g).unwrap();
    let mut c = Client::connect(&s.socket).unwrap();
    assert!(c.verb("goto", json!({"url": "http://a.test/x"})).is_ok());
    assert!(
        c.verb("goto", json!({"url": "https://app.a.test/"}))
            .is_ok(),
        "a subdomain is the same site"
    );
    let e = c
        .verb("goto", json!({"url": "http://evil.test/"}))
        .unwrap_err();
    assert!(e.contains("not allowed"), "{e}");
    g.auto = None;
    perms::save(&s.grants, &g).unwrap();
    assert!(
        c.verb("goto", json!({"url": "http://a.test/"})).is_err(),
        "AUTO off: asked again"
    );
}
