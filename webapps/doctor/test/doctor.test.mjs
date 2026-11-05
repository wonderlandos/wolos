// SPDX-License-Identifier: GPL-2.0-or-later
// Headless check of the Doctor web app in development mode (recorded answers, ADR 0048): it loads,
// counts what needs a look, is usable from the keyboard, runs a fix and shows the result, shows
// the empty state and the error state, keeps commands behind "Show the command", and takes
// light and dark pictures at 1280 and 800 wide.
//   node webapps/doctor/test/doctor.test.mjs
// Env: CHROME (browser binary; without it the test is skipped), PLAYWRIGHT (playwright-core folder), KIT (webapps/kit folder,
// default ../../kit), OUT (picture folder).
import { createRequire } from "node:module";
import { createServer } from "node:http";
import { createReadStream, existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const app = resolve(here, "..");
const kit = resolve(process.env.KIT || join(app, "..", "kit"));
const out = resolve(process.env.OUT || join(app, "test", "out"));
// No default location: a hand-placed browser in /tmp was the only way to run this (QA-WEB-12).
const chrome = process.env.CHROME;
const pw = process.env.PLAYWRIGHT || "playwright-core";
if (!chrome) {
  console.log("skipped: set CHROME to a headless Chromium binary (and PLAYWRIGHT to a playwright-core folder if it is not installed)");
  process.exit(0);
}
const { chromium } = createRequire(import.meta.url)(pw);
mkdirSync(out, { recursive: true });

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml" };
const srv = createServer((req, res) => {
  const p = decodeURIComponent(new URL(req.url, "http://x").pathname);
  const [root, rest] = p.startsWith("/kit/") ? [kit, p.slice(5)] : p.startsWith("/doctor/") ? [app, p.slice(8)] : [null, ""];
  const f = root && resolve(root, rest);
  if (!f || !f.startsWith(root) || !existsSync(f) || !statSync(f).isFile()) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { "content-type": TYPES[extname(f)] || "application/octet-stream" });
  createReadStream(f).pipe(res);
});
await new Promise((ok) => srv.listen(0, "127.0.0.1", ok));
const base = `http://127.0.0.1:${srv.address().port}/doctor/index.html?dev&delay=40`;

let failed = 0;
const ok = (cond, what) => { console.log(`${cond ? "ok  " : "FAIL"} ${what}`); if (!cond) failed++; };

const browser = await chromium.launch({ executablePath: chrome, args: ["--no-sandbox", "--password-store=basic", "--use-mock-keychain"] });
const errors = [];
async function page(theme, width, extra = "") {
  const ctx = await browser.newContext({ viewport: { width, height: 800 }, deviceScaleFactor: 1, colorScheme: theme });
  const pg = await ctx.newPage();
  pg.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  pg.on("pageerror", (e) => errors.push(String(e)));
  await pg.goto(`${base}&theme=${theme}${extra}`);
  return pg;
}

for (const theme of ["light", "dark"]) {
  for (const width of [1280, 800]) {
    const tag = `${theme}-${width}`;
    const pg = await page(theme, width);
    await pg.waitForSelector(".dr-check");
    ok(await pg.evaluate(() => document.documentElement.dataset.theme) === theme, `${tag}: theme follows the system`);
    const counts = await pg.evaluate(() => {
      const issues = [...document.querySelectorAll(".dr-check")].filter((c) => /^(warn|fail)$/.test(c.dataset.status)).length;
      const b = document.querySelector('wol-badge[data-count="all"]');
      return { issues, badge: b.hidden ? 0 : Number(b.textContent) };
    });
    ok(counts.issues > 0 && counts.badge === counts.issues, `${tag}: sidebar counts ${counts.badge} things to look at`);
    // Words on screen: no commands while "Show the command" is closed.
    const text = await pg.evaluate(() => document.querySelector("main").innerText);
    ok(!/\bwol |systemctl|pacman|sudo|--/.test(text), `${tag}: no command words on screen`);
    await pg.screenshot({ path: join(out, `doctor-all-${tag}.png`) });

    // Keyboard: sidebar to Security, Down between checks, Enter opens, Escape closes.
    await pg.focus('wol-sidebar a[href="#all"]');
    for (let i = 0; i < 4; i++) await pg.keyboard.press("ArrowDown");
    await pg.keyboard.press("Enter");
    await pg.waitForFunction(() => location.hash === "#security" && document.querySelector("#title").textContent === "Security");
    ok(true, `${tag}: arrows and Enter in the sidebar open Security`);
    await pg.focus(".dr-check .dr-head");
    await pg.keyboard.press("ArrowDown");
    const second = await pg.evaluate(() => document.activeElement === document.querySelectorAll(".dr-head")[1]);
    ok(second, `${tag}: Down moves to the next check`);
    await pg.keyboard.press("ArrowUp");
    await pg.keyboard.press("Enter");
    ok(await pg.evaluate(() => document.activeElement.getAttribute("aria-expanded") === "true"), `${tag}: Enter opens a check`);
    await pg.keyboard.press("Escape");
    ok(await pg.evaluate(() => document.activeElement.getAttribute("aria-expanded") === "false"), `${tag}: Escape closes it and keeps focus`);

    // Fix: open the firewall, Fix through the keyboard, see the progress, then the result.
    await pg.click('.dr-check[data-id="security.firewall"] .dr-head');
    const before = await pg.textContent('.dr-check[data-id="security.firewall"] wol-badge');
    await pg.focus('.dr-check[data-id="security.firewall"] [data-fix] button');
    await pg.keyboard.press("Enter");
    await pg.waitForSelector('.dr-check[data-id="security.firewall"] wol-progress:not([hidden])', { timeout: 2000 });
    ok(true, `${tag}: Fix shows progress`);
    // QA-WEB-25: the keyboard stays in the row while it works (the busy button is disabled).
    ok(await pg.evaluate(() => !!document.activeElement.closest('.dr-check[data-id="security.firewall"]')),
      `${tag}: focus stays in the check while the fix runs`);
    await pg.waitForFunction(() => {
      const r = document.querySelector('.dr-check[data-id="security.firewall"] .dr-result');
      return r && r.dataset.ok === "true";
    });
    const after = await pg.evaluate(() => {
      const w = document.querySelector('.dr-check[data-id="security.firewall"]');
      return { badge: w.querySelector("wol-badge").textContent, result: w.querySelector(".dr-result").textContent, status: w.dataset.status, open: w.querySelector(".dr-head").getAttribute("aria-expanded") };
    });
    ok(await pg.evaluate(() => !!document.activeElement.closest('.dr-check[data-id="security.firewall"]')),
      `${tag}: focus is still in the check after the fix, not on the page body`);
    ok(before === "Needs a fix" && after.badge === "Fine" && after.status === "ok" && after.result === "Fixed." && after.open === "true",
      `${tag}: Fix ran and the row says "${after.result}" (${before} -> ${after.badge})`);
    // "Show the command" opens to the command.
    await pg.click('.dr-check[data-id="security.firewall"] .dr-cmd summary');
    const cmd = await pg.textContent('.dr-check[data-id="security.firewall"] .dr-cmd');
    ok(cmd.includes("wol doctor security"), `${tag}: "Show the command" shows the command`);
    await pg.screenshot({ path: join(out, `doctor-fix-${tag}.png`) });

    // Empty state: an area with nothing to look at.
    await pg.evaluate(() => { location.hash = "#network"; });
    await pg.waitForSelector("#fine:not([hidden])");
    const fine = await pg.evaluate(() => ({ h: document.querySelector("#fine .wk-empty-heading").textContent, t: document.querySelector("#fine-time").textContent, lists: document.querySelector("#lists").hidden }));
    ok(fine.h === "Everything is fine" && /^Last checked at \d\d?:\d\d( [AP]M)?\.$/.test(fine.t) && fine.lists, `${tag}: empty state "${fine.h}", "${fine.t}"`);
    await pg.screenshot({ path: join(out, `doctor-fine-${tag}.png`) });

    // Check again.
    await pg.click("#again button");
    await pg.waitForFunction(() => document.querySelector("#announce").textContent.startsWith("Checked again."));
    ok(true, `${tag}: Check again reloads and says so`);
    await pg.context().close();
  }
}

// Error state.
const pg = await page("light", 1280, "&fail=doctor.check");
await pg.waitForSelector("#error:not([hidden])");
ok((await pg.textContent("#error")).includes("Doctor could not check your computer"), "error state names what failed");
await pg.screenshot({ path: join(out, "doctor-error-light-1280.png") });
await pg.context().close();

// ---- QA-100 failure report (journey qa100-doctor-failure-report): the app never shows an empty answer ----
// A page whose recorded answers are replaced after fixtures.js (so the mock bridge answers differently).
async function pageWith(calls, extra = "") {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, colorScheme: "light" });
  await ctx.route("**/doctor/fixtures.js", async (route) => {
    const body = await (await route.fetch()).text();
    await route.fulfill({ contentType: "text/javascript", body: `${body}\nwolkitFixtures({calls:${JSON.stringify(calls)}});` });
  });
  const p = await ctx.newPage();
  p.on("pageerror", (e) => errors.push(String(e)));
  await p.goto(`${base}&theme=light${extra}`);
  return p;
}
const failedRow = { area: "security", id: "security.firewall", status: "fail", title: "Firewall", detail: "The firewall is off.", fix: { title: "Turn on the firewall", command: "wol admin firewall on", needs_admin: true } };

{
  // wold refuses doctor.check: what was checked, what failed, what to do, and the raw text.
  const p = await pageWith({ "doctor.check": { $error: { code: "denied", message: "This app is not allowed to do that.", hint: "Turn it on in Settings > Privacy" } } });
  await p.waitForSelector("#error:not([hidden])");
  const txt = await p.evaluate(() => ({ main: document.querySelector("#error").innerText, raw: document.querySelector("#error-raw").textContent, more: document.querySelector("#error-more").open }));
  ok(/not allowed/.test(txt.main), "wold refuses: the reason is on screen");
  ok(/looking at: All checks/.test(txt.main), "wold refuses: what was being checked is on screen");
  ok(/Choose Try again/.test(txt.main), "wold refuses: what to do is on screen");
  ok(!txt.more && /denied:.*not allowed.*Settings > Privacy/.test(txt.raw), "wold refuses: the raw text is behind Show what happened");
  ok(!/\bwol /.test(txt.main), "wold refuses: no command words until Show what happened is opened");
  await p.context().close();
}
{
  // An answer with no checks is not "everything is fine".
  const p = await pageWith({ "doctor.check": { checked: 1793749137, ready: true, checks: [] } });
  await p.waitForSelector("#error:not([hidden])");
  ok(/nothing was checked/.test(await p.textContent("#error-text")) && await p.evaluate(() => document.querySelector("#fine").hidden), "an empty answer is the error state, not 'Everything is fine'");
  await p.context().close();
}
{
  // An answer that is not a report at all.
  const p = await pageWith({ "doctor.check": "nothing" });
  await p.waitForSelector("#error:not([hidden])");
  ok(/nothing was checked/.test(await p.textContent("#error-text")), "an answer that is not a report is the error state");
  await p.context().close();
}
{
  // A check that crashed: its row shows the error text (behind Show the command) and the area it stopped in.
  const crashed = { area: "security", id: "security.check", status: "fail", title: "Security checks", detail: "Doctor could not finish looking at Security. Choose Check again.", error: "the list was cut" };
  const p = await pageWith({ "doctor.check": { checked: 1793749137, ready: false, checks: [crashed] } });
  await p.waitForSelector(".dr-check");
  await p.click('.dr-check[data-id="security.check"] .dr-head');
  await p.click('.dr-check[data-id="security.check"] .dr-cmd summary');
  const t = await p.textContent('.dr-check[data-id="security.check"]');
  ok(/could not finish looking at Security/.test(t) && /the list was cut/.test(t), "a check that crashed shows its words and its error text");
  await p.context().close();
}
{
  // A fix that did not work: the controller's message, the row as it is now, and what to do.
  const p = await pageWith({
    "doctor.check": { checked: 1793749137, ready: false, checks: [failedRow] },
    "doctor.fix": { id: "security.firewall", ran: true, ok: false, check: failedRow, message: "The fix did not work: The firewall service would not start.", error: "The firewall service would not start.", next: "Choose Check again to see how it stands now." },
  });
  await p.waitForSelector(".dr-check");
  await p.click('.dr-check[data-id="security.firewall"] .dr-head');
  await p.click('.dr-check[data-id="security.firewall"] [data-fix] button');
  await p.waitForFunction(() => document.querySelector(".dr-result")?.dataset.ok === "false");
  const t = await p.textContent(".dr-result");
  ok(/would not start/.test(t) && /Choose Check again/.test(t) && /firewall is off/.test(t), "a fix that failed says why, what to do, and how the row stands");
  await p.context().close();
}
{
  // wold refuses the fix: the reason and what to do, never a blank row.
  const p = await pageWith({
    "doctor.check": { checked: 1793749137, ready: false, checks: [failedRow] },
    "doctor.fix": { $error: { code: "denied", message: "The user said no.", hint: "" } },
  });
  await p.waitForSelector(".dr-check");
  await p.click('.dr-check[data-id="security.firewall"] .dr-head');
  await p.click('.dr-check[data-id="security.firewall"] [data-fix] button');
  await p.waitForFunction(() => document.querySelector(".dr-result")?.dataset.ok === "false");
  const t = await p.textContent(".dr-result");
  ok(/user said no/.test(t) && /Check again/.test(t), "a fix wold refused says why and what to do");
  await p.context().close();
}

ok(errors.length === 0, `no page errors${errors.length ? ": " + errors.join(" | ") : ""}`);
await browser.close();
srv.close();
console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
