// SPDX-License-Identifier: BSD-2-Clause
// Hello: the starter system web app. Reads through window.wolos only (bridge.js), draws with the kit.
"use strict";
{
const { t, localize, explain } = window.wolkit;

const $ = (s) => document.querySelector(s);
const row = (label, detail, ...end) => {
  const r = document.createElement("wol-row");
  r.setAttribute("label", label);
  if (detail) r.setAttribute("detail", detail);
  r.append(...end);
  return r;
};
const badge = (state) => {
  const b = document.createElement("wol-badge");
  b.setAttribute("state", state);
  return b;
};
const gb = (bytes) => `${(bytes / 1e9).toFixed(bytes < 1e10 ? 1 : 0)} GB`;

/** A failed load: the error in words and a way to try again, inside the list it was for. */
function failed(list, err, retry) {
  const b = document.createElement("wol-button");
  b.textContent = t("error.retry");
  b.addEventListener("click", retry);
  const r = row(t("error.title"), explain(err), b);
  r.classList.add("error");
  list.replaceRows(r);
}

async function overview() {
  const list = $("#about");
  try {
    const [about, power] = await Promise.all([wolos.call("about"), wolos.call("power.status")]);
    const details = $("#details").checked;
    const rows = [
      row(t("overview.name"), about.name),
      row(t("overview.processor"), about.processor),
      row(t("overview.memory"), about.memory),
      row(t("overview.power"), power.source === "battery" ? t("overview.power.battery") : t("overview.power.ac")),
    ];
    if (details) rows.push(row(t("overview.kernel"), about.kernel), row(t("overview.base"), about.base));
    list.replaceRows(...rows);
  } catch (e) {
    failed(list, e, overview);
  }
}

async function storage() {
  const list = $("#disks");
  try {
    const disks = await wolos.call("sys", { topic: "disks" });
    list.replaceRows(...disks.map((d) => {
      const p = document.createElement("wol-progress");
      p.className = "disk";
      p.setAttribute("value", String(d.total ? d.used / d.total : 0));
      p.setAttribute("label", d.mount);
      const full = d.total && d.available / d.total < 0.1;
      return row(d.mount === "/" ? t("storage.system") : d.mount, t("storage.free", { free: gb(d.available), total: gb(d.total) }), p, badge(full ? "warn" : "ok"));
    }));
  } catch (e) {
    failed(list, e, storage);
  }
}

let checks = [];
function drawChecks() {
  const q = $("#sound-search").value.toLowerCase();
  const shown = checks.filter((c) => !q || `${c.id} ${c.detail} ${c.fix}`.toLowerCase().includes(q));
  $("#checks").replaceRows(...shown.map((c) => {
    const r = row(c.detail, "", badge(c.status === "info" ? "info" : c.status));
    if (c.fix) {
      const fix = document.createElement("div");
      fix.className = "fix";
      fix.slot = "text";
      const code = document.createElement("code");
      code.textContent = c.fix;
      fix.append(code);
      r.append(fix);
    }
    return r;
  }));
  $("#checks").hidden = !shown.length;
  $("#checks-empty").hidden = !!shown.length;
}

async function sound() {
  const again = $("#again");
  again.busy = true;
  try {
    const r = await wolos.call("audio.doctor");
    checks = r.checks || [];
    const bad = checks.filter((c) => c.status === "warn" || c.status === "fail");
    $("#sound-summary").textContent = r.ready || !bad.length ? t("sound.ready") : t("sound.notready", { n: bad.length });
    const worst = checks.some((c) => c.status === "fail") ? "fail" : bad.length ? "warn" : "ok";
    const sb = $("#sound-badge");
    sb.setAttribute("state", worst);
    sb.hidden = worst === "ok";
    drawChecks();
  } catch (e) {
    $("#checks").hidden = false;
    $("#checks-empty").hidden = true;
    failed($("#checks"), e, sound);
  } finally {
    again.busy = false;
  }
}

function show(page) {
  for (const s of document.querySelectorAll("[data-page]")) s.hidden = s.dataset.page !== page;
}

localize();
document.title = t("app.title");
document.addEventListener("select", (e) => show(e.detail.value));
show($("wol-sidebar").value || "overview");
$("#details").addEventListener("change", overview);
$("#again").addEventListener("click", sound);
$("#sound-search").addEventListener("search", drawChecks);
wolos.subscribe("permissions", () => { overview(); storage(); sound(); });
overview();
storage();
sound();
}
