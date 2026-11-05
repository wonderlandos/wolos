# SPDX-License-Identifier: GPL-2.0-or-later
"""page_asis.py: "Every page as the site made it" (plan AT-7, reel scenes 3 and 5 / R05, promise
P04, decision D4).

How: a local corpus page (or any --page FILE) is opened with the blocker off and with it on, each
on a fresh profile in the egress namespace. A probe added after the page's own markup reports the
page's text (every text node outside script, style, title, template, noscript), the sequence of its
elements, a hash of every element's box (x, y, width, height, rounded) and a canvas readback.
PASS when, with the blocker off, the text and the element sequence equal what the raw bytes of the
page say (read with the standard HTML parser) and the layout hash is the same with the blocker on
and off. The canvas readback is documented, not judged: fingerprint noise (D4, kept on) changes
readbacks of canvas pixels only, and the report gives how many of 4096 pixels differ from what was
drawn.

  python3 -I page_asis.py run --browser 'CMD' [--page FILE] [--out DIR]
  python3 -I page_asis.py --selftest
Exit: 0 pass, 1 the page changed, 2 cannot run.
"""
import argparse
import json
import os
import shlex
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import privlib as L  # noqa: E402

CORPUS = """<!doctype html>
<html lang=en><head><meta charset=utf-8><title>Walking the harbour</title>
<style>body{font:17px/1.5 sans-serif;margin:2em auto;max-width:44em;padding:0 1em}
figure{margin:1em 0}table{border-collapse:collapse}td,th{border:1px solid #888;padding:.2em .6em}
.note{border-left:4px solid #888;padding-left:1em}</style></head>
<body>
<header><h1>Walking the harbour</h1><p class=lede>Three routes along the water, with times and where to rest.</p></header>
<nav><ul><li><a href=#short>Short route</a></li><li><a href=#long>Long route</a></li><li><a href=#rain>When it rains</a></li></ul></nav>
<main>
<section id=short><h2>Short route</h2><p>Start at the ferry steps and follow the harbour wall to the lighthouse.
It takes <strong>forty minutes</strong> at an easy pace &amp; has benches every few hundred metres.</p>
<figure><img src=/map.gif width=320 height=180 alt="Map of the short route"><figcaption>The short route, drawn by hand.</figcaption></figure></section>
<section id=long><h2>Long route</h2><table><thead><tr><th>Stage</th><th>Distance</th><th>Time</th></tr></thead>
<tbody><tr><td>Ferry steps to the fish market</td><td>1.2 km</td><td>15 min</td></tr>
<tr><td>Fish market to the old rope works</td><td>2.4 km</td><td>35 min</td></tr>
<tr><td>Rope works to the lighthouse</td><td>1.9 km</td><td>30 min</td></tr></tbody></table>
<p class=note>Bring water: there is no cafe between the market and the rope works.</p>
<div class="ad-banner sponsor">Local bakery: fresh bread from seven.</div></section>
<section id=rain><h2>When it rains</h2><ol><li>Take the covered arcade behind the market.</li><li>Wait out the shower in the maritime museum (free on Sundays).</li></ol>
<details><summary>Tide times</summary><p>High tide is about fifty minutes later each day.</p></details></section>
</main><footer><p>Written by the people who walk it. <em>Last walked in October.</em></p></footer>
</body></html>
"""
PROBE = """<script type=application/json id=asis>{"report": "report"}</script><script>
window.addEventListener('load', async () => {
  const SKIP = new Set(['SCRIPT', 'STYLE', 'TITLE', 'TEMPLATE', 'NOSCRIPT']);
  const probe = [...document.querySelectorAll('script')].slice(-2);
  const text = [];
  const w = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_TEXT);
  for (let n = w.nextNode(); n; n = w.nextNode()) {
    let p = n.parentElement, skip = false;
    for (; p; p = p.parentElement) if (SKIP.has(p.tagName)) { skip = true; break; }
    if (!skip) text.push(n.data);
  }
  const els = [...document.getElementsByTagName('*')].filter(e => !probe.includes(e));
  const tags = els.map(e => e.tagName.toLowerCase());
  const boxes = els.filter(e => e.closest('body')).map(e => { const r = e.getBoundingClientRect();
    return [e.tagName, Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; });
  const vw = document.documentElement.clientWidth;
  const buf = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(JSON.stringify([vw, boxes])));
  const layout = [...new Uint8Array(buf)].map(b => b.toString(16).padStart(2, '0')).join('');
  const c = document.createElement('canvas'); c.width = c.height = 64;
  const g = c.getContext('2d'); g.fillStyle = 'rgb(10,120,200)'; g.fillRect(0, 0, 64, 64);
  const d = g.getImageData(0, 0, 64, 64).data; let diff = 0;
  for (let i = 0; i < d.length; i += 4) if (d[i] !== 10 || d[i + 1] !== 120 || d[i + 2] !== 200 || d[i + 3] !== 255) diff++;
  const hidden = els.filter(e => e.closest('body') && getComputedStyle(e).display === 'none' && e.tagName !== 'SCRIPT')
    .map(e => e.tagName.toLowerCase() + (e.className ? '.' + e.className : ''));
  await fetch('report', {method: 'POST', body: JSON.stringify({text: text.join(' ').split(/\\s+/).join(' ').trim(),
    tags, layout, viewport: vw, boxes: boxes.length, canvas_diff: diff, hidden})});
});
</script>"""
IGNORED_TAGS = {"html", "head", "body", "tbody"}  # implied by the parser when the markup leaves them out


def with_probe(raw):
    i = raw.lower().rfind("</body>")
    return raw[:i] + PROBE + raw[i:] if i >= 0 else raw + PROBE


def raw_view(raw):
    p = L.TextAndTags()
    p.feed(raw)
    p.close()
    return L.norm_text(" ".join(p.text)), [t for t in p.tags if t not in IGNORED_TAGS]


def run_one(iso, a, srv, name, extra, reports, prefs=None):
    profile = iso.mkdir(f"profile-{name}")
    if prefs:
        L.seed_prefs(profile, prefs)
    pipe = L.CdpPipe()
    b = L.Browser(L.browser_argv(a.browser), profile, prefix=iso.prefix(),
                  extra=shlex.split(a.browser_args) + extra, urls=["about:blank"],
                  logfile=os.path.join(iso.tmp, f"browser-{name}.log"), env=L.person_env(iso), pipe=pipe)
    b.start()
    time.sleep(a.settle)
    try:
        pipe.page_text(srv.url(f"/r/{name}/page.html"), wait=1, expr="document.readyState")
    except RuntimeError as e:
        L.log(f"page_asis: {name}: {e}")
    L.wait_for(lambda: name in reports, a.timeout)
    b.stop()
    return reports.get(name)


def inner(a, iso):
    with open(a.page, encoding="utf-8") if a.page else _corpus() as f:
        raw = f.read()
    reports = {}
    gif = bytes.fromhex("47494638396101000100800000000000ffffff21f90401000000002c00000000010001000002024401003b")

    def r(h, rec):
        parts = rec["path"].split("?")[0].split("/")
        name, leaf = (parts[2], parts[-1]) if len(parts) > 3 else ("?", parts[-1])
        if leaf == "page.html":
            return 200, "text/html; charset=utf-8", with_probe(raw).encode()
        if leaf == "report":
            try:
                reports[name] = json.loads(rec["body"])
            except ValueError:
                reports[name] = {"bad": rec["body"][:200]}
            return 200, "text/plain", b"ok"
        return 200, "image/gif", gif
    srv = L.Server({"/r/*": r, "/map.gif": ("image/gif", gif)}).start()
    off = run_one(iso, a, srv, "off", shlex.split(a.off_args), reports, None if a.off_args else L.UBLOCK_OFF_PREFS)
    on = run_one(iso, a, srv, "on", [], reports)
    srv.stop()
    text, tags = raw_view(raw)
    fails = []
    for nm, rep in (("off", off), ("on", on)):
        if not rep or "text" not in rep:
            fails.append(f"{nm}: the page did not report")
    if off and "text" in off:
        if off["text"] != text:
            a_, b_ = off["text"], text
            k = next((i for i in range(min(len(a_), len(b_))) if a_[i] != b_[i]), min(len(a_), len(b_)))
            fails.append(f"off: the page text differs from the raw page at character {k}: "
                         f"browser {a_[max(0, k - 30):k + 30]!r} vs raw {b_[max(0, k - 30):k + 30]!r}")
        dom_tags = [t for t in off["tags"] if t not in IGNORED_TAGS]
        if dom_tags != tags:
            extra = [t for t in dom_tags if t not in tags]
            fails.append(f"off: the element sequence differs from the raw page ({len(dom_tags)} vs {len(tags)}; "
                         f"not in the raw page: {sorted(set(extra))})")
    if off and on and "layout" in off and "layout" in on:
        if off.get("viewport") != on.get("viewport"):
            fails.append(f"viewport differs between runs ({off.get('viewport')} vs {on.get('viewport')}): layout not comparable")
        elif off["layout"] != on["layout"]:
            fails.append(f"layout hash differs with the blocker on ({on['layout'][:12]}) and off ({off['layout'][:12]}); "
                         f"hidden with it on: {on.get('hidden')}")
    rep = {"tool": "page_asis", "browser": a.browser, "page": a.page or "built-in corpus", "isolation": iso.facts,
           "raw": {"text_chars": len(text), "elements": len(tags)}, "off": off, "on": on, "off_args": a.off_args,
           "canvas": {"pixels": 4096, "differing_off": (off or {}).get("canvas_diff"), "differing_on": (on or {}).get("canvas_diff"),
                      "note": "fingerprint noise changes canvas readbacks only (D4); not judged"},
           "failures": fails, "verdict": "FAIL" if fails else "PASS"}
    L.write_json(a.out and os.path.join(a.out, "page_asis.json"), rep)
    print(f"PAGE AS MADE {rep['verdict']}: raw page {len(text)} characters of text, {len(tags)} elements; "
          f"layout off {(off or {}).get('layout', '?')[:12]} on {(on or {}).get('layout', '?')[:12]}; "
          f"canvas readback: {rep['canvas']['differing_off']} of 4096 pixels differ from what was drawn (blocker off), "
          f"{rep['canvas']['differing_on']} (on)")
    for f in fails:
        print(f"  FAIL {f}")
    L.cleanup(iso, a.keep)
    return 0 if not fails else 1


class _corpus:
    def __enter__(self):
        import io
        return io.StringIO(CORPUS)

    def __exit__(self, *e):
        return False


def selftest():
    import subprocess
    import tempfile
    me = os.path.abspath(__file__)
    fails = []
    print("page_asis selftest: stub browser (no isolation needed)")
    t, tags = raw_view(CORPUS)
    ok = "forty minutes at an easy pace & has benches" in t and tags[:3] == ["meta", "title", "style"]
    print(f"  {'ok  ' if ok else 'FAIL'} raw view: text and element sequence ({len(t)} characters, {len(tags)} elements)")
    if not ok:
        fails.append("raw")
    for flags, want_rc, want in (("", 0, ""), ("--stub-rewrite", 1, "text differs")):
        out = tempfile.mkdtemp(prefix="asis-selftest-")
        rc = subprocess.call([sys.executable, "-I", me, "run", "--no-isolation", "--browser", "stub",
                              f"--browser-args={flags}", "--settle", "0.3", "--timeout", "10", "--out", out],
                             stdout=subprocess.DEVNULL)
        try:
            with open(os.path.join(out, "page_asis.json")) as f:
                rep = json.load(f)
        except (OSError, ValueError):
            rep = {"failures": ["no report"]}
        text = " ".join(rep["failures"])
        ok = rc == want_rc and (want in text if want else not rep["failures"])
        print(f"  {'ok  ' if ok else 'FAIL'} stub {flags or 'as made'}: exit {rc} (want {want_rc}){' naming ' + repr(want) if want else ''}")
        if not ok:
            fails.append(flags)
    print(f"page_asis selftest: {'PASS' if not fails else 'FAIL'}")
    return 0 if not fails else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--selftest", action="store_true")
    sub = ap.add_subparsers(dest="cmd")
    r = sub.add_parser("run")
    r.add_argument("--browser", required=True)
    r.add_argument("--browser-args", default="--no-first-run --password-store=basic --window-size=1280,800")
    r.add_argument("--off-args", default="", help="flags that turn the blocker off (default: disable its extension in the profile)")
    r.add_argument("--page", help="a local HTML file (default: the built-in corpus page)")
    r.add_argument("--settle", type=float, default=10)
    r.add_argument("--timeout", type=float, default=30)
    r.add_argument("--no-isolation", dest="isolate", action="store_false")
    r.add_argument("--user")
    r.add_argument("--out")
    r.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.cmd != "run":
        ap.print_help()
        return 2
    if a.out:
        a.out = os.path.abspath(a.out)
        os.makedirs(a.out, exist_ok=True)
    if a.page:
        a.page = os.path.abspath(a.page)
    return L.run_isolated(__file__, L.fix_out(sys.argv[1:], a.out), a.isolate, a.user, lambda iso: inner(a, iso))


if __name__ == "__main__":
    sys.exit(main())
