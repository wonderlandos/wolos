// SPDX-License-Identifier: BSD-2-Clause
// The web path, end to end (WOLKIT-1TO1): serves SITE/public/kit, opens /kit.html in headless
// Chromium with WebGPU (SwiftShader's software Vulkan, so no GPU is needed), waits for Wolkit's
// first frame, reads the frame back from the GPU (window.__kit.snapshot) and compares it with the
// native CPU frame of the same page (crates/wolkit-render tests/page.rs writes them; the shell's
// Kit goldens are the same bytes): zero differing pixels on the exact path (the default), the GPU
// tolerance with ?raster=gpu. Cases: the sample window, the sample and Settings > Look at 480, 900
// and 1280 px, light and dark. Then the live theme: prefers-color-scheme flips (no reload) and the
// host's theme message, each compared with the other variant's native frame. Then a click on the
// switch. Prints the table as JSON.
//   node crates/wolkit-web/check-browser.mjs SITE_DIR NATIVE_DIR [OUT_DIR]
// NATIVE_DIR holds <case>-<light|dark>-cpu.png (the target tmp dir of the page test, tmp/page).
// Needs playwright-core (PLAYWRIGHT_CORE, or SITE/.tools/docs-ai/node_modules) and a Chromium
// (CHROME, or the Playwright cache in /tmp/pw-cr/browsers or ~/.cache/ms-playwright).
import { createServer } from "node:http";
import { existsSync, readFileSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { inflateSync, deflateSync } from "node:zlib";

const [site, nativeDir, outDir = "/tmp/wolkit-web-check"] = process.argv.slice(2);
if (!site || !nativeDir) { console.error("usage: check-browser.mjs SITE_DIR NATIVE_DIR [OUT_DIR]"); process.exit(2); }
mkdirSync(outDir, { recursive: true });
const root = resolve(site, "public");
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".otf": "font/otf", ".svg": "image/svg+xml", ".json": "application/json" };

function chrome() {
  if (process.env.CHROME) return process.env.CHROME;
  for (const base of ["/tmp/pw-cr/browsers", join(process.env.HOME || "", ".cache/ms-playwright")]) {
    if (!existsSync(base)) continue;
    for (const d of readdirSync(base).filter((d) => d.startsWith("chromium-")).sort().reverse()) {
      const p = join(base, d, "chrome-linux64/chrome");
      if (existsSync(p)) return p;
    }
  }
  throw new Error("no Chromium (set CHROME)");
}

// A minimal PNG reader/writer (RGBA8, filter 0..4) so the check needs nothing but playwright-core.
function readPng(buf) {
  let at = 8, w = 0, h = 0, ct = 0; const idat = [];
  while (at < buf.length) {
    const len = buf.readUInt32BE(at), type = buf.toString("ascii", at + 4, at + 8), d = buf.subarray(at + 8, at + 8 + len);
    if (type === "IHDR") { w = d.readUInt32BE(0); h = d.readUInt32BE(4); ct = d[9]; }
    if (type === "IDAT") idat.push(d);
    at += 12 + len;
  }
  const bpp = ct === 6 ? 4 : 3, raw = inflateSync(Buffer.concat(idat)), out = Buffer.alloc(w * h * 4), prev = Buffer.alloc(w * bpp), cur = Buffer.alloc(w * bpp);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (w * bpp + 1)], row = raw.subarray(y * (w * bpp + 1) + 1, (y + 1) * (w * bpp + 1));
    for (let i = 0; i < w * bpp; i++) {
      const a = i >= bpp ? cur[i - bpp] : 0, b = prev[i], c = i >= bpp ? prev[i - bpp] : 0;
      const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
      const pred = [0, a, b, (a + b) >> 1, pa <= pb && pa <= pc ? a : pb <= pc ? b : c][f];
      cur[i] = (row[i] + pred) & 255;
    }
    for (let x = 0; x < w; x++) { for (let k = 0; k < 3; k++) out[(y * w + x) * 4 + k] = cur[x * bpp + k]; out[(y * w + x) * 4 + 3] = bpp === 4 ? cur[x * bpp + 3] : 255; }
    cur.copy(prev);
  }
  return { w, h, data: out };
}
function writePng(path, w, h, rgba) {
  const crcT = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (b) => { let c = ~0; for (const x of b) c = crcT[(c ^ x) & 255] ^ (c >>> 8); return (~c) >>> 0; };
  const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([l, td, c]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 6;
  const raw = Buffer.alloc(h * (w * 4 + 1)); for (let y = 0; y < h; y++) Buffer.from(rgba.subarray(y * w * 4, (y + 1) * w * 4)).copy(raw, y * (w * 4 + 1) + 1);
  writeFileSync(path, Buffer.concat([Buffer.from([0x89, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]));
}
// The GPU holds premultiplied pixels; the window is opaque, so they are also the straight ones.
const straight = (px) => px;

const server = createServer((req, res) => {
  let p = decodeURIComponent(new URL(req.url, "http://x").pathname);
  if (p.endsWith("/")) p += "index.html";
  const f = join(root, p);
  if (!f.startsWith(root) || !existsSync(f)) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { "Content-Type": TYPES[extname(f)] || "application/octet-stream" });
  res.end(readFileSync(f));
}).listen(0);
const port = server.address().port;

const pw = process.env.PLAYWRIGHT_CORE || resolve(site, ".tools/docs-ai/node_modules/playwright-core/index.mjs");
const { chromium } = await import(pathToFileURL(pw).href);
const browser = await chromium.launch({
  executablePath: chrome(),
  args: ["--enable-unsafe-webgpu", "--enable-features=Vulkan", "--use-vulkan=swiftshader", "--use-webgpu-adapter=swiftshader", "--use-angle=swiftshader", "--ignore-gpu-blocklist"],
});
const result = { ok: false, cases: [], theme: [], gpu: [] };
const compare = (px, want) => {
  let any = 0, over1 = 0, worst = 0;
  if (px.length !== want.data.length) return { pixels: px.length / 4, any: -1, over1: -1, worst: 255 };
  for (let i = 0; i < px.length; i += 4) {
    let m = 0; for (let k = 0; k < 4; k++) m = Math.max(m, Math.abs(px[i + k] - want.data[i + k]));
    if (m > 0) any++; if (m > 1) over1++; worst = Math.max(worst, m);
  }
  return { pixels: px.length / 4, any, over1, worst };
};
const native = (name, theme) => readPng(readFileSync(join(nativeDir, `${name}-${theme}-cpu.png`)));
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 }, deviceScaleFactor: 1 });
  const logs = [];
  page.on("console", (m) => logs.push(m.text()));
  page.on("pageerror", (e) => logs.push(`pageerror: ${e}`));
  result.logs = logs;
  let all = true;
  const open = async (q) => {
    await page.goto(`http://127.0.0.1:${port}/kit.html?${q}`);
    await page.waitForFunction(() => window.__kit && (window.__kit.timings || window.__kit.error), null, { timeout: 60000 });
    const err = await page.evaluate(() => window.__kit.error);
    if (err) throw new Error(err);
  };
  const snap = async () => Buffer.from(await page.evaluate(() => window.__kit.snapshot()));
  const cases = [["app-window-parts", "w=420&h=380"]];
  for (const w of [480, 900, 1280]) {
    cases.push([`app-window-parts-w${w}`, `w=${w}&h=600`]);
    cases.push([`settings-look-w${w}`, `tree=look&w=${w}&h=600`]);
  }
  // The Design System component cases (crates/wolkit-render/fixtures/components.json).
  const comps = JSON.parse(readFileSync(new URL("../wolkit-render/fixtures/components.json", import.meta.url), "utf8"));
  for (const c of comps.cases) cases.push([`kit-view-${c.name}`, `case=${c.name}`]);
  // No HTML or CSS for the interface (the lead's rule): the served page's body holds exactly the
  // canvas, the loader script and the no-WebGPU notice (hidden while the engine draws); no style
  // sheet, no other text.
  {
    await open("w=420&h=380&theme=light");
    const dom = await page.evaluate(() => ({
      body: [...document.body.querySelectorAll("*")].map((e) => e.tagName.toLowerCase() + (e.id ? `#${e.id}` : "")),
      sheets: document.styleSheets.length,
      links: document.querySelectorAll('link[rel="stylesheet"], style').length,
      text: (() => { const out = []; const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT); while (w.nextNode()) { const t = w.currentNode.textContent.trim(); if (t && w.currentNode.parentElement.id !== "kit-notice") out.push(t); } return out; })(),
      notice_hidden: document.getElementById("kit-notice").hidden,
      canvas_fills: (() => { const c = document.getElementById("kit"); return [c.width, c.height]; })(),
    }));
    const want = ["canvas#kit", "p#kit-notice", "script"];
    dom.ok = JSON.stringify(dom.body) === JSON.stringify(want) && dom.sheets === 0 && dom.links === 0 && dom.text.length === 0 && dom.notice_hidden;
    result.dom = dom;
    all = all && dom.ok;
    // Without ?w/?h the canvas is the viewport (1400 x 900 here) and follows a resize.
    await open("theme=light");
    const full = await page.evaluate(() => window.__kit.size());
    await page.setViewportSize({ width: 900, height: 700 });
    await page.waitForFunction(() => window.__kit.size()[0] === 900, null, { timeout: 10000 });
    const resized = await page.evaluate(() => window.__kit.size());
    await page.setViewportSize({ width: 1400, height: 900 });
    result.viewport = { full, resized };
    all = all && full[0] === 1400 && full[1] === 900 && resized[0] === 900 && resized[1] === 700;
  }
  // Exact path (the default): zero differing pixels from the native CPU frame.
  for (const [name, q] of cases) {
    for (const theme of ["light", "dark"]) {
      await open(`${q}&theme=${theme}`);
      const px = await snap();
      const want = native(name, theme);
      writePng(join(outDir, `web-${name}-${theme}.png`), want.w, want.h, px);
      const d = compare(px, want);
      const raster = await page.evaluate(() => window.__kit.page.raster());
      result.cases.push({ case: `${name}-${theme}`, raster, ...d });
      all = all && raster === "exact" && d.any === 0;
    }
  }
  await page.locator("#kit").screenshot({ path: join(outDir, "kit-page-screenshot.png") });
  // GPU rasteriser: within the GPU tolerance.
  for (const theme of ["light", "dark"]) {
    await open(`w=420&h=380&theme=${theme}&raster=gpu`);
    const d = compare(await snap(), native("app-window-parts", theme));
    const within = d.worst <= 2 && d.over1 <= 0.001 * d.pixels;
    result.gpu.push({ case: `app-window-parts-${theme}`, raster: "gpu", ...d, within });
    all = all && within;
  }
  // Live theme, no reload: the system scheme flips, then the host's theme message.
  for (const start of ["light", "dark"]) {
    const to = start === "light" ? "dark" : "light";
    await page.emulateMedia({ colorScheme: start });
    await open("w=420&h=380");
    const before = compare(await snap(), native("app-window-parts", start));
    const nav = await page.evaluate(() => performance.getEntriesByType("navigation").length);
    await page.emulateMedia({ colorScheme: to });
    await page.waitForFunction(() => window.__kit.themeFlips() > 0, null, { timeout: 10000 });
    const after = compare(await snap(), native("app-window-parts", to));
    const reloaded = (await page.evaluate(() => performance.getEntriesByType("navigation").length)) !== nav || (await page.evaluate(() => window.__kit.themeFlips())) !== 1;
    await page.evaluate((d) => window.postMessage({ type: "wolos-theme", dark: d === "dark" }, "*"), start);
    await page.waitForFunction(() => window.__kit.themeFlips() > 1, null, { timeout: 10000 });
    const back = compare(await snap(), native("app-window-parts", start));
    result.theme.push({ from: start, to, before: before.any, after_scheme_flip: after.any, after_host_message: back.any, reloaded });
    all = all && before.any === 0 && after.any === 0 && back.any === 0 && !reloaded;
  }
  await page.emulateMedia({ colorScheme: "light" });
  // The switch (part 2): click its centre; the window must change.
  await open("w=420&h=380&theme=light");
  const px = await snap();
  const box = await page.locator("#kit").boundingBox();
  const [x, y, w, h] = await page.evaluate(() => window.__kit.page.part_rect(2));
  await page.mouse.click(box.x + x + w / 2, box.y + y + h / 2);
  await page.waitForTimeout(600);
  const after = await snap();
  let changed = 0; for (let i = 0; i < after.length; i += 4) if (Math.abs(after[i] - px[i]) > 8) changed++;
  writePng(join(outDir, "kit-webgpu-light-clicked.png"), 420, 380, after);
  result.click_changed_pixels = changed;
  all = all && changed > 50;
  result.ok = all;
} catch (e) {
  result.error = String(e);
  throw e;
} finally {
  await browser.close();
  server.close();
}
console.log(JSON.stringify(result, null, 1));
process.exit(result.ok ? 0 : 1);
