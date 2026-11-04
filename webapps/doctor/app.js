// SPDX-License-Identifier: GPL-2.0-or-later
// Doctor (ADR 0048): the checks of `wol doctor --json` by area, each row opens to its plain
// detail and a Fix button (`doctor.fix`), with "Check again" and an empty state when all is fine.
"use strict";
{
const { t, localize, explain } = window.wolkit;

const AREAS = ["sound", "games", "updates", "security", "storage", "network", "power", "display"];
const RANK = { fail: 0, warn: 1, ok: 2, skipped: 3 };
const BADGE = { ok: "ok", warn: "warn", fail: "fail", skipped: "info" };

const $ = (s) => document.querySelector(s);
const state = {
  data: null,          // {checked, ready, checks}
  open: new Set(),     // expanded check ids
  busy: new Set(),     // fixes running
  results: new Map(),  // id -> {ok, text}
  showFine: false,
  loading: false,
};

const view = () => {
  const h = location.hash.slice(1);
  return h === "all" || AREAS.includes(h) ? h : "all";
};
const issue = (c) => c.status === "fail" || c.status === "warn";
const domId = (id) => "c-" + id.replace(/[^a-z0-9-]/gi, "-");

function make(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function timeOf(secs) {
  const d = new Date(secs * 1000);
  return d.toLocaleTimeString((window.wolos && wolos.app.locale) || undefined, { hour: "2-digit", minute: "2-digit" });
}

function summaryOf(checks) {
  const fail = checks.filter((c) => c.status === "fail").length;
  const warn = checks.filter((c) => c.status === "warn").length;
  const parts = [];
  if (fail) parts.push(fail === 1 ? t("summary.fail.one") : t("summary.fail.many", { n: fail }));
  if (warn) parts.push(warn === 1 ? t("summary.warn.one") : t("summary.warn.many", { n: warn }));
  return parts.length ? parts.join(" ") : t("summary.fine");
}

function announce(text) {
  const a = $("#announce");
  a.textContent = "";
  requestAnimationFrame(() => { a.textContent = text; });
}

// ---- One check ----

function commandBlock(c) {
  const d = make("details", "dr-cmd");
  d.append(make("summary", null, t("command.show")));
  if (c.fix) {
    d.append(make("p", null, t("command.fix")), make("code", null, `wol doctor fix ${c.id}`), make("code", null, c.fix.command));
  } else {
    d.append(make("p", null, t("command.check")), make("code", null, `wol doctor ${c.area}`));
  }
  return d;
}

function checkRow(c) {
  const id = domId(c.id);
  const open = state.open.has(c.id);
  const busy = state.busy.has(c.id);
  const wrap = make("div", "dr-check");
  wrap.setAttribute("role", "listitem");
  wrap.dataset.id = c.id;
  wrap.dataset.status = c.status;

  const head = make("button", "dr-head");
  head.type = "button";
  head.id = id + "-head";
  head.setAttribute("aria-expanded", String(open));
  head.setAttribute("aria-controls", id + "-panel");
  const badge = document.createElement("wol-badge");
  badge.setAttribute("state", BADGE[c.status] || "info");
  badge.textContent = t("status." + c.status);
  head.append(make("span", "dr-title", c.title), badge, make("span", "dr-chev"));
  head.querySelector(".dr-chev").setAttribute("aria-hidden", "true");
  head.addEventListener("click", () => toggle(c.id));

  const panel = make("div", "dr-panel");
  panel.id = id + "-panel";
  panel.setAttribute("role", "region");
  panel.setAttribute("aria-labelledby", head.id);
  panel.hidden = !open;
  panel.append(make("p", "dr-detail", c.detail));
  if (c.fix && c.status !== "ok") {
    const actions = make("div", "dr-actions");
    const btn = document.createElement("wol-button");
    btn.setAttribute("kind", "primary");
    btn.dataset.fix = c.id;
    btn.append(make("span", null, c.fix.title));
    if (busy) btn.setAttribute("busy", "");
    btn.addEventListener("click", () => runFix(c));
    actions.append(btn);
    if (c.fix.needs_admin) actions.append(make("span", "dr-note", t("fix.admin")));
    panel.append(actions);
    const bar = document.createElement("wol-progress");
    bar.setAttribute("label", t("fix.progress", { title: c.title }));
    bar.dataset.progress = c.id;
    bar.hidden = !busy;
    panel.append(bar);
  }
  const r = state.results.get(c.id);
  const res = make("p", "dr-result", busy ? t("fix.working") : r ? r.text : "");
  res.setAttribute("role", "status");
  if (r && !busy) res.dataset.ok = String(r.ok);
  panel.append(res, commandBlock(c));
  wrap.append(head, panel);
  return wrap;
}

// ---- The page ----

function render() {
  const v = view();
  const all = (state.data && state.data.checks) || [];
  $("#title").textContent = t("area." + v);
  $("#lead").textContent = t("lead." + v);
  document.title = v === "all" ? t("app.title") : `${t("area." + v)} – ${t("app.title")}`;

  // Sidebar counts: what needs a look, per area and in total.
  for (const b of document.querySelectorAll("wol-badge[data-count]")) {
    const a = b.dataset.count;
    const mine = all.filter((c) => (a === "all" || c.area === a) && issue(c));
    const link = b.closest("a");
    b.hidden = mine.length === 0;
    b.setAttribute("state", mine.some((c) => c.status === "fail") ? "fail" : "warn");
    b.text = String(mine.length);
    if (mine.length) link.setAttribute("aria-label", t("nav.count", { area: t("area." + a), n: mine.length }));
    else link.removeAttribute("aria-label");
  }

  const shown = all.filter((c) => v === "all" || c.area === v);
  const fine = !!state.data && !shown.some(issue);
  const when = state.data ? t("checked.at", { time: timeOf(state.data.checked) }) : "";
  $("#summary").textContent = state.data && !fine ? `${summaryOf(shown)} ${when}` : "";
  $("#fine").hidden = !fine;
  $("#fine").setAttribute("heading", t("empty.heading"));
  $("#fine-time").textContent = when;
  $("#fine-toggle").querySelector("span").textContent = state.showFine ? t("empty.hide") : t("empty.show");

  // Keep focus on the same control across the re-render.
  const f = document.activeElement;
  const keep = f && f.closest && f.closest(".dr-check") ? { id: f.closest(".dr-check").dataset.id, fix: !!f.closest("[data-fix]") } : null;

  const lists = $("#lists");
  lists.hidden = fine && !state.showFine;
  const groups = v === "all" ? AREAS : [v];
  const out = [];
  for (const a of groups) {
    const rows = shown.filter((c) => c.area === a).sort((x, y) => RANK[x.status] - RANK[y.status]);
    if (!rows.length) continue;
    const list = document.createElement("wol-list");
    if (v === "all") list.setAttribute("heading", t("area." + a));
    out.push(list);
    list.append(...rows.map(checkRow));
  }
  lists.replaceChildren(...out);

  if (keep) {
    const w = lists.querySelector(`.dr-check[data-id="${CSS.escape(keep.id)}"]`);
    const target = w && (keep.fix ? w.querySelector("[data-fix] button") : null) || (w && w.querySelector(".dr-head"));
    if (target) target.focus();
  }
}

function toggle(id) {
  if (state.open.has(id)) state.open.delete(id); else state.open.add(id);
  render();
}

function replaceCheck(c) {
  if (!state.data || !c) return;
  const i = state.data.checks.findIndex((x) => x.id === c.id);
  if (i >= 0) state.data.checks[i] = c;
}

// The check's header, the one control of a row that stays enabled while a fix runs.
function headOf(id) { return document.querySelector(`.dr-check[data-id="${CSS.escape(id)}"] .dr-head`); }

async function runFix(c) {
  if (state.busy.has(c.id)) return;
  // A busy button is disabled, and a disabled control drops the keyboard to the page body: take
  // the person's place to the row's header first, and give it back there when the fix is over
  // (QA-WEB-25).
  const had = document.activeElement && document.activeElement.closest && document.activeElement.closest(`.dr-check[data-id="${CSS.escape(c.id)}"]`);
  if (had && headOf(c.id)) headOf(c.id).focus();
  state.busy.add(c.id);
  state.results.delete(c.id);
  render();
  announce(t("fix.working"));
  let res;
  try {
    const r = await wolos.call("doctor.fix", { id: c.id });
    const after = r && r.check && r.check.id === c.id ? r.check : null;
    if (r && r.ok) res = { ok: true, text: t("fix.done") };
    else if (r && !r.ran) res = { ok: false, text: t("fix.nothing") };
    else res = { ok: false, text: [t("fix.notyet"), after ? after.detail : ""].filter(Boolean).join(" ") };
    replaceCheck(after);
  } catch (e) {
    res = { ok: false, text: explain(e) };
  }
  state.busy.delete(c.id);
  state.results.set(c.id, res);
  state.open.add(c.id);
  render();
  if (had && headOf(c.id)) headOf(c.id).focus();
  announce(res.text);
}

async function load(again) {
  if (state.loading) return;
  state.loading = true;
  const btn = $("#again");
  btn.busy = true;
  $("#checking").hidden = false;
  $("#error").hidden = true;
  try {
    const r = await wolos.call("doctor.check", {});
    // Anything but a report with its list is a fault, not a page with nothing on it (QA-WEB-30).
    if (!r || !Array.isArray(r.checks)) throw new Error("no report");
    state.data = r;
    state.results.clear();
    render();
    if (again) announce(t("announce.checked", { summary: summaryOf(state.data.checks) }));
  } catch (e) {
    $("#error").setAttribute("heading", t("error.title"));
    $("#error-text").textContent = explain(e);
    $("#error").hidden = false;
    $("#lists").hidden = true;
    $("#fine").hidden = true;
  } finally {
    state.loading = false;
    btn.busy = false;
    $("#checking").hidden = true;
  }
}

// ---- Keyboard: Up and Down move between checks, Home and End jump, Escape closes a check. ----

function heads() { return [...document.querySelectorAll("#lists .dr-head")]; }

document.addEventListener("keydown", (e) => {
  const inList = e.target.closest && e.target.closest("#lists");
  if (!inList) return;
  if (e.key === "Escape") {
    const w = e.target.closest(".dr-check");
    if (w && state.open.has(w.dataset.id)) {
      e.preventDefault();
      state.open.delete(w.dataset.id);
      render();
      const h = document.querySelector(`.dr-check[data-id="${CSS.escape(w.dataset.id)}"] .dr-head`);
      if (h) h.focus();
    }
    return;
  }
  if (!e.target.classList.contains("dr-head")) return;
  const hs = heads();
  const i = hs.indexOf(e.target);
  const to = { ArrowDown: i + 1, ArrowUp: i - 1, Home: 0, End: hs.length - 1 }[e.key];
  if (to == null) return;
  e.preventDefault();
  hs[Math.max(0, Math.min(hs.length - 1, to))].focus();
});

// ---- Start ----

localize();
$("#checking").setAttribute("label", t("checking"));
$("#again").addEventListener("click", () => load(true));
$("#retry").addEventListener("click", () => load(true));
$("#fine-toggle").addEventListener("click", () => { state.showFine = !state.showFine; render(); });
// The sidebar's own "select" follows the same hash change: one listener, one render (QA-WEB-17).
window.addEventListener("hashchange", render);
wolos.subscribe("progress", (p) => {
  if (!p || p.method !== "doctor.fix") return;
  for (const bar of document.querySelectorAll("wol-progress[data-progress]")) {
    bar.value = p.total ? p.done / p.total : null;
  }
});
wolos.ready.then(() => load(false));
}
