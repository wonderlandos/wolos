// SPDX-License-Identifier: BSD-2-Clause
// bridge.js without a browser (ADR 0048, QA-WEB-6 and QA-WEB-12): the page side of the contract,
// run in a bare node context with a fake runtime channel.
//   node --test webapps/kit/test        (crates/webengine/tests/kit_js.rs runs it when node exists)
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "bridge.js"), "utf8");

/** A page with `bridge.js` loaded over a fake channel; `posted` is what the page sent. */
function page({ search = "", init } = {}) {
  const posted = [];
  const timers = [];
  const host = {
    init: init || { app: { id: "org.wolos.webapp.t", version: "1", locale: "en", theme: "dark", accent: "green" }, permissions: ["audio.read"] },
    post: (text) => posted.push(JSON.parse(text)),
    onmessage: null,
  };
  const dataset = {};
  const ctx = {
    __wolosHost: host,
    location: { search },
    navigator: { language: "en" },
    document: { documentElement: { dataset, lang: "" } },
    matchMedia: () => ({ matches: false }),
    setTimeout: (f, ms) => { timers.push({ f, ms }); return timers.length; },
    clearTimeout: (n) => { if (timers[n - 1]) timers[n - 1].cleared = true; },
    unescape, encodeURIComponent,
    Object, Promise, Map, JSON, Error, Array, Number, RegExp, TypeError, String,
  };
  ctx.window = ctx;
  vm.runInNewContext(src, ctx);
  return { w: ctx.wolos, host, posted, timers, dataset, reply: (m) => host.onmessage(JSON.stringify(m)) };
}

test("a call is posted with its id and settles with the reply", async () => {
  const p = page();
  const r = p.w.call("audio.get", { a: 1 });
  assert.deepEqual(p.posted.filter((m) => m.call), [{ id: 1, call: "audio.get", params: { a: 1 } }]);
  p.reply({ id: 1, ok: { volume: 40 } });
  assert.deepEqual(await r, { volume: 40 });
});

test("an error reply rejects with code, message and hint", async () => {
  const p = page();
  const r = p.w.call("audio.get");
  p.reply({ id: 1, err: { code: "denied", message: "No.", hint: "Turn it on" } });
  await assert.rejects(r, (e) => e.code === "denied" && e.message === "No." && e.hint === "Turn it on");
});

test("the page and the init record: theme and accent are painted before the first call", () => {
  const p = page();
  assert.equal(p.dataset.theme, "dark");
  assert.equal(p.dataset.accent, "green");
  assert.deepEqual([...p.w.permissions], ["audio.read"]);
});

test("a request over 64 KiB is refused in the page and never posted (QA-WEB-6)", async () => {
  const p = page();
  const before = p.posted.length;
  await assert.rejects(p.w.call("audio.get", { s: "x".repeat(70000) }), (e) => e.code === "usage" && /too large/.test(e.message));
  // Multi-byte text counts in bytes: 40 000 two-byte characters are 80 000 bytes.
  await assert.rejects(p.w.call("audio.get", { s: "é".repeat(40000) }), (e) => e.code === "usage");
  assert.equal(p.posted.length, before, "nothing was sent");
  const ok = p.w.call("audio.get", { s: "x".repeat(60000) });
  p.reply({ id: 1, ok: 1 });
  assert.equal(await ok, 1, "a request under the cap goes, and takes the first id");
});

test("a call nobody answers ends in an error, not a promise that never settles (QA-WEB-6)", async () => {
  const p = page({ search: "?dev&watch=25" });
  const r = p.w.call("audio.get");
  const t = p.timers.at(-1);
  assert.equal(t.ms, 25, "?dev&watch=ms sets the limit");
  t.f();
  await assert.rejects(r, (e) => e.code === "unavailable" && /^[A-Z]/.test(e.hint));
  // A late reply for it is ignored, not thrown.
  p.reply({ id: 1, ok: 1 });
});

test("the limit is 210 s by default, more than the host's own wait for wold", () => {
  const p = page();
  p.w.call("audio.get");
  assert.equal(p.timers.at(-1).ms, 210000);
});

test("an administrator method has no page-side limit: a person may take their time at the sheet", async () => {
  const p = page();
  const n = p.timers.length;
  const r = p.w.call("doctor.fix", { id: "x" });
  assert.equal(p.timers.length, n, "no timer");
  p.reply({ id: 1, ok: { ran: true } });
  assert.deepEqual(await r, { ran: true });
});

test("an answered call clears its timer", async () => {
  const p = page();
  const r = p.w.call("audio.get");
  p.reply({ id: 1, ok: 0 });
  await r;
  assert.equal(p.timers.at(-1).cleared, true);
});

test("without a runtime every call fails with unavailable", async () => {
  const ctx = { location: { search: "" }, navigator: { language: "en" }, document: { documentElement: { dataset: {} } }, matchMedia: () => ({ matches: false }), setTimeout, clearTimeout, unescape, encodeURIComponent, Object, Promise, Map, JSON, Error, Array, Number, RegExp, TypeError, String };
  ctx.window = ctx;
  vm.runInNewContext(src, ctx);
  await assert.rejects(ctx.wolos.call("audio.get"), (e) => e.code === "unavailable");
});
