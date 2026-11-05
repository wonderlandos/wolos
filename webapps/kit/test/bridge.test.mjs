// SPDX-License-Identifier: BSD-2-Clause
// bridge.js without a browser (ADR 0048, QA-WEB-6 and QA-WEB-12): the page side of the contract,
// run in a bare node context with a fake runtime channel.
//   node --test webapps/kit/test/*.test.mjs   (crates/webengine/tests/kit_js.rs runs it when node exists;
//   a directory argument fails on Node 22, so name the files or use the glob)
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

test("a request over 60 KiB is refused in the page and never posted (QA-WEB-6)", async () => {
  const p = page();
  const before = p.posted.length;
  await assert.rejects(p.w.call("audio.get", { s: "x".repeat(70000) }), (e) => e.code === "usage" && /too large/.test(e.message));
  // Multi-byte text counts in bytes: 40 000 two-byte characters are 80 000 bytes.
  await assert.rejects(p.w.call("audio.get", { s: "é".repeat(40000) }), (e) => e.code === "usage");
  assert.equal(p.posted.length, before, "nothing was sent");
  // The cap is the host's: wold takes a call line of 60 KiB less room for the verb (QA-WEB-19).
  await assert.rejects(p.w.call("audio.get", { s: "x".repeat(60900) }), (e) => e.code === "usage");
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

test("params that cannot be sent are refused as usage and leave nothing pending (QA-WEB-28, QA-WEB-27)", async () => {
  const p = page();
  const cyclic = {};
  cyclic.me = cyclic;
  const throwing = { get a() { throw new Error("getter"); } };
  const bad = [{ n: 10n }, cyclic, throwing, { when: new Date(0) }, { s: "a\ud800b" }, { s: "\udc00" }, { "k\ud800": 1 }];
  for (const params of bad) {
    await assert.rejects(p.w.call("audio.get", params), (e) => e.code === "usage" && e instanceof Error, JSON.stringify(Object.keys(params)));
  }
  assert.equal(p.posted.filter((m) => m.call).length, 0, "nothing reached the host");
  assert.equal(p.timers.length, 0, "no watchdog was started");
  // Well-formed pairs and ordinary text still go, and the id was not used up by the refusals.
  const r = p.w.call("audio.get", { s: "ok 😀 é" });
  assert.deepEqual(p.posted.filter((m) => m.call).map((m) => m.id), [1]);
  p.reply({ id: 1, ok: 1 });
  assert.equal(await r, 1);
});

test("reads beyond their lane of four wait their turn instead of failing (QA-WEB-35)", async () => {
  const p = page();
  const rs = Array.from({ length: 10 }, (_, i) => p.w.call("audio.get", { i }));
  const sent = () => p.posted.filter((m) => m.call).map((m) => m.params.i);
  assert.deepEqual(sent(), [0, 1, 2, 3], "four reads in flight");
  p.reply({ id: 3, ok: "a" });
  assert.equal(await rs[2], "a");
  assert.deepEqual(sent(), [0, 1, 2, 3, 4], "one answer lets the next go");
  p.reply({ id: 1, ok: "b" });
  p.reply({ id: 2, err: { code: "failed", message: "No." } });
  await rs[0];
  await assert.rejects(rs[1]);
  assert.deepEqual(sent(), [0, 1, 2, 3, 4, 5, 6]);
  for (let id = 4; id <= 10; id++) p.reply({ id, ok: id });
  assert.deepEqual(await Promise.all(rs.slice(3)), [4, 5, 6, 7, 8, 9, 10]);
  assert.deepEqual(sent(), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
});

test("a read that waits in the queue has the same time limit as one that runs (QA-WEB-35)", async () => {
  const p = page({ search: "?dev&watch=25" });
  const rs = Array.from({ length: 6 }, (_, i) => p.w.call("audio.get", { i }));
  const rejected = rs.map((r) => r.then(() => "ok", (e) => e.code));
  assert.equal(p.timers.length, 6, "a watchdog for every read, queued or not");
  // The queued ones time out too, and leave the queue; the four running ones as well.
  for (const t of [...p.timers].reverse()) t.f();
  assert.deepEqual(await Promise.all(rejected), Array(6).fill("unavailable"));
  assert.equal(p.posted.filter((m) => m.call).length, 4, "the expired queued reads were never sent");
});

test("administrator calls have a lane of their own: reads never wait behind them (QA-WEB-35)", async () => {
  const p = page();
  const fixes = Array.from({ length: 4 }, (_, i) => p.w.call("doctor.fix", { id: "x" + i }));
  assert.equal(p.posted.filter((m) => m.call === "doctor.fix").length, 4, "sent at once, no timer");
  assert.equal(p.timers.length, 0, "no limit for a person at a sheet");
  // The fifth is answered busy at once, as the host would.
  await assert.rejects(p.w.call("doctor.fix", { id: "x5" }), (e) => e.code === "unavailable" && /busy/.test(e.message));
  // Four sheets nobody has answered do not hold a read back.
  const read = p.w.call("audio.get");
  assert.equal(p.posted.filter((m) => m.call === "audio.get").length, 1);
  p.reply({ id: 5, ok: "r" });
  assert.equal(await read, "r");
  // An answered fix frees its lane slot.
  p.reply({ id: 1, ok: 1 });
  assert.equal(await fixes[0], 1);
  const again = p.w.call("doctor.fix", { id: "y" });
  assert.equal(p.posted.filter((m) => m.call === "doctor.fix").length, 5);
  p.reply({ id: 6, ok: 2 });
  assert.equal(await again, 2);
});

test("a channel that throws gives the lane slot back (QA-WEB-35)", async () => {
  const p = page();
  p.host.post = () => { throw new Error("channel gone"); };
  for (let i = 0; i < 6; i++) await assert.rejects(p.w.call("audio.get"), (e) => e.code === "unavailable");
  for (let i = 0; i < 6; i++) await assert.rejects(p.w.call("doctor.fix", { id: "x" }), (e) => e.code === "unavailable" && !/busy/.test(e.message));
});

test("the page says its words are English until an app ships a table for the locale (QA-WEB-22)", () => {
  const init = { app: { id: "org.wolos.webapp.t", version: "1", locale: "pt-BR", theme: "light", accent: "violet" }, permissions: [] };
  const lang = (extra = {}) => {
    const h = { init, post() {}, onmessage: null };
    const root = { dataset: {}, lang: "" };
    const ctx = { __wolosHost: h, location: { search: "" }, navigator: { language: "pt-BR" }, document: { documentElement: root }, matchMedia: () => ({ matches: false }), setTimeout, clearTimeout, unescape, encodeURIComponent, Object, Promise, Map, JSON, Error, Array, Number, RegExp, TypeError, String, ...extra };
    ctx.window = ctx;
    vm.runInNewContext(src, ctx);
    return { root, ctx, h };
  };
  const a = lang();
  assert.equal(a.root.lang, "en", "pt-BR locale, English strings: lang en");
  assert.equal(a.ctx.wolos.app.locale, "pt-BR", "the locale itself is still the app's to read");
  assert.equal(lang({ WOLKIT_LANG: "pt-BR" }).root.lang, "pt-BR", "an app with a table says so");
});
