// SPDX-License-Identifier: GPL-2.0-or-later
// Playwright side of the Atlas Drive benchmark (docs/bench/atlas-drive.md): the same sequence as
// crates/atlas-drive/examples/bench.rs, driving the SAME Atlas binary as executablePath.
// usage: node pw.mjs <atlas binary> <name=url,...> <app page name>
import { chromium } from 'playwright';
import fs from 'node:fs';

const [exe, pageSpec, app] = process.argv.slice(2);
const pages = pageSpec.split(',').map((p) => [p.slice(0, p.indexOf('=')), p.slice(p.indexOf('=') + 1)]);
const ms = (t) => Math.round((performance.now() - t) * 100) / 100;
const tokens = (bytes) => Math.ceil(bytes / 4);
const rssKb = (pid) => { try { return +fs.readFileSync(`/proc/${pid}/status`, 'utf8').match(/VmRSS:\s+(\d+)/)[1]; } catch { return 0; } };
const ctx = (pid) => {
  let n = 0;
  try {
    for (const t of fs.readdirSync(`/proc/${pid}/task`)) {
      const s = fs.readFileSync(`/proc/${pid}/task/${t}/status`, 'utf8');
      for (const m of s.matchAll(/(?:non)?voluntary_ctxt_switches:\s+(\d+)/g)) n += +m[1];
    }
  } catch {}
  return n;
};
const children = (root) => {
  const kids = new Map();
  for (const d of fs.readdirSync('/proc')) {
    if (!/^\d+$/.test(d)) continue;
    try {
      const st = fs.readFileSync(`/proc/${d}/stat`, 'utf8');
      const pp = +st.slice(st.lastIndexOf(')') + 2).split(' ')[1];
      if (!kids.has(pp)) kids.set(pp, []);
      kids.get(pp).push(+d);
    } catch {}
  }
  const out = []; const st = [root];
  while (st.length) { const p = st.pop(); out.push(p); for (const k of kids.get(p) || []) st.push(k); }
  return out;
};

const out = {};
const browser = await chromium.launch({ executablePath: exe, headless: true, args: ['--no-sandbox', '--disable-gpu'] });
let page = await browser.newPage();
await page.goto('about:blank');
out.cold_ms = Math.round(process.uptime() * 1000 * 100) / 100;

out.pages = {};
for (const [name, url] of pages) {
  const m = {};
  let t = performance.now();
  try { await page.goto(url, { waitUntil: 'load' }); } catch (e) { m.error = String(e.message).split('\n')[0]; out.pages[name] = m; continue; }
  m.goto_ms = ms(t);
  t = performance.now();
  const aria = await page.locator('body').ariaSnapshot();
  m.aria_ms = ms(t); m.aria_bytes = Buffer.byteLength(aria); m.aria_tokens = tokens(m.aria_bytes);
  t = performance.now();
  const html = await page.content();
  m.content_ms = ms(t); m.content_bytes = Buffer.byteLength(html); m.content_tokens = tokens(m.content_bytes);
  out.pages[name] = m;
}

const appUrl = pages.find(([n]) => n === app)[1];
await page.goto(appUrl);
const bottom = page.locator('#bottom');
const click = [];
for (let i = 0; i < 10; i++) { const t = performance.now(); await page.click('#bottom'); click.push(ms(t)); }
out.click_ms = click;
const clickLoc = [];
for (let i = 0; i < 10; i++) { const t = performance.now(); await page.getByRole('button', { name: 'Bottom button' }).click(); clickLoc.push(ms(t)); }
out.click_locator_ms = clickLoc;
let t = performance.now();
for (let i = 0; i < 100; i++) await page.click('#bottom');
out.clicks100_ms = ms(t);
const shot = `/tmp/pwb-shot-${process.pid}.png`;
t = performance.now();
await page.screenshot({ fullPage: true, path: shot });
out.shot_ms = ms(t); out.shot_bytes = fs.statSync(shot).size; fs.unlinkSync(shot);

// agent task, exploring: the agent has no locators yet, so it reads the page, acts, reads again
await page.goto(appUrl);
t = performance.now();
let toks = 0;
const a = await page.locator('body').ariaSnapshot(); toks += tokens(Buffer.byteLength(a));
await page.getByLabel('Email').fill('me@example.com'); toks += 2;
await page.getByRole('button', { name: 'Save' }).click(); toks += 2;
await page.getByRole('status').waitFor();
const b = await page.locator('body').ariaSnapshot(); toks += tokens(Buffer.byteLength(b));
out.task = { ms: ms(t), tokens: toks, toast_seen: b.includes('Saved!'), calls: 4 };
// the same task when the script already knows its locators (nothing is re-read)
await page.goto(appUrl);
t = performance.now();
await page.getByLabel('Email').fill('me@example.com');
await page.getByRole('button', { name: 'Save' }).click();
const toast = await page.getByRole('status').innerText();
out.task_known = { ms: ms(t), tokens: tokens(Buffer.byteLength(toast)) + 4, toast_seen: toast.includes('Saved!'), calls: 3 };

await new Promise((r) => setTimeout(r, 1500));
const tree = children(process.pid).filter((p) => p !== process.pid);
out.browser_rss_mb = Math.round(tree.reduce((s, p) => s + rssKb(p), 0) / 1024);
out.driver_rss_mb = Math.round(rssKb(process.pid) / 1024 * 10) / 10;
const c0 = ctx(process.pid);
await new Promise((r) => setTimeout(r, 10000));
out.idle_wakeups_per_s = Math.round((ctx(process.pid) - c0) / 10 * 100) / 100;
await browser.close();
console.log(JSON.stringify(out));
