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
  // MAX_MESSAGE) and wold takes a call line of 60 KiB less room for the verb, which is what the
  // host reads (bridge::MAX_REQUEST, kept equal by tests/webapps.rs), so the page refuses first; and a call that gets no answer ends in an error
  // instead of a promise that never settles (QA-WEB-6). The host's own wait for wold is 150-180 s
  // (wol_core::sysapp::host_wait_secs); an administrator method waits on a sheet for a person and
  // has no page-side limit (ADMIN is test-checked against the catalog, tests/webapps.rs).
  var MAX_BYTES = 60928;
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
    // The words on the page are English until an app ships a table for the locale, so the page
    // says so: a screen reader must not read English with a Portuguese voice (QA-WEB-22).
    r.lang = window.WOLKIT_LANG || "en";
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
      var p = finish(m.id);
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

  // The host runs at most 8 calls of one page at once and answers the rest "busy"
  // (crates/webengine/src/host.rs MAX_IN_FLIGHT; the two lanes add up to it, tests/webapps.rs). The
  // page keeps two lanes so that neither starves the other: the reads wait their turn here (a
  // ninth Doctor check is not an error, QA-WEB-35), and an administrator method, which waits for a
  // person at a sheet with no time limit, is never queued behind others: beyond its lane it is
  // answered "busy" at once, as the host would, so reads never wait behind a sheet nobody answers.
  var READ_LANE = 4;
  var ADMIN_LANE = 4;
  var running = 0;
  var runningAdmin = 0;
  var waiting = [];

  function wellFormed(s) {
    if (s.isWellFormed) return s.isWellFormed();
    return !/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/.test(s);
  }

  // The request as text, or a usage error: a lone surrogate is written as an escape the host
  // cannot read (QA-WEB-27), a BigInt, a cycle or a throwing getter used to escape as a raw
  // TypeError (QA-WEB-28), and a Date would go as a string the page did not mean.
  function encode(obj) {
    try {
      return JSON.stringify(obj, function (k, v) {
        if (Object.prototype.toString.call(this[k]) === "[object Date]") throw 0;
        if ((typeof v === "string" && !wellFormed(v)) || !wellFormed(k)) throw 0;
        return v;
      });
    } catch (x) {
      throw fault({ code: "usage", message: "The request was not understood.",
        hint: "params: plain objects, arrays, numbers, booleans and text only" });
    }
  }

  function isAdmin(method) { return ADMIN.indexOf(method) >= 0; }

  function finish(id) {
    var job = pending.get(id);
    if (!job || !pending.delete(id)) return null;
    if (job.timer) clearTimeout(job.timer);
    if (isAdmin(job.method)) runningAdmin--; else running--;
    pump();
    return job;
  }

  // Posts a job (it holds a lane slot from here). A channel that throws gives the slot back.
  function start(job) {
    if (isAdmin(job.method)) runningAdmin++; else running++;
    pending.set(job.id, job);
    try {
      t.post(job.text);
    } catch (x) {
      finish(job.id);
      job.reject(fault({ code: "unavailable", message: "WolOS did not get that request.", hint: "Try again" }));
    }
  }

  function pump() {
    while (running < READ_LANE && waiting.length) start(waiting.shift());
  }

  // The time limit of a read starts when it is asked, queued or not (an administrator method has
  // none: a person takes their time at the sheet).
  function expire(job) {
    var i = waiting.indexOf(job);
    if (i >= 0) waiting.splice(i, 1); else if (!finish(job.id)) return;
    job.reject(fault({ code: "unavailable", message: "WolOS did not answer, so that did not finish.",
      hint: "Try again" }));
  }

  function call(method, params) {
    if (!t) {
      return Promise.reject(fault({ code: "unavailable", message: "This app runs inside WolOS.",
        hint: "Open it from the launcher, or add ?dev to the address to use recorded answers" }));
    }
    if (typeof method !== "string" || (params != null && (typeof params !== "object" || Array.isArray(params)))) {
      return Promise.reject(fault({ code: "usage", message: "The request was not understood.",
        hint: "call(method: string, params?: object)" }));
    }
    var text;
    try { text = encode({ id: next, call: method, params: params || {} }); } catch (x) { return Promise.reject(x); }
    if (bytes(text) > MAX_BYTES) {
      return Promise.reject(fault({ code: "usage", message: "The request was too large.",
        hint: "call(method, params): keep the whole request under 60 KiB" }));
    }
    if (isAdmin(method) && runningAdmin >= ADMIN_LANE) {
      return Promise.reject(fault({ code: "unavailable",
        message: "WolOS is busy with other requests from this app. Try again in a moment.",
        hint: "too many calls in flight" }));
    }
    var id = next++;
    return new Promise(function (resolve, reject) {
      var job = { id: id, method: method, text: text, resolve: resolve, reject: reject, timer: 0 };
      if (!isAdmin(method)) job.timer = setTimeout(function () { expire(job); }, WATCH_MS);
      if (isAdmin(method)) start(job); else { waiting.push(job); pump(); }
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
