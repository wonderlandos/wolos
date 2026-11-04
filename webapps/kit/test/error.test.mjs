// SPDX-License-Identifier: BSD-2-Clause
// error.js without a browser (ADR 0048, QA-WEB-9): the words per reason, and where Try again goes.
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "error.js"), "utf8");

function page(search) {
  const nodes = {};
  const mk = (id) => (nodes[id] = { id, textContent: "", hidden: false, listeners: {}, addEventListener(ev, f) { this.listeners[ev] = f; }, focus() { this.focused = true; } });
  ["title", "lead", "again", "again-row"].forEach(mk);
  const ctx = {
    URLSearchParams,
    URL,
    location: { search, href: "file:///usr/share/wolos/webapps/kit/error.html" + search },
    document: { readyState: "complete", title: "", getElementById: (id) => nodes[id], addEventListener() {} },
    Object,
  };
  ctx.window = ctx;
  vm.runInNewContext(src, ctx);
  return { nodes, ctx };
}

test("a crash names the app and offers Try again to the app's own page", () => {
  const retry = "file:///usr/share/wolos/webapps/doctor/index.html";
  const p = page(`?reason=crashed&name=Doctor&retry=${encodeURIComponent(retry)}`);
  assert.equal(p.nodes.title.textContent, "Doctor stopped");
  assert.match(p.nodes.lead.textContent, /stopped working/);
  assert.equal(p.nodes["again-row"].hidden, false);
  p.nodes.again.listeners.click();
  assert.equal(p.ctx.location.href, retry);
});

test("every reason has its own words and an unknown one is the generic line", () => {
  for (const r of ["crashed", "manifest", "kit", "refused", "channel"]) {
    const p = page(`?reason=${r}&name=Doctor`);
    assert.match(p.nodes.title.textContent, /^Doctor /, r);
    assert.ok(p.nodes.lead.textContent.length > 10, r);
  }
  const g = page("?reason=__proto__&name=Doctor");
  assert.equal(g.nodes.title.textContent, "Something went wrong");
  assert.equal(page("").nodes.title.textContent, "Something went wrong");
});

test("no Try again without the app's own file address, read the way the engine reads it", () => {
  const apps = "file:///usr/share/wolos/webapps";
  for (const bad of [
    "https://example.org/x.html", "javascript:alert(1)", "",
    `${apps}/doctor/../../../../etc/x.html`, `${apps}/doctor/%2e%2e/%2e%2e/x.html`, `${apps}/doctor/%2E%2E/kit/error.html`,
    `${apps}/kit/error.html`, `${apps}/doctor/index.txt`, `${apps}/doctor/index.html?x=1`, `${apps}/doctor/index.html#a`,
    `${apps}/doctor/a%2fb.html`, "file:///etc/passwd.html", `${apps}/Doctor/index.html`, `${apps}/index.html`,
  ]) {
    const p = page(`?reason=crashed&name=D&retry=${encodeURIComponent(bad)}`);
    assert.equal(p.nodes["again-row"].hidden, true, bad);
  }
  const good = page(`?reason=crashed&name=D&retry=${encodeURIComponent(`${apps}/hello/index.html`)}`);
  assert.equal(good.nodes["again-row"].hidden, false);
});
