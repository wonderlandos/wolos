// SPDX-License-Identifier: BSD-2-Clause
// Wolkit for system web apps (ADR 0048): the WolOS components as custom elements in the light DOM,
// styled by wolkit.css. No dependencies, no build step. Every visible kit string is in STRINGS and
// an app overrides or adds its own with window.WOLKIT_STRINGS (one table per app, strings.js).
// A classic script loaded with defer (file: pages cannot load modules in every browser); the
// helpers are window.wolkit: t, localize, explain, confirmSheet.
(() => {
"use strict";
const STRINGS = {
  "badge.ok": "OK",
  "badge.warn": "Needs attention",
  "badge.fail": "Problem",
  "badge.info": "Info",
  "sheet.confirm": "Continue",
  "sheet.cancel": "Cancel",
  "search.label": "Search",
  "skip": "Skip to content",
  "progress.label": "Working",
  "error.generic": "Something went wrong.",
};

/** The string for `key` from the app's table, then the kit's; {name} placeholders filled from `vars`. */
function t(key, vars) {
  const table = window.WOLKIT_STRINGS || {};
  const s = key in table ? table[key] : key in STRINGS ? STRINGS[key] : key;
  return vars ? s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m)) : s;
}

/** Fills the page from the string table: [data-t] sets the text, data-t-<attr>="key" sets <attr>
 * (data-t-heading, data-t-label, data-t-placeholder, data-t-aria-label). */
function localize(root = document) {
  for (const e of root.querySelectorAll("*")) {
    for (const [k, key] of Object.entries(e.dataset)) {
      if (k === "t") e.textContent = t(key);
      else if (/^t[A-Z]/.test(k)) e.setAttribute(k.slice(1).replace(/[A-Z]/g, (c) => "-" + c.toLowerCase()).slice(1), t(key));
    }
  }
}

const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
};
let uid = 0;
const nextId = (p) => `${p}-${++uid}`;

/** Base: renders once when connected, again when an observed attribute changes. */
class Base extends HTMLElement {
  connectedCallback() { if (!this._built) { this._built = true; this.build(); } this.update(); }
  attributeChangedCallback() { if (this._built) this.update(); }
  build() {}
  update() {}
}

/* <wol-app>: sidebar + main, with a skip link to main. */
class WolApp extends Base {
  build() {
    const main = this.querySelector(":scope > main");
    if (main && !main.id) main.id = "content";
    if (main) { main.tabIndex = -1; const a = el("a", "wk-skip", t("skip")); a.href = "#" + main.id; this.prepend(a); }
    if (!this.querySelector(":scope > wol-sidebar")) this.classList.add("wk-solo");
  }
}

/* <wol-sidebar label="Sections"><a href="#sound">Sound</a>...</wol-sidebar>
   Links are the items (hash routing works without script); the current one gets aria-current and
   a "select" event fires with {value}. Up and Down move between items. */
class WolSidebar extends Base {
  static observedAttributes = ["label", "value"];
  build() {
    const nav = el("nav");
    const links = [...this.querySelectorAll(":scope > a")];
    nav.append(...links);
    this.title_ = el("div", "wk-sidebar-title");
    this.title_.id = nextId("wk-side");
    nav.setAttribute("aria-labelledby", this.title_.id);
    this.append(this.title_, nav);
    this.nav = nav;
    nav.addEventListener("keydown", (e) => {
      const items = this.items();
      const i = items.indexOf(document.activeElement);
      const step = { ArrowDown: 1, ArrowUp: -1, ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (i < 0 || !step) return;
      e.preventDefault();
      items[(i + step + items.length) % items.length].focus();
    });
    this._hash = () => this.select(location.hash.slice(1) || (this.items()[0] && this.items()[0].hash.slice(1)));
    window.addEventListener("hashchange", this._hash);
    queueMicrotask(this._hash);
  }
  disconnectedCallback() { window.removeEventListener("hashchange", this._hash); }
  items() { return [...this.nav.querySelectorAll("a")]; }
  update() {
    this.title_.textContent = this.getAttribute("label") || "";
    this.title_.hidden = !this.getAttribute("label");
  }
  get value() { const a = this.nav && this.nav.querySelector('[aria-current="page"]'); return a ? a.hash.slice(1) : ""; }
  select(value) {
    if (!value) return;
    let hit = false;
    for (const a of this.items()) {
      const on = a.hash.slice(1) === value;
      hit ||= on;
      if (on) a.setAttribute("aria-current", "page"); else a.removeAttribute("aria-current");
    }
    if (hit) this.dispatchEvent(new CustomEvent("select", { detail: { value }, bubbles: true }));
  }
}

/* <wol-list heading="Checks" note="..."> rows </wol-list>: a grouped card with a heading. */
class WolList extends Base {
  static observedAttributes = ["heading", "note"];
  build() {
    this.head = el("div", "wk-list-head");
    this.h = el("h2");
    this.h.id = nextId("wk-list");
    const extra = [...this.querySelectorAll(':scope > [slot="head"]')];
    this.head.append(this.h, ...extra);
    this.body = el("div", "wk-list-body");
    this.body.setAttribute("role", "list");
    this.body.setAttribute("aria-labelledby", this.h.id);
    this.body.append(...[...this.childNodes].filter((n) => n !== this.head && !extra.includes(n)));
    this.foot = el("p", "wk-list-foot");
    this.append(this.head, this.body, this.foot);
  }
  update() {
    const h = this.getAttribute("heading");
    this.h.textContent = h || "";
    this.head.hidden = !h;
    const n = this.getAttribute("note");
    this.foot.textContent = n || "";
    this.foot.hidden = !n;
  }
  /** Where rows go when a page adds them later. */
  get rows() { return this.body; }
  replaceRows(...rows) { this.body.replaceChildren(...rows); }
}

/* <wol-row label="Sound service" detail="Running"> end content </wol-row>
   Children with slot="text" go under the label and detail, the rest at the end of the row.
   With href it is one link (Enter or click opens it). */
class WolRow extends Base {
  static observedAttributes = ["label", "detail", "href"];
  build() {
    this.setAttribute("role", "listitem");
    this.text = el("div", "wk-row-text");
    this.lab = el("span", "wk-row-label");
    this.det = el("span", "wk-row-detail");
    this.text.append(this.lab, this.det);
    const end = el("div", "wk-row-end");
    const kids = [...this.childNodes];
    this.text.append(...kids.filter((n) => n.slot === "text"));
    end.append(...kids.filter((n) => n.slot !== "text"));
    this.append(this.text, end);
    this.addEventListener("click", (e) => { if (this.hasAttribute("href") && !e.target.closest("button,a,wol-switch")) location.href = this.getAttribute("href"); });
    this.addEventListener("keydown", (e) => { if (e.key === "Enter" && e.target === this && this.hasAttribute("href")) location.href = this.getAttribute("href"); });
  }
  update() {
    this.lab.textContent = this.getAttribute("label") || "";
    const d = this.getAttribute("detail");
    this.det.textContent = d || "";
    this.det.hidden = !d;
    // A link row is one control: its name carries the detail too, or a screen reader never
    // hears it (QA-WEB-21). Without a link the label and detail are read as the text they are.
    if (this.hasAttribute("href")) {
      this.tabIndex = 0;
      this.setAttribute("aria-label", d ? `${this.lab.textContent}, ${d}` : this.lab.textContent);
    } else { this.removeAttribute("tabindex"); this.removeAttribute("aria-label"); }
    // A switch in the row is named by the row's label, which may be set after the switch
    // upgraded (data-t-label is localised later, QA-WEB-29).
    for (const s of this.querySelectorAll("wol-switch")) if (s.update) s.update();
  }
}

/* <wol-switch checked label="Reduce noise">: role switch, Space or Enter flips it, "change" fires. */
class WolSwitch extends Base {
  static observedAttributes = ["checked", "disabled", "label"];
  build() {
    this.setAttribute("role", "switch");
    this.addEventListener("click", () => this.flip());
    this.addEventListener("keydown", (e) => { if (e.key === " " || e.key === "Enter") { e.preventDefault(); this.flip(); } });
  }
  update() {
    this.setAttribute("aria-checked", String(this.checked));
    this.setAttribute("aria-disabled", String(this.disabled));
    this.tabIndex = this.disabled ? -1 : 0;
    const row = this.closest("wol-row");
    const label = this.getAttribute("label") || (row && row.getAttribute("label"));
    if (label) this.setAttribute("aria-label", label);
  }
  get checked() { return this.hasAttribute("checked"); }
  set checked(v) { this.toggleAttribute("checked", !!v); }
  get disabled() { return this.hasAttribute("disabled"); }
  set disabled(v) { this.toggleAttribute("disabled", !!v); }
  flip() {
    if (this.disabled) return;
    this.checked = !this.checked;
    this.dispatchEvent(new Event("change", { bubbles: true }));
  }
}

/* <wol-button kind="primary|quiet|danger" busy disabled>Fix</wol-button>: a real <button> inside. */
class WolButton extends Base {
  static observedAttributes = ["disabled", "busy", "label", "type"];
  build() {
    this.btn = el("button");
    this.btn.type = this.getAttribute("type") || "button";
    this.btn.append(...this.childNodes);
    this.append(this.btn);
  }
  update() {
    this.btn.disabled = this.hasAttribute("disabled") || this.hasAttribute("busy");
    this.btn.setAttribute("aria-busy", String(this.hasAttribute("busy")));
    const l = this.getAttribute("label");
    if (l) this.btn.setAttribute("aria-label", l); else this.btn.removeAttribute("aria-label");
  }
  focus() { this.btn.focus(); }
  get busy() { return this.hasAttribute("busy"); }
  set busy(v) { this.toggleAttribute("busy", !!v); }
}

/* <wol-badge state="ok|warn|fail|info">text</wol-badge>: colour, shape and words, never colour alone. */
class WolBadge extends Base {
  static observedAttributes = ["state"];
  build() {
    this.txt = el("span", "wk-badge-text");
    this.own = this.textContent.trim();
    this.replaceChildren(this.txt);
  }
  update() {
    const s = ["ok", "warn", "fail", "info"].includes(this.getAttribute("state")) ? this.getAttribute("state") : "info";
    this.txt.textContent = this.own || t("badge." + s);
  }
  set text(v) { this.own = v; this.update(); }
}

/* <wol-progress value="0.4" label="Downloading">: no value is "working, length unknown". */
class WolProgress extends Base {
  static observedAttributes = ["value", "label"];
  build() {
    this.setAttribute("role", "progressbar");
    this.setAttribute("aria-valuemin", "0");
    this.setAttribute("aria-valuemax", "100");
    this.bar = el("span");
    this.replaceChildren(this.bar);
  }
  update() {
    this.setAttribute("aria-label", this.getAttribute("label") || t("progress.label"));
    const v = this.getAttribute("value");
    if (v == null) { this.removeAttribute("aria-valuenow"); this.bar.style.width = ""; return; }
    const pct = Math.round(Math.min(1, Math.max(0, Number(v) || 0)) * 100);
    this.setAttribute("aria-valuenow", String(pct));
    this.bar.style.width = pct + "%";
  }
  set value(v) { if (v == null) this.removeAttribute("value"); else this.setAttribute("value", String(v)); }
}

/* <wol-empty heading="Nothing to fix">More words. <wol-button>…</wol-button></wol-empty> */
class WolEmpty extends Base {
  static observedAttributes = ["heading"];
  build() { this.setAttribute("role", "status"); this.h = el("p", "wk-empty-heading"); this.prepend(this.h); }
  update() { this.h.textContent = this.getAttribute("heading") || ""; }
}

/* <wol-search label="Search checks" placeholder="Search">: "search" fires with {value} as you type;
   "/" anywhere outside a field focuses it, Escape clears it. */
class WolSearch extends Base {
  static observedAttributes = ["label", "placeholder"];
  build() {
    this.setAttribute("role", "search");
    this.input = el("input");
    this.input.type = "search";
    this.input.autocomplete = "off";
    this.input.spellcheck = false;
    this.append(this.input);
    this.input.addEventListener("input", () => this.fire());
    this.input.addEventListener("keydown", (e) => { if (e.key === "Escape" && this.input.value) { e.stopPropagation(); this.input.value = ""; this.fire(); } });
    this._key = (e) => {
      if (e.key !== "/" || e.ctrlKey || e.metaKey || e.altKey) return;
      const a = document.activeElement;
      if (a && (a.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(a.tagName))) return;
      e.preventDefault();
      this.input.focus();
    };
    document.addEventListener("keydown", this._key);
  }
  disconnectedCallback() { document.removeEventListener("keydown", this._key); }
  update() {
    const l = this.getAttribute("label") || t("search.label");
    this.input.setAttribute("aria-label", l);
    this.input.placeholder = this.getAttribute("placeholder") || l;
  }
  get value() { return this.input ? this.input.value : ""; }
  fire() { this.dispatchEvent(new CustomEvent("search", { detail: { value: this.input.value.trim() }, bubbles: true })); }
}

/* <wol-sheet heading="Reset sound?" confirm="Reset" cancel="Cancel" danger>Body</wol-sheet>
   sheet.open() -> Promise<boolean>. A modal dialog: focus moves in and comes back, Escape is Cancel.
   For the system's own approval (administrator actions) wold shows its sheet, not this one. */
class WolSheet extends Base {
  static observedAttributes = ["heading", "confirm", "cancel", "danger"];
  build() {
    this.dlg = el("dialog");
    this.dlg.setAttribute("role", "alertdialog");
    this.h = el("h2");
    this.h.id = nextId("wk-sheet");
    this.body = el("div", "wk-sheet-body");
    this.body.id = nextId("wk-sheet-body");
    this.body.append(...this.childNodes);
    this.dlg.setAttribute("aria-labelledby", this.h.id);
    this.dlg.setAttribute("aria-describedby", this.body.id);
    const actions = el("div", "wk-sheet-actions");
    this.no = new WolButton();
    this.yes = new WolButton();
    this.yes.setAttribute("kind", "primary");
    actions.append(this.no, this.yes);
    this.dlg.append(this.h, this.body, actions);
    this.append(this.dlg);
    this.no.addEventListener("click", () => this.close(false));
    this.yes.addEventListener("click", () => this.close(true));
    this.dlg.addEventListener("cancel", (e) => { e.preventDefault(); this.close(false); });
  }
  update() {
    this.h.textContent = this.getAttribute("heading") || "";
    this.no.btn.textContent = this.getAttribute("cancel") || t("sheet.cancel");
    this.yes.btn.textContent = this.getAttribute("confirm") || t("sheet.confirm");
    this.yes.setAttribute("kind", this.hasAttribute("danger") ? "danger" : "primary");
  }
  open() {
    // Open already: the same answer, not a second promise that nothing would ever settle (QA-WEB-16).
    if (this.dlg.open && this.opened) return this.opened;
    this.back = document.activeElement;
    this.dlg.showModal();
    (this.hasAttribute("danger") ? this.no : this.yes).focus();
    this.opened = new Promise((r) => { this.done = r; });
    return this.opened;
  }
  close(answer) {
    if (!this.dlg.open) return;
    this.opened = null;
    this.dlg.close();
    if (this.back && this.back.focus) this.back.focus();
    const d = this.done;
    this.done = null;
    if (d) d(answer);
    this.dispatchEvent(new CustomEvent("close", { detail: { answer } }));
  }
}

/** One-off confirm: await confirmSheet({heading, body, confirm, cancel, danger}). */
function confirmSheet({ heading, body = "", confirm, cancel, danger = false }) {
  const s = new WolSheet();
  s.setAttribute("heading", heading);
  if (confirm) s.setAttribute("confirm", confirm);
  if (cancel) s.setAttribute("cancel", cancel);
  if (danger) s.setAttribute("danger", "");
  s.append(body);
  document.body.append(s);
  return s.open().finally(() => s.remove());
}

/** A bridge error as one line for a person: its message, then what to do (hints in lower case are
 * for developers and stay out). Anything that is not a bridge error is the generic line. */
function explain(err) {
  if (!err) return "";
  if (!err.code) return t("error.generic");
  // The hint may carry its own full stop: one is enough (QA-WEB-14).
  return err.hint && /^[A-Z]/.test(err.hint) ? `${err.message} ${err.hint.replace(/[.\s]+$/, "")}.` : err.message || t("error.generic");
}

const DEFS = { "wol-app": WolApp, "wol-sidebar": WolSidebar, "wol-list": WolList, "wol-row": WolRow, "wol-switch": WolSwitch,
  "wol-button": WolButton, "wol-badge": WolBadge, "wol-progress": WolProgress, "wol-empty": WolEmpty, "wol-search": WolSearch,
  "wol-sheet": WolSheet };
for (const [name, cls] of Object.entries(DEFS)) if (!customElements.get(name)) customElements.define(name, cls);
window.wolkit = Object.freeze({ t, localize, explain, confirmSheet });
})();
