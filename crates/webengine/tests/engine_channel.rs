// SPDX-License-Identifier: BSD-2-Clause
//! The engine library's host channel (`src/engine/channel.rs`, WWE_1.1, ADR 0048), without the
//! engine: the strings, and (when `node` is installed) the injected script run in a JavaScript
//! interpreter against a stand-in window, the way the engine runs it: defined once, only in the
//! app's top-level document, not replaceable, a message out as one console line only the channel
//! can write, a message in through the handler the page set.
#[path = "../src/engine/channel.rs"]
mod channel;

use std::process::Command;

const ORIGIN: &str = "file:///usr/share/wolos/webapps/hello/";

#[test]
fn a_string_literal_carries_any_text_and_ends_no_script_or_line() {
    assert_eq!(channel::js_string("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
    let s = channel::js_string("</script>\u{2028}\u{0}é😀");
    assert!(
        !s.contains('<') && !s.contains('\u{2028}') && !s.contains('\u{0}'),
        "{s}"
    );
    assert!(s.contains("\\ud83d\\ude00") || s.contains('😀'), "{s}");
}

#[test]
fn secrets_are_random_hex_and_origins_are_policy_folders() {
    let (a, b) = (channel::secret().unwrap(), channel::secret().unwrap());
    assert_eq!(a.len(), 32);
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
    assert!(channel::valid_origin(ORIGIN));
    for bad in [
        "",
        "file:///",
        "file:///a",
        "https://x/",
        "file:///a/?q=1/",
        "file:///a\"/",
        "file:///a'/",
    ] {
        assert!(!channel::valid_origin(bad), "{bad}");
    }
    assert!(channel::inside(
        ORIGIN,
        "file:///usr/share/wolos/webapps/hello/index.html#x"
    ));
    assert!(!channel::inside(
        ORIGIN,
        "file:///usr/share/wolos/webapps/hello2/index.html"
    ));
}

#[test]
fn only_a_line_with_the_secret_is_a_page_message() {
    let line = format!("{}{}{{\"id\":1}}", channel::MARK, "s3cret");
    assert_eq!(channel::from_console("s3cret", &line), Some("{\"id\":1}"));
    assert_eq!(channel::from_console("other", &line), None);
    assert_eq!(channel::from_console("s3cret", "{\"id\":1}"), None);
    assert_eq!(channel::from_console("s3cret", "wolos-host:"), None);
    let big = format!(
        "{}s3cret{}",
        channel::MARK,
        "x".repeat(channel::MAX_MESSAGE)
    );
    assert_eq!(
        channel::from_console("s3cret", &big).map(str::len),
        Some(channel::MAX_MESSAGE)
    );
    let over = format!(
        "{}s3cret{}",
        channel::MARK,
        "x".repeat(channel::MAX_MESSAGE + 1)
    );
    assert_eq!(channel::from_console("s3cret", &over), None);
}

/// The driver: argv = bootstrap script, delivery script, origin, secret. Prints one JSON report.
const DRIVER: &str = r#"
const vm = require("vm");
const [boot, deliver, origin, secret] = process.argv.slice(1);
function world(href, top) {
  const lines = [];
  const win = { location: { href }, console: { debug: (s) => lines.push(s) } };
  win.window = win; win.top = top ? win : {};
  return { ctx: vm.createContext(win), win, lines };
}
const report = {};
{ // the app's own document
  const w = world(origin + "index.html", true);
  vm.runInContext(boot, w.ctx);
  const h = w.win.__wolosHost;
  report.defined = !!h && h.init.app.id === "x" && Object.isFrozen(h.init);
  w.win.console = { debug: () => { throw new Error("page console used"); } };
  h.post('{"id":1}');
  report.line = w.lines[0];
  h.post("x".repeat(70000));
  report.dropped = w.lines.length === 1;
  report.locked = (() => { try { w.win.__wolosHost = 1; } catch (e) {} return w.win.__wolosHost === h; })();
  report.noRedefine = (() => { try { Object.defineProperty(w.win, "__wolosHost", { value: 1 }); return false; } catch (e) { return true; } })();
  report.noHandler = vm.runInContext(deliver, w.ctx);
  let got = null;
  h.onmessage = (t) => { got = t; };
  report.delivered = vm.runInContext(deliver, w.ctx);
  report.got = got;
  h.onmessage = () => { throw new Error("boom"); };
  report.throwing = vm.runInContext(deliver, w.ctx);
  h.onmessage = 5;
  report.notFn = h.onmessage === null;
}
{ // a frame, another folder, another scheme
  for (const [href, top] of [[origin + "a.html", false], ["file:///usr/share/wolos/webapps/other/index.html", true], ["https://example.org/", true]]) {
    const w = world(href, top);
    vm.runInContext(boot, w.ctx);
    (report.elsewhere ||= []).push("__wolosHost" in w.win);
  }
}
{ // a page that already defined the name keeps its own value
  const w = world(origin + "index.html", true);
  w.win.__wolosHost = "page";
  vm.runInContext(boot, w.ctx);
  report.mine = w.win.__wolosHost;
}
console.log(JSON.stringify(report));
"#;

fn node() -> Option<String> {
    let ok = Command::new("node")
        .arg("--version")
        .output()
        .ok()?
        .status
        .success();
    ok.then(|| "node".to_owned())
}

#[test]
fn the_injected_script_defines_a_locked_channel_in_the_apps_top_document_only() {
    let Some(node) = node() else {
        eprintln!("no node: the injected script is not run here");
        return;
    };
    let secret = channel::secret().unwrap();
    let boot = channel::bootstrap(ORIGIN, r#"{"app":{"id":"x"},"permissions":["a"]}"#, &secret);
    let deliver = channel::delivery("hello \"there\"\n</script>");
    let out = Command::new(node)
        .args(["-e", DRIVER, "--", &boot, &deliver, ORIGIN, &secret])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let r: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(r["defined"], true, "{r}");
    assert_eq!(r["line"], format!("{}{secret}{{\"id\":1}}", channel::MARK));
    assert_eq!(
        channel::from_console(&secret, r["line"].as_str().unwrap()),
        Some("{\"id\":1}")
    );
    assert_eq!(r["dropped"], true, "a text over 64 KiB is not posted");
    assert_eq!(r["locked"], true);
    assert_eq!(r["noRedefine"], true);
    assert_eq!(
        r["noHandler"], false,
        "nothing is delivered before the page set a handler"
    );
    assert_eq!(r["delivered"], true);
    assert_eq!(r["got"], "hello \"there\"\n</script>");
    assert_eq!(r["throwing"], false, "a throwing handler is not delivered");
    assert_eq!(r["notFn"], true);
    assert_eq!(
        r["elsewhere"],
        serde_json::json!([false, false, false]),
        "{r}"
    );
    assert_eq!(r["mine"], "page");
}
