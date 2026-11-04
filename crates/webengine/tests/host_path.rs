// SPDX-License-Identifier: BSD-2-Clause
//! The message path of a system web app, end to end except the engine and wold (ADR 0048): the real
//! kit `bridge.js` and the Hello app's own calls run in a JavaScript interpreter (node, when it is
//! installed) inside a stand-in window; the engine library's injected script (`channel.rs`, included
//! here as the library builds it) defines `window.__wolosHost` there; what the page posts comes out as
//! the console line only that channel can write, goes through `channel::from_console` into the host
//! (`Host`, the real `Bridge`) and a wold stand-in (the recorded fixtures), and each reply is handed
//! back through the delivery script (`channel::delivery`), exactly the strings the shim evaluates.
//! What this does not run: the engine that evaluates those scripts and the real wold.
#[path = "../src/engine/channel.rs"]
mod channel;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use webengine::bridge::manifest::{KIT, Manifest};
use webengine::bridge::mock::Fixtures;
use webengine::bridge::{AppInfo, Bridge, Theme};
use webengine::host::Host;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// argv: bootstrap, bridge.js, origin, secret, then `DELIVER` lines on stdin. The page script is the
/// Hello app's four calls, a call the app never declared, an unknown method and a subscription.
const DRIVER: &str = r#"
const vm = require("vm");
const readline = require("readline");
const [boot, bridge, origin, secret] = process.argv.slice(1);
const doc = { documentElement: { dataset: {}, lang: "" } };
const win = {
  location: { href: origin + "index.html", search: "" },
  console: { debug: (s) => process.stdout.write("LINE " + s + "\n"), log() {}, warn() {} },
  document: doc, navigator: { language: "en" },
  matchMedia: () => ({ matches: false }),
  setTimeout, Promise, JSON, Object, Map, Error, TypeError, String,
};
win.window = win; win.top = win;
const ctx = vm.createContext(win);
vm.runInContext(boot, ctx);
vm.runInContext(bridge, ctx);
const deliver = readline.createInterface({ input: process.stdin });
deliver.on("line", (l) => { if (l.startsWith("DELIVER ")) vm.runInContext(JSON.parse(l.slice(8)), ctx); });
(async () => {
  const w = win.wolos;
  const out = { permissions: w.permissions, app: w.app, dev: w.dev };
  const ask = async (m, p) => { try { return { ok: await w.call(m, p) }; } catch (e) { return { err: { code: e.code, message: e.message, hint: e.hint } }; } };
  const seen = [];
  w.subscribe("progress", (d) => seen.push(d));
  out.about = await ask("about");
  [out.power, out.disks, out.audio] = await Promise.all([ask("power.status"), ask("sys", { topic: "disks" }), ask("audio.doctor")]);
  out.undeclared = await ask("games.doctor");
  out.unknown = await ask("rm.rf");
  out.badparams = await ask("sys", [1]);
  out.subscribed = seen.length;
  process.stdout.write("RESULT " + JSON.stringify(out) + "\n");
  process.exit(0);
})();
"#;

fn node() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn hello_calls_travel_page_to_host_to_wold_and_back() {
    if !node() {
        eprintln!("no node: the page side of the message path is not run here");
        return;
    }
    let dir = repo().join("webapps/hello");
    let m = Manifest::load(&dir).unwrap();
    let mut wold =
        Fixtures::parse(&read(&repo().join("webapps").join(KIT).join("fixtures.js"))).unwrap();
    wold.merge(Fixtures::parse(&read(&dir.join("fixtures.js"))).unwrap());
    let app = AppInfo {
        id: m.id.clone(),
        version: m.version.clone(),
        locale: "en".into(),
        theme: Theme::Dark,
        accent: "blue".into(),
    };
    let bridge = Bridge::new(m.clone(), app, &m.permissions);
    let init = bridge.init().to_string();
    let mut host = Host::new(bridge, Arc::new(wold.clone()));

    let origin = format!("file://{}/", dir.display());
    // The shim's own checks of the origin and of the page's address accept this app's.
    assert!(
        channel::valid_origin(&origin) && channel::inside(&origin, &format!("{origin}index.html"))
    );
    let secret = channel::secret().unwrap();
    let boot = channel::bootstrap(&origin, &init, &secret);
    let mut child = Command::new("node")
        .args([
            "-e",
            DRIVER,
            "--",
            &boot,
            &read(&repo().join("webapps/kit/bridge.js")),
            &origin,
            &secret,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let out = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for l in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let (mut posted, mut delivered) = (0, 0);
    let end = Instant::now() + Duration::from_secs(30);
    let result = loop {
        assert!(
            Instant::now() < end,
            "timed out after {posted} posts, {delivered} replies"
        );
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(l) => {
                if let Some(r) = l.strip_prefix("RESULT ") {
                    break r.to_owned();
                }
                let text = l
                    .strip_prefix("LINE ")
                    .and_then(|line| channel::from_console(&secret, line))
                    .unwrap_or_else(|| panic!("a console line that is not the channel's: {l}"));
                posted += 1;
                host.on_page_message(text);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("the page ended early: {e}"),
        }
        for reply in host.take_ready() {
            delivered += 1;
            let script = channel::delivery(&reply);
            writeln!(stdin, "DELIVER {}", serde_json::to_string(&script).unwrap()).unwrap();
        }
    };
    let _ = child.wait();
    let r: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(r["dev"], false, "the runtime's channel, not the mock: {r}");
    assert_eq!(
        r["permissions"],
        serde_json::json!(["system.read", "audio.read"])
    );
    assert_eq!(r["app"]["theme"], "dark");
    assert_eq!(r["app"]["accent"], "blue");
    // The answers are wold's (here the recorded ones), byte for byte.
    assert_eq!(r["about"]["ok"], wold.calls["about"]);
    assert_eq!(r["power"]["ok"], wold.calls["power.status"]);
    assert_eq!(r["audio"]["ok"], wold.calls["audio.doctor"]);
    assert!(r["disks"].get("ok").is_some(), "{}", r["disks"]);
    // What the host refuses never reached wold, and the page sees a code and a sentence.
    assert_eq!(r["undeclared"]["err"]["code"], "denied");
    assert_eq!(r["unknown"]["err"]["code"], "not-found");
    assert_eq!(r["badparams"]["err"]["code"], "usage");
    assert!(
        r["unknown"]["err"]["message"]
            .as_str()
            .unwrap()
            .ends_with('.')
    );
    // Posted: 2 subscriptions of the bridge itself, 1 of the page's, 6 calls (the bad params never
    // leave the page: bridge.js refuses them); every call is answered, and the bridge's two
    // subscriptions get the record as it is now (QA-WEB-2).
    assert_eq!(posted, 3 + 6, "posted");
    assert_eq!(delivered, 2 + 6, "delivered");
}

#[test]
fn a_page_in_another_folder_has_no_channel_so_the_bridge_fails_closed() {
    if !node() {
        return;
    }
    let script = r#"
const vm = require("vm");
const [boot, bridge] = process.argv.slice(1);
const win = {
  location: { href: "file:///tmp/evil/index.html", search: "" },
  console: { debug() { throw new Error("posted"); } },
  document: { documentElement: { dataset: {} } }, navigator: { language: "en" },
  matchMedia: () => ({ matches: false }), Promise, JSON, Object, Map, Error, TypeError, String, setTimeout,
};
win.window = win; win.top = win;
const ctx = vm.createContext(win);
vm.runInContext(boot, ctx); vm.runInContext(bridge, ctx);
win.wolos.call("about").then(() => console.log("answered"), (e) => console.log(e.code));
"#;
    let boot = channel::bootstrap("file:///usr/share/wolos/webapps/hello/", "{}", "s");
    let out = Command::new("node")
        .args([
            "-e",
            script,
            "--",
            &boot,
            &read(&repo().join("webapps/kit/bridge.js")),
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "unavailable",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
