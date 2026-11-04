// SPDX-License-Identifier: BSD-2-Clause
// wolkit.js without a browser (ADR 0048, QA-WEB-12): the parts of the kit that are logic, in a
// node context with a very small stand-in for the DOM (elements with attributes, children, a
// custom element registry). It is not a browser: layout, focus and the dialog are not here (the
// browser test of an app, webapps/doctor/test/doctor.test.mjs, runs those with CHROME).
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "wolkit.js"), "utf8");

function connectTree(n) {
  if (!n || typeof n !== "object") return;
  if (n.connectedCallback && !n.isConnected) { n.isConnected = true; n.connectedCallback(); }
  for (const c of n.children || []) connectTree(c);
}

class Node_ {
  constructor() { this.children = []; this.parent = null; this.attrs = new Map(); this.listeners = {}; this.dataset = {}; this.hidden = false; this._text = ""; }
  get childNodes() { return this.children; }
  get slot() { return this.attrs.get("slot") || ""; }
  get textContent() { return this._text + this.children.map((c) => c.textContent ?? c).join(""); }
  set textContent(v) { this._text = String(v); this.children = []; }
  append(...kids) {
    for (const k of kids) { if (k && k.parent) k.parent.children.splice(k.parent.children.indexOf(k), 1); if (k && typeof k === "object") k.parent = this; this.children.push(k); }
    // In a document, an element that is added is connected at once (custom elements upgrade).
    if (this.isConnected) for (const k of kids) connectTree(k);
  }
  prepend(...kids) { this.append(...kids); }
  replaceChildren(...kids) { this.children = []; this.append(...kids); }
  remove() { if (this.parent) this.parent.children.splice(this.parent.children.indexOf(this), 1); }
  setAttribute(k, v) { const old = this.attrs.get(k); this.attrs.set(k, String(v)); this.changed(k, old); }
  getAttribute(k) { return this.attrs.has(k) ? this.attrs.get(k) : null; }
  hasAttribute(k) { return this.attrs.has(k); }
  removeAttribute(k) { const old = this.attrs.get(k); this.attrs.delete(k); this.changed(k, old); }
  toggleAttribute(k, on) { if (on) this.setAttribute(k, ""); else this.removeAttribute(k); }
  changed(k, old) { const c = this.constructor; if (c.observedAttributes && c.observedAttributes.includes(k) && this.attributeChangedCallback) this.attributeChangedCallback(k, old, this.attrs.get(k)); }
  addEventListener(ev, f) { (this.listeners[ev] ||= []).push(f); }
  dispatchEvent(e) { for (const f of this.listeners[e.type] || []) f(e); return true; }
  closest(sel) { for (let n = this; n; n = n.parent) if (n.localName === sel) return n; return null; }
  descendants() { return this.children.flatMap((c) => (c.descendants ? [c, ...c.descendants()] : [])); }
  querySelectorAll(sel) { return this.descendants().filter((n) => n.localName === sel); }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
  focus() { this.focused = true; }
  set tabIndex(v) { this.attrs.set("tabindex", String(v)); }
  get tabIndex() { return Number(this.attrs.get("tabindex") ?? -1); }
  set className(v) { this.attrs.set("class", v); }
  get className() { return this.attrs.get("class") || ""; }
}

function kit() {
  const registry = new Map();
  const dialogs = [];
  const document = {
    activeElement: null,
    body: new Node_(),
    createElement(tag) {
      const n = new Node_();
      n.localName = tag;
      if (tag === "dialog") {
        n.open = false;
        n.showModal = () => { n.open = true; };
        n.close = () => { n.open = false; };
        dialogs.push(n);
      }
      return n;
    },
    querySelectorAll() { return []; },
  };
  const ctx = {
    document,
    HTMLElement: Node_,
    customElements: { get: (n) => registry.get(n), define: (n, c) => { registry.set(n, c); c.prototype.localName = n; } },
    CustomEvent: class { constructor(type, init) { this.type = type; this.detail = init && init.detail; } },
    Event: class { constructor(type) { this.type = type; } },
    location: { href: "" },
    queueMicrotask: () => {},
    Object, Array, String, Map, Promise, RegExp, Error,
  };
  ctx.window = ctx;
  vm.runInNewContext(src, ctx);
  const make = (name, attrs = {}) => {
    const e = new (registry.get(name))();
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    return e;
  };
  document.body.isConnected = true;
  return { ctx, make, document, dialogs, wolkit: ctx.wolkit };
}

test("explain: a hint's own full stop is not doubled (QA-WEB-14)", () => {
  const { wolkit } = kit();
  assert.equal(wolkit.explain({ code: "failed", message: "That did not work.", hint: "Try again." }), "That did not work. Try again.");
  assert.equal(wolkit.explain({ code: "failed", message: "That did not work.", hint: "Try again" }), "That did not work. Try again.");
  assert.equal(wolkit.explain({ code: "failed", message: "That did not work.", hint: "Try again..  " }), "That did not work. Try again.");
  assert.equal(wolkit.explain({ code: "failed", message: "No.", hint: "turn it on" }), "No.", "a developer hint stays out");
  assert.equal(wolkit.explain(new Error("x")), "Something went wrong.");
});

test("a link row is named by its label and its detail (QA-WEB-21)", () => {
  const { make } = kit();
  const row = make("wol-row", { label: "Sound", detail: "Running", href: "#sound" });
  row.connectedCallback();
  assert.equal(row.getAttribute("aria-label"), "Sound, Running");
  row.setAttribute("detail", "");
  assert.equal(row.getAttribute("aria-label"), "Sound");
  row.removeAttribute("href");
  assert.equal(row.getAttribute("aria-label"), null, "no link: read as the text it is");
});

test("a switch in a row takes the row's label when it is set after the switch upgraded (QA-WEB-29)", () => {
  const { make } = kit();
  const row = make("wol-row");
  const sw = make("wol-switch");
  row.append(sw);
  row.connectedCallback();
  sw.connectedCallback();
  assert.equal(sw.getAttribute("aria-label"), null, "no label yet");
  row.setAttribute("label", "Show details");
  assert.equal(sw.getAttribute("aria-label"), "Show details");
  // The switch's own label wins.
  sw.setAttribute("label", "Mine");
  assert.equal(sw.getAttribute("aria-label"), "Mine");
});

test("opening a sheet that is open gives the same answer, not a second promise (QA-WEB-16)", async () => {
  const { make, document } = kit();
  const sheet = make("wol-sheet", { heading: "Sure?" });
  document.body.append(sheet);
  const a = sheet.open();
  const b = sheet.open();
  assert.equal(a, b, "one promise while open");
  sheet.close(true);
  assert.equal(await a, true);
  const c = sheet.open();
  assert.notEqual(c, a, "a new one after it closed");
  sheet.close(false);
  assert.equal(await c, false);
});
