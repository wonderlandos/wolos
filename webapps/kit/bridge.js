// SPDX-License-Identifier: BSD-2-Clause
// window.wolos, the system web app bridge (ADR 0048). Frozen contract:
//   wolos.call(method, params) -> Promise<result>; rejects with an Error carrying {code, message, hint}
//   wolos.subscribe(topic, handler) -> unsubscribe()
//   wolos.app {id, version, locale, theme: "light"|"dark", accent}, wolos.permissions [names], wolos.ready
// The transport is the runtime's window.__wolosHost {init, post(text), onmessage}; in a desktop
// browser with ?dev in the address it is the mock (mock-bridge.js), otherwise every call fails.
(function () {
  "use strict";
  if (window.wolos) return;
  var dev = /[?&]dev(=|&|$)/.test(location.search);
  var t = window.__wolosHost || (dev && window.__wolosMock ? window.__wolosMock() : null);
  var dark = window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches;
  var init = (t && t.init) || {
    app: { id: "", version: "", locale: navigator.language || "en", theme: dark ? "dark" : "light", accent: "violet" },
    permissions: []
  };
  var app = Object.freeze(Object.assign({}, init.app));
  var granted = Object.freeze((init.permissions || []).slice());
  var pending = new Map();
  var subs = new Map();
  var next = 1;
  // The channel drops a message over 64 KiB without a word (crates/webengine/src/engine/channel.rs
  // MAX_MESSAGE), so the page refuses it first; and a call that gets no answer ends in an error
  // instead of a promise that never settles (QA-WEB-6). The host's own wait for wold is 150-180 s
  // (wol_core::sysapp::host_wait_secs); an administrator method waits on a sheet for a person and
  // has no page-side limit (ADMIN is test-checked against the catalog, tests/webapps.rs).
  var MAX_BYTES = 64 * 1024;
  var ADMIN = ["doctor.fix", "update.install"];
  var WATCH_MS = 210000;
  var watch = dev && /[?&]watch=(\d+)/.exec(location.search);
  if (watch) WATCH_MS = Number(watch[1]);

  function bytes(text) {
    try { return unescape(encodeURIComponent(text)).length; } catch (x) { return text.length * 3; }
  }

  function paint() {
    var r = document.documentElement;
    r.dataset.theme = app.theme === "dark" ? "dark" : "light";
    r.dataset.accent = app.accent || "violet";
    if (app.locale) r.lang = app.locale;
  }

  function fault(e) {
    e = e || {};
    var err = new Error(e.message || "Something went wrong.");
    err.code = e.code || "failed";
    err.hint = e.hint || "";
    return err;
  }

  function emit(topic, data) {
    (subs.get(topic) || []).slice().forEach(function (h) {
      try { h(data); } catch (x) { setTimeout(function () { throw x; }); }
    });
  }

  function receive(text) {
    var m;
    try { m = JSON.parse(text); } catch (x) { return; }
    if (typeof m.id === "number" && pending.has(m.id)) {
      var p = pending.get(m.id);
      pending.delete(m.id);
      if (p.timer) clearTimeout(p.timer);
      if ("err" in m) p.reject(fault(m.err)); else p.resolve(m.ok);
    } else if (typeof m.topic === "string") {
      if (m.topic === "app" && m.data) { app = Object.freeze(Object.assign({}, m.data)); paint(); }
      if (m.topic === "permissions" && Array.isArray(m.data)) granted = Object.freeze(m.data.slice());
      emit(m.topic, m.data);
    }
  }

  function send(obj) { t.post(JSON.stringify(obj)); }

  // The page always follows its theme and grants, so these two stay subscribed.
  var ALWAYS = ["app", "permissions"];
  if (t) { t.onmessage = receive; ALWAYS.forEach(function (topic) { send({ sub: topic }); }); }

  function call(method, params) {
    if (!t) {
      return Promise.reject(fault({ code: "unavailable", message: "This app runs inside WolOS.",
        hint: "Open it from the launcher, or add ?dev to the address to use recorded answers" }));
    }
    if (typeof method !== "string" || (params != null && (typeof params !== "object" || Array.isArray(params)))) {
      return Promise.reject(fault({ code: "usage", message: "The request was not understood.",
        hint: "call(method: string, params?: object)" }));
    }
    var text = JSON.stringify({ id: next, call: method, params: params || {} });
    if (bytes(text) > MAX_BYTES) {
      return Promise.reject(fault({ code: "usage", message: "The request was too large.",
        hint: "call(method, params): keep the whole request under 64 KiB" }));
    }
    var id = next++;
    return new Promise(function (resolve, reject) {
      var entry = { resolve: resolve, reject: reject, timer: 0 };
      if (ADMIN.indexOf(method) < 0) {
        entry.timer = setTimeout(function () {
          if (!pending.delete(id)) return;
          reject(fault({ code: "unavailable", message: "WolOS did not answer, so that did not finish.",
            hint: "Try again" }));
        }, WATCH_MS);
      }
      pending.set(id, entry);
      t.post(text);
    });
  }

  function subscribe(topic, handler) {
    if (typeof handler !== "function") throw new TypeError("subscribe(topic, handler)");
    var list = subs.get(topic);
    if (!list) { list = []; subs.set(topic, list); if (t && ALWAYS.indexOf(topic) < 0) send({ sub: topic }); }
    list.push(handler);
    var done = false;
    return function unsubscribe() {
      if (done) return;
      done = true;
      var i = list.indexOf(handler);
      if (i >= 0) list.splice(i, 1);
      if (!list.length) { subs.delete(topic); if (t && ALWAYS.indexOf(topic) < 0) send({ unsub: topic }); }
    };
  }

  // Theme, accent and language before the first frame.
  paint();
  Object.defineProperty(window, "wolos", {
    configurable: false, enumerable: true, writable: false,
    value: Object.freeze({
      call: call,
      subscribe: subscribe,
      get app() { return app; },
      get permissions() { return granted; },
      ready: Promise.resolve(),
      dev: !window.__wolosHost && !!t
    })
  });
})();
