// SPDX-License-Identifier: BSD-2-Clause
//! The plain-Rust half of the host channel (`wwe_engine_set_host_channel`, `wwe_view_post_message`,
//! WWE_1.1, ADR 0048): the script the engine runs in each top-level document of the app before any
//! page script, how a page message is told from the page's other console output, and the script
//! that hands the host's message to the page. Only std: `tests/engine_channel.rs` includes this file
//! and tests it in the WolOS workspace without the engine (and runs the scripts in a JavaScript
//! interpreter where one is installed).
//!
//! How the page reaches the host. The engine has no page-to-embedder message call at the pin, so
//! `window.__wolosHost.post(text)` writes one `console.debug` line, `MARK`, the view's secret and
//! the text, and the shim reads it in the delegate's console callback. The secret is made per
//! engine from the kernel's random source and lives only in the closure of the injected script, so a
//! document that does not have the channel (a frame, another origin, the engine's own messages)
//! cannot forge a message: it does not know the secret. The line is never logged.
use std::io::Read;

/// Largest text either way (`wwe_message_fn`: longer texts are dropped).
pub const MAX_MESSAGE: usize = 64 * 1024;
/// What a page message line starts with.
pub const MARK: &str = "wolos-host:";
/// The property the channel is defined as on the page's `window`.
pub const PROPERTY: &str = "__wolosHost";

/// A random 128-bit secret as 32 hex digits, or why the kernel would not give one.
pub fn secret() -> Result<String, String> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| format!("no random bytes for the host channel: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// `file:///dir/`: a folder of the policy, the only kind of origin the channel is given to.
pub fn valid_origin(origin: &str) -> bool {
    origin.len() > "file:///".len()
        && origin.starts_with("file:///")
        && origin.ends_with('/')
        && !origin.contains(['?', '#', '"', '\'', '\\', '\n', '\r'])
}

/// `s` as a JavaScript string literal (UTF-16 safe, nothing that ends a script or a line).
pub fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            c if c.is_control() || c == '\u{2028}' || c == '\u{2029}' => {
                let mut u = [0u16; 2];
                for unit in c.encode_utf16(&mut u) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The user script of a view: defines `window.__wolosHost` in a top-level document whose URL is
/// inside `origin` and nowhere else. `init_json` is parsed in the page (`JSON.parse` of a string
/// literal: it can carry nothing but data), the property is not writable and not configurable, and
/// `post` and the console it writes to are captured now, before any page script can replace them.
pub fn bootstrap(origin: &str, init_json: &str, secret: &str) -> String {
    format!(
        r#"(function () {{
  "use strict";
  try {{
    if (window !== window.top) return;
    var origin = {origin};
    if (String(location.href).indexOf(origin) !== 0) return;
    if (Object.prototype.hasOwnProperty.call(window, {prop})) return;
    var line = console.debug.bind(console);
    var prefix = {mark} + {secret};
    var handler = null;
    var host = {{ init: Object.freeze(JSON.parse({init})) }};
    Object.defineProperty(host, "post", {{ enumerable: true, value: function (text) {{
      text = String(text);
      if (text.length > {max}) return;
      line(prefix + text);
    }} }});
    Object.defineProperty(host, "onmessage", {{ enumerable: true,
      get: function () {{ return handler; }},
      set: function (f) {{ handler = typeof f === "function" ? f : null; }} }});
    Object.defineProperty(window, {prop}, {{ value: Object.freeze(host), writable: false, configurable: false }});
  }} catch (e) {{}}
}})();
"#,
        origin = js_string(origin),
        prop = js_string(PROPERTY),
        mark = js_string(MARK),
        secret = js_string(secret),
        init = js_string(init_json),
        max = MAX_MESSAGE,
    )
}

/// The page message in a console line, when the line is one: `MARK`, `secret`, the text. At most
/// [`MAX_MESSAGE`] bytes (a longer text is dropped, as `post` does in the page).
pub fn from_console<'a>(secret: &str, line: &'a str) -> Option<&'a str> {
    let text = line.strip_prefix(MARK)?.strip_prefix(secret)?;
    (text.len() <= MAX_MESSAGE).then_some(text)
}

/// The script that calls the page's `onmessage` with `text`; it evaluates to whether a handler ran.
pub fn delivery(text: &str) -> String {
    format!(
        "(function () {{ var h = window[{prop}]; if (!h || typeof h.onmessage !== \"function\") return false; \
         try {{ h.onmessage({text}); return true; }} catch (e) {{ return false; }} }})()",
        prop = js_string(PROPERTY),
        text = js_string(text),
    )
}

/// Whether `url` (the view's top-level document) is inside `origin`.
pub fn inside(origin: &str, url: &str) -> bool {
    url.starts_with(origin)
}
