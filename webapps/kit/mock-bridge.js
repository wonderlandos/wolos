// SPDX-License-Identifier: BSD-2-Clause
// Development mode for system web apps (ADR 0048): a stand-in for the runtime's host channel, so
// a page runs in any desktop browser. Acts only when bridge.js finds no runtime and the address
// has ?dev. Answers come from fixture files (kit/fixtures.js, then the app's own fixtures.js).
// Address options: theme=light|dark, accent=<id>, locale=<tag>, deny=<perm,perm>, fail=<method,..>,
// delay=<ms> (default 120). The permission catalog below is the Rust one
// (crates/webengine/src/bridge/catalog.rs); tests/webapps.rs keeps them equal.
(function () {
  "use strict";
  var CATALOG = /*catalog*/{
    "system.read": ["about", "sys", "hw.status", "power.status", "battery", "boot.status", "modules.status"],
    "update.read": ["update.status", "update.check"],
    "packages.update": ["update.apply"],
    "security.read": ["security.status", "firewall.status"],
    "network.read": ["network", "wifi.get", "bluetooth.get"],
    "audio.read": ["audio.get", "audio.doctor"],
    "audio.control": ["audio.volume", "audio.mute", "audio.unmute", "audio.reset"],
    "games.read": ["games.doctor"],
    "doctor.read": ["doctor.check"],
    "doctor.fix": ["doctor.fix"],
    "notifications": ["notify"]
  }/*end*/;
  var TOPICS = ["app", "permissions", "progress"];
  var data = { app: {}, calls: {}, permissions: null };

  window.wolkitFixtures = function (f) {
    if (!f || typeof f !== "object") return;
    Object.assign(data.app, f.app || {});
    Object.assign(data.calls, f.calls || {});
    if (Array.isArray(f.permissions)) data.permissions = f.permissions.slice();
  };

  function owner(method) {
    for (var p in CATALOG) if (CATALOG[p].indexOf(method) >= 0) return p;
    return null;
  }

  window.__wolosMock = function () {
    var q = new URLSearchParams(location.search);
    var list = function (k) { return (q.get(k) || "").split(",").filter(Boolean); };
    var declared = data.permissions || Object.keys(CATALOG);
    var deny = list("deny"), fail = list("fail");
    var delay = q.has("delay") ? Number(q.get("delay")) || 0 : 120;
    var dark = window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches;
    var app = Object.assign({ id: "org.wolos.webapp.dev", version: "0.0.0", locale: "en",
      theme: dark ? "dark" : "light", accent: "violet" }, data.app);
    if (q.get("theme")) app.theme = q.get("theme");
    if (q.get("accent")) app.accent = q.get("accent");
    if (q.get("locale")) app.locale = q.get("locale");
    var subs = {};
    var t = {
      init: { app: app, permissions: declared.filter(function (p) { return deny.indexOf(p) < 0; }) },
      onmessage: null,
      post: function (text) {
        var m = JSON.parse(text);
        if (m.sub) { if (TOPICS.indexOf(m.sub) >= 0) subs[m.sub] = true; return; }
        if (m.unsub) { delete subs[m.unsub]; return; }
        var answer = function (body) {
          setTimeout(function () { if (t.onmessage) t.onmessage(JSON.stringify(Object.assign({ id: m.id }, body))); }, delay);
        };
        var event = function (topic, d) { if (subs[topic] && t.onmessage) t.onmessage(JSON.stringify({ topic: topic, data: d })); };
        var p = owner(m.call);
        if (!p) return answer({ err: { code: "not-found", message: "This app asked for something WolOS does not offer.", hint: "unknown method " + m.call } });
        if (declared.indexOf(p) < 0) return answer({ err: { code: "denied", message: "This app did not ask for that permission.", hint: "add \"" + p + "\" to permissions in app.toml" } });
        if (t.init.permissions.indexOf(p) < 0) return answer({ err: { code: "denied", message: "This app is not allowed to do that.", hint: "Turn it on in Settings > Privacy" } });
        if (fail.indexOf(m.call) >= 0) return answer({ err: { code: "failed", message: "That did not work this time.", hint: "Try again in a moment" } });
        var r = data.calls[m.call];
        if (r === undefined) return answer({ err: { code: "unavailable", message: "No recorded answer for this.", hint: "add \"" + m.call + "\" to a fixtures file" } });
        if (r && r.$error) return answer({ err: r.$error });
        event("progress", { method: m.call, done: 0, total: 1, label: "" });
        answer({ ok: JSON.parse(JSON.stringify(r)) });
        setTimeout(function () { event("progress", { method: m.call, done: 1, total: 1, label: "" }); }, delay);
      }
    };
    var send = function (topic, d) { if (subs[topic] && t.onmessage) t.onmessage(JSON.stringify({ topic: topic, data: d })); };
    // Dev helpers for the console: wolosDev.theme("dark"), wolosDev.grant([...]).
    window.wolosDev = {
      theme: function (theme) { app = Object.assign({}, app, { theme: theme }); send("app", app); },
      grant: function (names) { t.init.permissions = names.slice(); send("permissions", names); }
    };
    return t;
  };
})();
