// SPDX-License-Identifier: BSD-2-Clause
// The eleven elements of wolkit.js, one by one, on the home-made DOM (design/tools/dom-stub.mjs;
// DS-WEB-1, ADR 0058): what each builds, what its attributes do, what a key or a click does, what
// it announces. Not a browser: layout, the cascade and real focus order are checked in the
// browser test of an app (webapps/doctor/test/doctor.test.mjs, CHROME), the look by the shell
// goldens. Every case names the element it is about.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createDom } from "../../../design/tools/dom-stub.mjs";

const SRC = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "wolkit.js"), "utf8");

/** A page with the kit loaded, as `<script src=wolkit.js defer>` leaves it. */
function page() {
  const dom = createDom();
  dom.load(SRC, "wolkit.js");
  return dom;
}
const q = (el, sel) => el.querySelector(sel);

test("kit: all eleven elements are defined, once, and a second load does not redefine", () => {
  const dom = page();
  const names = ["wol-app", "wol-sidebar", "wol-list", "wol-row", "wol-switch", "wol-button", "wol-badge", "wol-progress", "wol-empty", "wol-search", "wol-sheet"];
  for (const n of names) assert.ok(dom.window.customElements.get(n), n);
  dom.load(SRC, "wolkit-again.js"); // would throw "already defined" without the guard
  assert.throws(() => (dom.window.wolkit.t = 1) && null, TypeError, "window.wolkit is frozen") ;
});

// ---- wol-app ----
test("wol-app: a skip link leads to main, which becomes a focus target", () => {
  const dom = page();
  const app = dom.mount('<wol-app><wol-sidebar label="S"><a href="#a">A</a></wol-sidebar><main>words</main></wol-app>');
  const skip = app.firstChild;
  assert.equal(skip.localName, "a");
  assert.equal(skip.className, "wk-skip");
  assert.equal(skip.textContent, "Skip to content");
  const main = q(app, "main");
  assert.equal(main.id, "content");
  assert.equal(skip.getAttribute("href"), "#content");
  assert.equal(main.tabIndex, -1);
  assert.equal(app.classList.contains("wk-solo"), false, "has a sidebar");
});

test("wol-app: main keeps its own id; no sidebar is solo; no main is no skip link", () => {
  const dom = page();
  const solo = dom.mount('<wol-app><main id="mine"></main></wol-app>');
  assert.equal(q(solo, "main").id, "mine");
  assert.equal(q(solo, ".wk-skip").getAttribute("href"), "#mine");
  assert.ok(solo.classList.contains("wk-solo"));
  const bare = dom.mount("<wol-app><p>x</p></wol-app>");
  assert.equal(q(bare, ".wk-skip"), null);
});

test("wol-app: connecting the same app twice builds once", () => {
  const dom = page();
  const app = dom.mount("<wol-app><main></main></wol-app>");
  const host = app.parentNode;
  app.remove();
  host.append(app);
  assert.equal(app.querySelectorAll(".wk-skip").length, 1);
});

// ---- wol-sidebar ----
const SIDEBAR = '<wol-sidebar label="Sections"><a href="#sound">Sound</a><a href="#net">Network</a><a href="#power">Power</a></wol-sidebar>';

test("wol-sidebar: links become the items of a nav named by the label; the first is current", async () => {
  const dom = page();
  const sb = dom.mount(SIDEBAR);
  await dom.settle();
  const nav = q(sb, "nav");
  const title = q(sb, ".wk-sidebar-title");
  assert.equal(title.textContent, "Sections");
  assert.equal(title.hidden, false);
  assert.equal(nav.getAttribute("aria-labelledby"), title.id);
  assert.deepEqual([...sb.items().map((a) => a.textContent)], ["Sound", "Network", "Power"]);
  assert.equal(sb.value, "sound");
  assert.deepEqual([...sb.items().map((a) => a.getAttribute("aria-current"))], ["page", null, null]);
});

test("wol-sidebar: without a label the title is hidden; the label can change", async () => {
  const dom = page();
  const sb = dom.mount(SIDEBAR.replace(' label="Sections"', ""));
  await dom.settle();
  assert.equal(q(sb, ".wk-sidebar-title").hidden, true);
  sb.setAttribute("label", "Pages");
  assert.equal(q(sb, ".wk-sidebar-title").textContent, "Pages");
  assert.equal(q(sb, ".wk-sidebar-title").hidden, false);
});

test("wol-sidebar: the address picks the item, a click on a link moves it, select fires once per change", async () => {
  const dom = page();
  dom.location.hash = "#net";
  const sb = dom.mount(SIDEBAR);
  const picks = [];
  sb.addEventListener("select", (e) => picks.push(e.detail.value));
  await dom.settle();
  assert.equal(sb.value, "net", "the address at start");
  dom.click(sb.items()[2]);
  assert.equal(sb.value, "power");
  assert.deepEqual(picks, ["net", "power"]);
  assert.equal(sb.items()[1].hasAttribute("aria-current"), false, "only one is current");
});

// The skip link (#content) or any in-page anchor is no item: the current one stays (found by this
// test; select() used to clear aria-current before it knew the value was an item).
test("wol-sidebar: an address that is no item leaves the current item as it was", async () => {
  const dom = page();
  const sb = dom.mount(SIDEBAR);
  await dom.settle();
  dom.click(sb.items()[1]);
  dom.location.hash = "#content";
  assert.equal(sb.value, "net");
});

test("wol-sidebar: Up and Down (and Left and Right) move focus between the items and wrap", async () => {
  const dom = page();
  const sb = dom.mount(SIDEBAR);
  await dom.settle();
  const [a, b, c] = sb.items();
  a.focus();
  dom.key("ArrowDown");
  assert.equal(dom.document.activeElement, b);
  dom.key("ArrowRight");
  assert.equal(dom.document.activeElement, c);
  dom.key("ArrowDown");
  assert.equal(dom.document.activeElement, a, "wraps to the first");
  dom.key("ArrowUp");
  assert.equal(dom.document.activeElement, c, "wraps to the last");
  dom.key("ArrowLeft");
  assert.equal(dom.document.activeElement, b);
  const ev = [];
  sb.addEventListener("keydown", (e) => ev.push(e.defaultPrevented));
  dom.key("ArrowDown");
  dom.key("a");
  assert.deepEqual(ev, [true, false], "arrows are taken, other keys are left alone");
});

test("wol-sidebar: leaving the page stops listening to the address", async () => {
  const dom = page();
  const sb = dom.mount(SIDEBAR);
  await dom.settle();
  sb.remove();
  dom.location.hash = "#power";
  assert.equal(sb.value, "sound");
});

// ---- wol-list ----
test("wol-list: a heading names the rows' list; rows keep their order; a note goes under", () => {
  const dom = page();
  const list = dom.mount('<wol-list heading="Checks" note="Last checked now"><wol-row label="A"></wol-row><wol-row label="B"></wol-row></wol-list>');
  const h = q(list, "h2");
  const body = q(list, ".wk-list-body");
  assert.equal(h.textContent, "Checks");
  assert.equal(body.getAttribute("role"), "list");
  assert.equal(body.getAttribute("aria-labelledby"), h.id);
  assert.deepEqual(body.children.map((r) => r.getAttribute("label")), ["A", "B"]);
  assert.equal(list.rows, body);
  const foot = q(list, ".wk-list-foot");
  assert.equal(foot.textContent, "Last checked now");
  assert.equal(foot.hidden, false);
  assert.equal(q(list, ".wk-list-head").hidden, false);
});

test("wol-list: without a heading or note both stay hidden; setting them later shows them", () => {
  const dom = page();
  const list = dom.mount("<wol-list><wol-row label='A'></wol-row></wol-list>");
  assert.equal(q(list, ".wk-list-head").hidden, true);
  assert.equal(q(list, ".wk-list-foot").hidden, true);
  list.setAttribute("heading", "Later");
  list.setAttribute("note", "Note");
  assert.equal(q(list, ".wk-list-head").hidden, false);
  assert.equal(q(list, "h2").textContent, "Later");
  assert.equal(q(list, ".wk-list-foot").hidden, false);
  list.removeAttribute("note");
  assert.equal(q(list, ".wk-list-foot").hidden, true);
});

test("wol-list: a slot=head child sits in the head; replaceRows swaps the rows", () => {
  const dom = page();
  const list = dom.mount('<wol-list heading="Checks"><wol-button slot="head">Run</wol-button><wol-row label="A"></wol-row></wol-list>');
  const head = q(list, ".wk-list-head");
  assert.deepEqual(head.children.map((c) => c.localName), ["h2", "wol-button"]);
  assert.deepEqual(q(list, ".wk-list-body").children.map((c) => c.localName), ["wol-row"], "the head child is not a row");
  const row = dom.document.createElement("wol-row");
  row.setAttribute("label", "New");
  list.replaceRows(row);
  assert.deepEqual(list.rows.children.map((c) => c.getAttribute("label")), ["New"]);
});

// ---- wol-row ----
test("wol-row: a list item with label and detail; the detail is hidden when empty", () => {
  const dom = page();
  const row = dom.mount('<wol-row label="Sound service" detail="Running"></wol-row>');
  assert.equal(row.getAttribute("role"), "listitem");
  assert.equal(q(row, ".wk-row-label").textContent, "Sound service");
  assert.equal(q(row, ".wk-row-detail").textContent, "Running");
  assert.equal(q(row, ".wk-row-detail").hidden, false);
  row.setAttribute("detail", "");
  assert.equal(q(row, ".wk-row-detail").hidden, true);
  row.removeAttribute("label");
  assert.equal(q(row, ".wk-row-label").textContent, "");
});

test("wol-row: slot=text goes under the label, everything else at the end", () => {
  const dom = page();
  const row = dom.mount('<wol-row label="L"><small slot="text">more</small><wol-badge></wol-badge></wol-row>');
  assert.equal(q(row, ".wk-row-text small").textContent, "more");
  assert.equal(q(row, ".wk-row-end wol-badge").localName, "wol-badge");
  assert.equal(q(row, ".wk-row-end small"), null);
});

test("wol-row: without a link it is not a control", () => {
  const dom = page();
  const row = dom.mount('<wol-row label="Plain" detail="d"></wol-row>');
  assert.equal(row.hasAttribute("tabindex"), false);
  assert.equal(row.hasAttribute("aria-label"), false);
  row.focus();
  assert.notEqual(dom.document.activeElement, row);
});

test("wol-row: a link row is one focusable control, named by label and detail; a click or Enter opens it", () => {
  const dom = page();
  const row = dom.mount('<wol-row label="Network" detail="Online" href="#net"></wol-row>');
  assert.equal(row.tabIndex, 0);
  assert.equal(row.getAttribute("aria-label"), "Network, Online");
  row.focus();
  assert.equal(dom.document.activeElement, row);
  dom.click(row);
  assert.equal(dom.navigations.at(-1), `${dom.location.href.split("#")[0]}#net`);
  dom.location.hash = "#";
  dom.key("Enter");
  assert.equal(dom.location.hash, "#net", "Enter on the focused row");
  row.removeAttribute("href");
  assert.equal(row.hasAttribute("tabindex"), false);
});

test("wol-row: a button, link or switch inside a link row does its own job, the row does not open", () => {
  const dom = page();
  const row = dom.mount('<wol-row label="Row" href="#row"><button id="b">Do</button><wol-switch></wol-switch></wol-row>');
  dom.click(q(row, "#b"));
  assert.equal(dom.location.hash, "", "button");
  dom.click(q(row, "wol-switch"));
  assert.equal(dom.location.hash, "", "switch");
  assert.equal(q(row, "wol-switch").checked, true);
  q(row, "#b").focus();
  dom.key("Enter");
  assert.equal(dom.location.hash, "", "Enter on a child is the child's");
});

// ---- wol-switch ----
test("wol-switch: role, state and focus follow the attributes", () => {
  const dom = page();
  const sw = dom.mount('<wol-switch label="Reduce noise" checked></wol-switch>');
  assert.equal(sw.getAttribute("role"), "switch");
  assert.equal(sw.getAttribute("aria-checked"), "true");
  assert.equal(sw.getAttribute("aria-disabled"), "false");
  assert.equal(sw.getAttribute("aria-label"), "Reduce noise");
  assert.equal(sw.tabIndex, 0);
  sw.checked = false;
  assert.equal(sw.getAttribute("aria-checked"), "false");
  sw.disabled = true;
  assert.equal(sw.getAttribute("aria-disabled"), "true");
  assert.equal(sw.tabIndex, -1);
  assert.equal(sw.hasAttribute("checked"), false);
});

test("wol-switch: click, Space and Enter flip it and fire change, which bubbles", () => {
  const dom = page();
  const row = dom.mount('<wol-row label="Row"><wol-switch></wol-switch></wol-row>');
  const sw = q(row, "wol-switch");
  const seen = [];
  row.addEventListener("change", (e) => seen.push([e.target.localName, e.target.checked]));
  dom.click(sw);
  sw.focus();
  dom.key(" ");
  dom.key("Enter");
  dom.key("x");
  assert.deepEqual(seen, [["wol-switch", true], ["wol-switch", false], ["wol-switch", true]]);
  assert.equal(sw.getAttribute("aria-checked"), "true");
});

test("wol-switch: Space is taken (no page scroll); a disabled switch does nothing", () => {
  const dom = page();
  const sw = dom.mount("<wol-switch disabled></wol-switch>");
  let changes = 0;
  sw.addEventListener("change", () => changes++);
  let prevented = null;
  sw.addEventListener("keydown", (e) => { prevented = e.defaultPrevented; });
  dom.key(" ", { on: sw });
  dom.click(sw);
  assert.equal(changes, 0);
  assert.equal(sw.checked, false);
  assert.equal(prevented, true, "the key is still taken, so the page does not scroll");
});

test("wol-switch: in a row it takes the row's label, its own label wins, and a late row label arrives", () => {
  const dom = page();
  const row = dom.mount('<wol-row><wol-switch></wol-switch></wol-row>');
  const sw = q(row, "wol-switch");
  assert.equal(sw.hasAttribute("aria-label"), false);
  row.setAttribute("label", "Show details");
  assert.equal(sw.getAttribute("aria-label"), "Show details");
  sw.setAttribute("label", "Mine");
  assert.equal(sw.getAttribute("aria-label"), "Mine");
});

// ---- wol-button ----
test("wol-button: wraps a real button that holds the content", () => {
  const dom = page();
  const b = dom.mount('<wol-button kind="primary" type="submit">Fix <b>it</b></wol-button>');
  assert.equal(b.btn.localName, "button");
  assert.equal(b.btn.type, "submit");
  assert.equal(b.btn.textContent, "Fix it");
  assert.equal(b.btn.getAttribute("aria-busy"), "false");
  assert.equal(b.btn.disabled, false);
  assert.equal(dom.mount("<wol-button>x</wol-button>").btn.type, "button", "never submits a form by accident");
});

test("wol-button: disabled and busy both disable the inner button; busy is announced", () => {
  const dom = page();
  const b = dom.mount("<wol-button>Fix</wol-button>");
  b.setAttribute("disabled", "");
  assert.equal(b.btn.disabled, true);
  assert.equal(b.btn.getAttribute("aria-busy"), "false");
  b.removeAttribute("disabled");
  b.busy = true;
  assert.equal(b.btn.disabled, true);
  assert.equal(b.btn.getAttribute("aria-busy"), "true");
  assert.equal(b.hasAttribute("busy"), true);
  b.busy = false;
  assert.equal(b.btn.disabled, false);
  assert.equal(b.btn.getAttribute("aria-busy"), "false");
});

test("wol-button: a label names it; clicks reach the host; a disabled one gets none; focus goes to the button", () => {
  const dom = page();
  const b = dom.mount('<wol-button label="Fix sound">Fix</wol-button>');
  assert.equal(b.btn.getAttribute("aria-label"), "Fix sound");
  b.removeAttribute("label");
  assert.equal(b.btn.hasAttribute("aria-label"), false);
  let clicks = 0;
  b.addEventListener("click", () => clicks++);
  dom.click(b.btn);
  assert.equal(clicks, 1, "the click bubbles from the button to the element");
  b.btn.focus();
  dom.key("Enter");
  assert.equal(clicks, 2);
  b.setAttribute("disabled", "");
  dom.click(b.btn);
  assert.equal(clicks, 2);
  b.btn.blur();
  b.removeAttribute("disabled");
  b.focus();
  assert.equal(dom.document.activeElement, b.btn);
});

// ---- wol-badge ----
test("wol-badge: each state has words of its own, never colour alone", () => {
  const dom = page();
  const words = { ok: "OK", warn: "Needs attention", fail: "Problem", info: "Info" };
  for (const [state, text] of Object.entries(words)) {
    const b = dom.mount(`<wol-badge state="${state}"></wol-badge>`);
    assert.equal(b.textContent, text, state);
  }
});

test("wol-badge: no or unknown state is info; own text wins and can be replaced; the state can change", () => {
  const dom = page();
  assert.equal(dom.mount("<wol-badge></wol-badge>").textContent, "Info");
  assert.equal(dom.mount('<wol-badge state="loud"></wol-badge>').textContent, "Info");
  const b = dom.mount('<wol-badge state="ok"> Fine </wol-badge>');
  assert.equal(b.textContent, "Fine");
  b.setAttribute("state", "fail");
  assert.equal(b.textContent, "Fine", "own words stay");
  b.text = "Broken";
  assert.equal(b.textContent, "Broken");
  const c = dom.mount('<wol-badge state="ok"></wol-badge>');
  c.setAttribute("state", "warn");
  assert.equal(c.textContent, "Needs attention");
});

// ---- wol-progress ----
test("wol-progress: a progressbar from 0 to 100 with a name; a value sets now and width in percent", () => {
  const dom = page();
  const p = dom.mount('<wol-progress value="0.4" label="Downloading"></wol-progress>');
  assert.equal(p.getAttribute("role"), "progressbar");
  assert.equal(p.getAttribute("aria-valuemin"), "0");
  assert.equal(p.getAttribute("aria-valuemax"), "100");
  assert.equal(p.getAttribute("aria-label"), "Downloading");
  assert.equal(p.getAttribute("aria-valuenow"), "40");
  assert.equal(q(p, "span").style.width, "40%");
});

test("wol-progress: no value is working with no length, no label is 'Working'; values are clamped", () => {
  const dom = page();
  const p = dom.mount("<wol-progress></wol-progress>");
  assert.equal(p.getAttribute("aria-label"), "Working");
  assert.equal(p.hasAttribute("aria-valuenow"), false);
  assert.equal(q(p, "span").style.width, "");
  for (const [v, now] of [[2, "100"], [-1, "0"], ["junk", "0"], [0.005, "1"], [1, "100"]]) {
    p.value = v;
    assert.equal(p.getAttribute("aria-valuenow"), now, String(v));
  }
  p.value = null;
  assert.equal(p.hasAttribute("aria-valuenow"), false, "back to unknown length");
  assert.equal(q(p, "span").style.width, "");
});

// ---- wol-empty ----
test("wol-empty: a status with a heading above the words; the heading follows the attribute", () => {
  const dom = page();
  const e = dom.mount('<wol-empty heading="Nothing to fix">All good. <wol-button>Run again</wol-button></wol-empty>');
  assert.equal(e.getAttribute("role"), "status");
  const h = e.firstChild;
  assert.equal(h.className, "wk-empty-heading");
  assert.equal(h.textContent, "Nothing to fix");
  assert.match(e.textContent, /All good\./);
  assert.ok(q(e, "wol-button"));
  e.setAttribute("heading", "Fixed");
  assert.equal(h.textContent, "Fixed");
  e.removeAttribute("heading");
  assert.equal(h.textContent, "");
});

// ---- wol-search ----
test("wol-search: a search landmark with a labelled input; label and placeholder follow the attributes", () => {
  const dom = page();
  const s = dom.mount("<wol-search></wol-search>");
  assert.equal(s.getAttribute("role"), "search");
  assert.equal(s.input.type, "search");
  assert.equal(s.input.autocomplete, "off");
  assert.equal(s.input.spellcheck, false);
  assert.equal(s.input.getAttribute("aria-label"), "Search");
  assert.equal(s.input.placeholder, "Search");
  s.setAttribute("label", "Search checks");
  s.setAttribute("placeholder", "Name of a check");
  assert.equal(s.input.getAttribute("aria-label"), "Search checks");
  assert.equal(s.input.placeholder, "Name of a check");
  s.removeAttribute("placeholder");
  assert.equal(s.input.placeholder, "Search checks", "the label stands in");
});

test("wol-search: typing fires search for each change with the trimmed value; value reads the field", () => {
  const dom = page();
  const s = dom.mount("<wol-search></wol-search>");
  const got = [];
  s.addEventListener("search", (e) => got.push(e.detail.value));
  dom.type(s.input, "so ");
  dom.type(s.input, "x");
  assert.deepEqual(got, ["s", "so", "so", "so x"]);
  assert.equal(s.value, "so x");
});

test("wol-search: Escape clears a filled field and stays inside; on an empty field it passes on", () => {
  const dom = page();
  const host = dom.mount("<div><wol-search></wol-search></div>");
  const s = q(host, "wol-search");
  const got = [];
  const outer = [];
  s.addEventListener("search", (e) => got.push(e.detail.value));
  host.addEventListener("keydown", (e) => { if (e.key === "Escape") outer.push("escape"); });
  dom.type(s.input, "net");
  got.length = 0;
  dom.key("Escape", { on: s.input });
  assert.equal(s.value, "");
  assert.deepEqual(got, [""], "the list is told it is cleared");
  assert.deepEqual(outer, [], "a surrounding sheet does not also close");
  dom.key("Escape", { on: s.input });
  assert.deepEqual(outer, ["escape"], "nothing to clear: Escape goes on");
});

test("wol-search: '/' outside a field focuses it; in a field, with a modifier, or in a contenteditable it does not", () => {
  const dom = page();
  const [s, other, editable] = dom.mount('<wol-search></wol-search><input id="o"><div contenteditable="true" tabindex="0">x</div>');
  const slash = (mods) => {
    const ev = new dom.window.KeyboardEvent("keydown", { key: "/", ...mods });
    dom.document.activeElement.dispatchEvent(ev);
    return ev.defaultPrevented;
  };
  assert.equal(slash({}), true);
  assert.equal(dom.document.activeElement, s.input);
  s.input.blur();
  other.focus();
  assert.equal(slash({}), false);
  assert.equal(dom.document.activeElement, other);
  editable.focus();
  assert.equal(slash({}), false, "contenteditable");
  dom.document.activeElement.blur();
  for (const m of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }]) {
    assert.equal(slash(m), false, JSON.stringify(m));
    assert.notEqual(dom.document.activeElement, s.input);
  }
});

test("wol-search: after it leaves the page '/' is no longer its key", () => {
  const dom = page();
  const s = dom.mount("<wol-search></wol-search>");
  s.remove();
  const ev = new dom.window.KeyboardEvent("keydown", { key: "/" });
  dom.document.body.dispatchEvent(ev);
  assert.equal(ev.defaultPrevented, false);
});

// ---- wol-sheet ----
const SHEET = '<wol-sheet heading="Reset sound?" confirm="Reset" cancel="Keep">Settings will be lost.</wol-sheet>';

test("wol-sheet: an alert dialog named by its heading and described by its words, two buttons", () => {
  const dom = page();
  const s = dom.mount(SHEET);
  const dlg = s.dlg;
  assert.equal(dlg.getAttribute("role"), "alertdialog");
  const h = q(dlg, "h2");
  assert.equal(h.textContent, "Reset sound?");
  assert.equal(dlg.getAttribute("aria-labelledby"), h.id);
  const body = q(dlg, ".wk-sheet-body");
  assert.equal(dlg.getAttribute("aria-describedby"), body.id);
  assert.equal(body.textContent, "Settings will be lost.");
  assert.equal(s.no.btn.textContent, "Keep");
  assert.equal(s.yes.btn.textContent, "Reset");
  assert.equal(s.yes.getAttribute("kind"), "primary");
  assert.equal(dlg.open, false);
});

test("wol-sheet: default words are Cancel and Continue; danger makes the confirm danger", () => {
  const dom = page();
  const s = dom.mount('<wol-sheet heading="Sure?"></wol-sheet>');
  assert.equal(s.no.btn.textContent, "Cancel");
  assert.equal(s.yes.btn.textContent, "Continue");
  s.setAttribute("danger", "");
  assert.equal(s.yes.getAttribute("kind"), "danger");
  s.removeAttribute("danger");
  assert.equal(s.yes.getAttribute("kind"), "primary");
});

test("wol-sheet: open focuses Confirm, a click on it answers true and focus goes back", async () => {
  const dom = page();
  const [opener, s] = dom.mount(`<button id="o">Reset</button>${SHEET}`);
  opener.focus();
  const answer = s.open();
  assert.equal(s.dlg.open, true);
  assert.equal(dom.document.activeElement, s.yes.btn);
  const closed = [];
  s.addEventListener("close", (e) => closed.push(e.detail.answer));
  dom.click(s.yes.btn);
  assert.equal(await answer, true);
  assert.equal(s.dlg.open, false);
  assert.equal(dom.document.activeElement, opener, "focus comes back");
  assert.deepEqual(closed, [true]);
});

test("wol-sheet: a danger sheet starts on Cancel, so Enter does not destroy anything", async () => {
  const dom = page();
  const s = dom.mount(SHEET.replace("<wol-sheet", "<wol-sheet danger"));
  const answer = s.open();
  assert.equal(dom.document.activeElement, s.no.btn);
  dom.key("Enter");
  assert.equal(await answer, false);
});

test("wol-sheet: Cancel and Escape answer false; closing twice answers once", async () => {
  const dom = page();
  const s = dom.mount(SHEET);
  let answer = s.open();
  dom.click(s.no.btn);
  assert.equal(await answer, false);
  answer = s.open();
  dom.key("Escape", { on: s.dlg });
  assert.equal(await answer, false);
  assert.equal(s.dlg.open, false, "Escape closed it through cancel");
  const closed = [];
  s.addEventListener("close", (e) => closed.push(e.detail.answer));
  answer = s.open();
  s.close(true);
  s.close(false);
  assert.equal(await answer, true);
  assert.deepEqual(closed, [true], "the second close is ignored");
});

test("wol-sheet: opening an open sheet gives the same promise (QA-WEB-16)", async () => {
  const dom = page();
  const s = dom.mount(SHEET);
  const a = s.open();
  assert.equal(s.open(), a);
  s.close(true);
  assert.equal(await a, true);
  const b = s.open();
  assert.notEqual(b, a);
  s.close(false);
  assert.equal(await b, false);
});

test("confirmSheet: a one-off sheet that is gone again after the answer", async () => {
  const dom = page();
  const answer = dom.window.wolkit.confirmSheet({ heading: "Delete?", body: "No undo.", confirm: "Delete", cancel: "Keep", danger: true });
  const sheet = q(dom.document.body, "wol-sheet");
  assert.ok(sheet, "in the page while open");
  assert.equal(sheet.getAttribute("heading"), "Delete?");
  assert.equal(sheet.hasAttribute("danger"), true);
  assert.equal(sheet.yes.btn.textContent, "Delete");
  assert.equal(q(sheet, ".wk-sheet-body").textContent, "No undo.");
  dom.click(sheet.yes.btn);
  assert.equal(await answer, true);
  assert.equal(q(dom.document.body, "wol-sheet"), null, "removed");
});

// ---- the helpers the elements use ----
test("t: the app's table, then the kit's, then the key; placeholders are filled", () => {
  const dom = page();
  const { t } = dom.window.wolkit;
  assert.equal(t("sheet.cancel"), "Cancel");
  assert.equal(t("no.such.key"), "no.such.key");
  dom.window.WOLKIT_STRINGS = { "sheet.cancel": "Keep it", "mine": "Hello {name}, {n} left, {missing}" };
  assert.equal(t("sheet.cancel"), "Keep it", "the app overrides the kit");
  assert.equal(t("mine", { name: "Ana", n: 3 }), "Hello Ana, 3 left, {missing}");
  assert.equal(dom.mount('<wol-badge state="ok"></wol-badge>').textContent, "OK", "kit strings the app does not override remain");
});

test("localize: data-t sets the text, data-t-<attr> sets an attribute", () => {
  const dom = page();
  dom.window.WOLKIT_STRINGS = { greet: "Hello", tip: "A tip", ph: "Type here" };
  const root = dom.mount('<div><p data-t="greet"></p><wol-row data-t-label="tip"></wol-row><input data-t-placeholder="ph" data-t-aria-label="tip"></div>');
  dom.window.wolkit.localize(root);
  assert.equal(q(root, "p").textContent, "Hello");
  assert.equal(q(root, "wol-row").getAttribute("label"), "A tip");
  assert.equal(q(root, "input").getAttribute("placeholder"), "Type here");
  assert.equal(q(root, "input").getAttribute("aria-label"), "A tip");
  assert.equal(q(root, "wol-row .wk-row-label").textContent, "A tip", "the element follows the attribute set after it upgraded");
});

test("explain: the message, then a hint that is a sentence; developer hints and non-bridge errors stay out", () => {
  const dom = page();
  const { explain } = dom.window.wolkit;
  assert.equal(explain(null), "");
  assert.equal(explain({ code: "denied", message: "Not allowed.", hint: "Ask an administrator" }), "Not allowed. Ask an administrator.");
  assert.equal(explain({ code: "denied", message: "Not allowed.", hint: "check app.toml" }), "Not allowed.");
  assert.equal(explain({ code: "denied", message: "", hint: "" }), "Something went wrong.");
  assert.equal(explain(new Error("TypeError: x is undefined")), "Something went wrong.");
});
