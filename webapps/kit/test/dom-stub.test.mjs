// SPDX-License-Identifier: BSD-2-Clause
// The home-made DOM (design/tools/dom-stub.mjs) held to the browser behaviour the kit tests lean
// on (DS-WEB-1). If a kit test passes only because the stub is generous, a case here is missing.
import test from "node:test";
import assert from "node:assert/strict";
import { createDom } from "../../../design/tools/dom-stub.mjs";

test("markup parses to elements and text, and serializes back", () => {
  const dom = createDom();
  const el = dom.mount('<p class="a b" data-x-y="1">one &amp; <b>two</b><br><input type=search disabled></p>');
  assert.equal(el.localName, "p");
  assert.equal(el.className, "a b");
  assert.equal(el.textContent, "one & two");
  assert.equal(dom.serialize(el), '<p class="a b" data-x-y="1">one &amp; <b>two</b><br><input type="search" disabled=""></p>');
  assert.equal(el.querySelector("input").disabled, true);
  assert.equal(el.querySelector("input").type, "search");
});

test("selectors: tag, id, class, attribute operators, combinators, :scope, lists", () => {
  const dom = createDom();
  const root = dom.mount('<div id="r"><a href="#x" class="k">1</a><section><a href="#y" data-t="hi there">2</a><i></i></section></div>');
  assert.equal(root.querySelectorAll("a").length, 2);
  assert.equal(root.querySelectorAll(":scope > a").length, 1, "child only");
  assert.equal(root.querySelectorAll("div a").length, 2, "selectors match against the whole tree, as in a browser: the scope itself is a div");
  assert.equal(root.querySelectorAll("section > div").length, 0);
  assert.equal(root.querySelectorAll("section a").length, 1);
  assert.equal(root.querySelector('[data-t~="there"]').textContent, "2");
  assert.equal(root.querySelector('a[href^="#y"]').textContent, "2");
  assert.equal(root.querySelectorAll("i, .k").length, 2);
  assert.equal(root.querySelector(".k").closest("#r"), root);
  assert.equal(dom.document.getElementById("r"), root);
  assert.throws(() => root.querySelector("a:hover"), /unsupported pseudo-class/);
  assert.throws(() => root.querySelector("a >"), /dangling combinator/);
});

test("events bubble to the document and window; stopPropagation and preventDefault work", () => {
  const dom = createDom();
  const [outer] = [dom.mount('<div><button id="b">x</button></div>')];
  const seen = [];
  dom.window.addEventListener("click", () => seen.push("window"));
  dom.document.addEventListener("click", () => seen.push("document"));
  outer.addEventListener("click", (e) => seen.push("div:" + e.target.id + ":" + e.currentTarget.localName));
  dom.click(outer.querySelector("button"));
  assert.deepEqual(seen, ["div:b:div", "document", "window"]);
  seen.length = 0;
  outer.addEventListener("click", (e) => e.stopPropagation());
  dom.click(outer.querySelector("button"));
  assert.deepEqual(seen, ["div:b:div"], "stopped at the div");
  const btn = outer.querySelector("button");
  btn.disabled = true;
  seen.length = 0;
  dom.click(btn);
  assert.deepEqual(seen, [], "a disabled button takes no click");
});

test("a link click changes the hash and fires hashchange; the default can be prevented", () => {
  const dom = createDom();
  const a = dom.mount('<a href="#one">one</a>');
  let n = 0;
  dom.window.addEventListener("hashchange", () => n++);
  dom.click(a);
  assert.equal(dom.location.hash, "#one");
  assert.equal(n, 1);
  dom.click(a);
  assert.equal(n, 1, "same hash: no event");
  const b = dom.mount('<a href="#two">two</a>');
  b.addEventListener("click", (e) => e.preventDefault());
  dom.click(b);
  assert.equal(dom.location.hash, "#one", "prevented");
  assert.equal(a.hash, "#one");
});

test("custom elements: construct, upgrade on connect, observed attributes only, disconnect", () => {
  const dom = createDom();
  const log = [];
  dom.load(`
    class X extends HTMLElement {
      static observedAttributes = ["a"];
      connectedCallback() { window.log.push("connected:" + this.childNodes.length); }
      disconnectedCallback() { window.log.push("disconnected"); }
      attributeChangedCallback(k, o, n) { window.log.push(["attr", k, o, n].join(",")); }
    }
    customElements.define("x-el", X);
  `);
  dom.window.log = log;
  const el = dom.mount('<x-el a="1" b="2"><i></i></x-el>');
  assert.deepEqual(log, ["attr,a,,1", "connected:1"], "an upgrade sees its children and its attributes first");
  el.setAttribute("b", "3");
  el.setAttribute("a", "9");
  assert.equal(log[2], "attr,a,1,9", "b is not observed");
  el.remove();
  assert.equal(log.at(-1), "disconnected");
  assert.equal(el.isConnected, false);
  const made = dom.document.createElement("x-el");
  assert.ok(made instanceof dom.window.customElements.get("x-el"));
  assert.throws(() => dom.load("customElements.define('x-el', class extends HTMLElement {})"), /already defined/);
  assert.throws(() => dom.load("customElements.define('nodash', class extends HTMLElement {})"), /not a valid custom element name/);
  assert.throws(() => dom.load("new HTMLElement()"), /Illegal constructor/);
});

test("an element defined after it is in the page upgrades at once", () => {
  const dom = createDom();
  const el = dom.mount("<late-el>hi</late-el>");
  assert.equal(el.constructor.name, "Plain");
  dom.load("window.n = 0; customElements.define('late-el', class extends HTMLElement { connectedCallback() { window.n++; } })");
  assert.equal(dom.window.n, 1);
});

test("moving a connected element disconnects then connects it; focus is lost when it leaves", () => {
  const dom = createDom();
  dom.load(`window.log = []; customElements.define('m-el', class extends HTMLElement {
    connectedCallback() { log.push('c'); } disconnectedCallback() { log.push('d'); } })`);
  const box = dom.mount('<div><m-el></m-el></div><div id="other"></div>');
  dom.window.log.length = 0;
  box[1].append(box[0].firstChild);
  assert.deepEqual([...dom.window.log], ["d", "c"]);
  const btn = dom.mount("<button>x</button>");
  btn.focus();
  assert.equal(dom.document.activeElement, btn);
  btn.remove();
  assert.equal(dom.document.activeElement, dom.document.body);
});

test("focus: only focusable things take it, never a disabled control", () => {
  const dom = createDom();
  const [d, b, bd, inp, a, ti] = dom.mount('<div></div><button>b</button><button disabled>d</button><input><a href="#x">a</a><span tabindex="-1">t</span>');
  for (const el of [d, bd]) { el.focus(); assert.notEqual(dom.document.activeElement, el, el.localName); }
  for (const el of [b, inp, a, ti]) { el.focus(); assert.equal(dom.document.activeElement, el, el.localName); }
  assert.equal(b.tabIndex, 0);
  assert.equal(d.tabIndex, -1);
  assert.equal(ti.tabIndex, -1);
});

test("keys: Enter and Space click a button, Enter follows a link, Escape cancels an open modal dialog", () => {
  const dom = createDom();
  const [btn, dlg] = dom.mount("<button>go</button><dialog></dialog>");
  let clicks = 0;
  btn.addEventListener("click", () => clicks++);
  btn.focus();
  dom.key("Enter");
  dom.key(" ");
  assert.equal(clicks, 2);
  btn.addEventListener("keydown", (e) => e.preventDefault());
  dom.key("Enter");
  assert.equal(clicks, 2, "a prevented keydown has no default action");
  dlg.showModal();
  assert.equal(dlg.open, true);
  dom.key("Escape", { on: dlg });
  assert.equal(dlg.open, false);
  dlg.showModal();
  dlg.addEventListener("cancel", (e) => e.preventDefault());
  dom.key("Escape", { on: dlg });
  assert.equal(dlg.open, true, "cancel prevented");
});

test("dataset maps data-* attributes both ways and lists its keys", () => {
  const dom = createDom();
  const el = dom.mount('<p data-t="a" data-t-aria-label="b"></p>');
  assert.deepEqual(Object.entries(el.dataset), [["t", "a"], ["tAriaLabel", "b"]]);
  el.dataset.fooBar = "1";
  assert.equal(el.getAttribute("data-foo-bar"), "1");
  assert.equal(el.dataset.missing, undefined);
});

test("style writes land in the style attribute; textContent replaces children; hidden is an attribute", () => {
  const dom = createDom();
  const el = dom.mount("<div><b>x</b></div>");
  el.style.width = "40%";
  assert.equal(el.getAttribute("style"), "width: 40%");
  el.style.width = "";
  assert.equal(el.hasAttribute("style"), false);
  el.textContent = "plain";
  assert.equal(el.childNodes.length, 1);
  el.textContent = "";
  assert.equal(el.childNodes.length, 0);
  el.hidden = true;
  assert.equal(el.getAttribute("hidden"), "");
  assert.equal(dom.text(dom.mount("<p>a <i hidden>x</i> b</p>")), "a b");
});

test("typing fires input per character; a prevented keydown types nothing", async () => {
  const dom = createDom();
  const inp = dom.mount("<input type=search>");
  const values = [];
  inp.addEventListener("input", () => values.push(inp.value));
  dom.type(inp, "abc");
  assert.deepEqual(values, ["a", "ab", "abc"]);
  inp.addEventListener("keydown", (e) => e.preventDefault());
  dom.type(inp, "d");
  assert.equal(inp.value, "abc");
  let ran = false;
  dom.window.queueMicrotask(() => { ran = true; });
  await dom.settle();
  assert.equal(ran, true);
});
