// SPDX-License-Identifier: GPL-2.0-or-later
// Doctor's page logic without a browser (ADR 0048, QA-WEB-12): app.js in a node context whose DOM
// is a set of stand-ins that remember what is set on them. Not a browser: nothing is laid out and
// no focus moves (doctor.test.mjs, with CHROME, does that).
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "app.js"), "utf8");

/** A stand-in element: properties are stored, every method is a no-op that returns another one. */
function stub(name, log) {
  const store = { name, listeners: {}, dataset: {}, children: [] };
  return new Proxy(store, {
    get(t, k) {
      if (k in t) return t[k];
      if (k === "addEventListener") return (ev, f) => { (t.listeners[ev] ||= []).push(f); };
      if (k === "closest" || k === "focus" || k === "setAttribute" && false) return () => null;
      if (typeof k === "symbol") return undefined;
      if (k === "classList") return { contains: () => false };
      return (...a) => stub(`${name}>${String(k)}`, log);
    },
    set(t, k, v) { t[k] = v; log.push([name, k, v]); return true; },
  });
}

function doctor(answer) {
  const log = [];
  const els = new Map();
  const get = (sel) => { if (!els.has(sel)) els.set(sel, stub(sel, log)); return els.get(sel); };
  const winListeners = {};
  const calls = [];
  const win = {
    wolkit: { t: (k) => k, localize() {}, explain: () => "explained" },
    wolos: {
      app: { locale: "en" },
      call: async (m, p) => { calls.push(m); return answer; },
      subscribe() {},
      ready: Promise.resolve(),
    },
    document: {
      querySelector: get,
      querySelectorAll: () => [],
      createElement: (tag) => stub(tag, log),
      addEventListener() {},
      activeElement: null,
      title: "",
    },
    location: { hash: "" },
    addEventListener: (ev, f) => { (winListeners[ev] ||= []).push(f); },
    requestAnimationFrame: (f) => f(),
    CSS: { escape: String },
    Set, Map, Date, Promise, Array, Object, String, Number,
  };
  win.window = win;
  vm.runInNewContext(src, win);
  return { get, log, winListeners, calls, settle: () => new Promise((r) => setTimeout(r, 5)) };
}

test("a report without its list is an error, not a page with nothing on it (QA-WEB-30)", async () => {
  for (const answer of [null, "", {}, { checks: null }, { checks: "x" }, 7]) {
    const d = doctor(answer);
    await d.settle();
    assert.equal(d.get("#error").hidden, false, `shown for ${JSON.stringify(answer)}`);
    assert.equal(d.get("#error-text").textContent, "explained");
    assert.equal(d.get("#fine").hidden, true);
    assert.equal(d.get("#lists").hidden, true);
  }
  const ok = doctor({ checked: 1, ready: true, checks: [] });
  await ok.settle();
  assert.equal(ok.get("#error").hidden, true, "a report with an empty list is a report");
  assert.equal(ok.get("#fine").hidden, false, "and everything is fine");
});

test("a change of the address renders the page once (QA-WEB-17)", async () => {
  const d = doctor({ checked: 1, ready: true, checks: [] });
  await d.settle();
  assert.equal((d.get("#areas").listeners.select || []).length, 0, "the sidebar's select is the same change");
  const titles = () => d.log.filter(([n, k]) => n === "#title" && k === "textContent").length;
  const before = titles();
  for (const f of d.winListeners.hashchange || []) f({});
  assert.equal(titles() - before, 1);
});
