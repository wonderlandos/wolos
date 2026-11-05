# SPDX-License-Identifier: GPL-2.0-or-later
"""blocklist.py: tracker blocking from a list built into the browser, working with no network
(plan AT-16, reel scene 5 / R05, promise P16; journey J-A10).

  run       A local page loads six well-known tracker URLs (each covered by a rule of a bundled
            list, checked with --assets) and one neutral control. Every host name is mapped to the
            local server (--host-resolver-rules), inside the egress namespace, so nothing can leave.
            Phase "on": a fresh profile; PASS when no tracker request reaches the server and the
            control does. Phase "off": a fresh profile with the blocker switched off (--off-args, a
            stand-in until the first-run switch exists); PASS when the trackers DO load (the test
            sees the difference, so "blocked" means blocked by the list).
  manifest  The pinned list manifest: every list the blocker ships, on by default or not, its file,
            size, rule count, version or date, and licence (from the list header), from an assets
            directory (third_party/ublock of the prepared tree, or the pinned release archive).
  --selftest  the stub browser, both phases, and the rule matcher.
Exit: 0 pass, 1 a check failed, 2 cannot run.
"""
import argparse
import json
import os
import re
import shlex
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import privlib as L  # noqa: E402

GIF = bytes.fromhex("47494638396101000100800000000000ffffff21f90401000000002c00000000010001000002024401003b")
# (tag, host, path): third-party requests from the 127.0.0.1 page. Plain http, so no host may be on the
# browser's HSTS preload list (www.google-analytics.com and bat.bing.com are: the browser upgrades
# them to https and the local server never sees the request, blocker or not).
TRACKERS = [
    ("script", "static.criteo.net", "/js/ld/ld.js"),
    ("img", "googleads.g.doubleclick.net", "/pagead/viewthroughconversion/1/?guid=ON"),
    ("img", "match.adsrvr.org", "/track/cmf/generic"),
    ("script", "static.hotjar.com", "/c/hotjar-1.js?sv=6"),
    ("script", "pixel.quantserve.com", "/quant.js"),
    ("img", "sb.scorecardresearch.com", "/b?c1=2&c2=1"),
]
CONTROL = ("img", "static.blocklist.test", "/neutral.gif")
DEFAULT_OFF = ""  # no flag turns the blocker off; the stand-in is a profile seed (privlib.UBLOCK_OFF_PREFS)


def page(port):
    tags = []
    for tag, host, path in TRACKERS + [CONTROL]:
        url = f"http://{host}:{port}{path}"
        tags.append(f"<script src='{url}'></script>" if tag == "script" else f"<img src='{url}' width=1 height=1 alt=''>")
    return ("<!doctype html><meta charset=utf-8><title>Tracker page</title><h1>Tracker page</h1>"
            "<p>Six trackers and one neutral image.</p>" + "".join(tags)).encode()


# ---------------------------------------------------------------------------------------------
# Rules: enough of the filter syntax to show which bundled rule covers each tracker URL

RULE = re.compile(r"^\|\|([a-z0-9.-]+)\^(\*?[^$]*)?(?:\$(.*))?$")


def rule_covers(rule, host, path, rtype):
    m = RULE.match(rule)
    if not m:
        if re.match(r"^(0\.0\.0\.0|127\.0\.0\.1)\s+\S+", rule) or re.match(r"^[a-z0-9.-]+\.[a-z]+$", rule):
            h = rule.split()[-1]
            return host == h or host.endswith("." + h)
        return False
    dom, rest, opts = m.group(1), (m.group(2) or "").lstrip("*"), m.group(3) or ""
    if not (host == dom or host.endswith("." + dom)):
        return False
    if rest and rest not in path:
        return False
    types = {"script", "image", "xmlhttprequest", "xhr", "stylesheet", "css", "subdocument", "frame", "object",
             "ping", "media", "font", "other", "websocket"}
    mine = {"script": "script", "img": "image"}[rtype]
    pos, neg = set(), set()
    for o in filter(None, opts.split(",")):
        if o in types:
            pos.add(o)
        elif o.startswith("~") and o[1:] in types:
            neg.add(o[1:])
        elif o not in ("third-party", "3p", "important", "~first-party", "~1p"):
            return False  # any other option narrows the rule in ways this matcher does not model
    if (pos and mine not in pos) or mine in neg:
        return False
    return True


def lists_of(assets):
    """(key, title, file, default_on, entry) of every list in assets.json with a local file."""
    with open(os.path.join(assets, "assets.json")) as f:
        d = json.load(f)
    out = []
    root = os.path.dirname(os.path.abspath(assets))
    for key, v in d.items():
        cu = v.get("contentURL", [])
        cu = [cu] if isinstance(cu, str) else cu
        local = [u for u in cu if not u.startswith(("http:", "https:")) and os.path.exists(os.path.join(root, u))]
        out.append((key, v.get("title") or key, local[0] if local else None, not v.get("off"), v))
    return out, root


def coverage(assets):
    """For each tracker: the first bundled, default-on list rule that covers it."""
    lists, root = lists_of(assets)
    cov = {}
    for key, title, path, on, v in lists:
        if not path or not on or v.get("content") != "filters":
            continue
        with open(os.path.join(root, path), encoding="utf-8", errors="replace") as f:
            rules = [ln.strip() for ln in f if ln.strip() and not ln.startswith(("!", "#", "["))]
        for tag, host, p in TRACKERS:
            if host in cov:
                continue
            for r in rules:
                if rule_covers(r, host, p, tag):
                    cov[host] = {"list": key, "rule": r[:120]}
                    break
    return cov


# ---------------------------------------------------------------------------------------------
# Manifest

KNOWN = {  # list key -> (public name, licence of the list as published)
    "ublock-filters": ("uBlock filters: Ads", "GPL-3.0 (uAssets)"),
    "ublock-badware": ("uBlock filters: Badware risks", "GPL-3.0 (uAssets)"),
    "ublock-privacy": ("uBlock filters: Privacy", "GPL-3.0 (uAssets)"),
    "ublock-unbreak": ("uBlock filters: Unbreak", "GPL-3.0 (uAssets)"),
    "ublock-quick-fixes": ("uBlock filters: Quick fixes", "GPL-3.0 (uAssets)"),
    "ublock-badlists": ("uBlock bad-lists index", "GPL-3.0 (uAssets)"),
    "easylist": ("EasyList", "GPL-3.0-or-later or CC BY-SA 3.0 (dual)"),
    "easyprivacy": ("EasyPrivacy", "GPL-3.0-or-later or CC BY-SA 3.0 (dual)"),
    "urlhaus-1": ("Online Malicious URL Blocklist", "CC0-1.0 (LICENSE.md in the bundle)"),
    "plowe-0": ("Peter Lowe's ad and tracking server list", "McRae GPL 4.r53: list owner asks for non-commercial use (README.md in the bundle)"),
    "public_suffix_list.dat": ("Public Suffix List", "MPL-2.0"),
    "assets.json": ("asset index", "GPL-3.0 (blocker)"),
}
HEADER = re.compile(r"^[!#/ ]*\s*(Title|Version|VERSION|Last modified|Updated|Licen[cs]e|Expires)\s*:\s*(.+)$", re.I)


def manifest(assets):
    lists, root = lists_of(assets)
    rows = []
    for key, title, path, on, v in lists:
        if not path:
            if on and v.get("content") == "filters":
                rows.append({"key": key, "name": KNOWN.get(key, (title,))[0], "default_on": True, "bundled": False,
                             "file": None, "note": "selected by default but not bundled: empty offline (downloads are off)"})
            continue
        fp = os.path.join(root, path)
        hdr = {}
        with open(fp, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
        for ln in lines[:40]:
            m = HEADER.match(ln)
            if m and m.group(1).lower() not in hdr:
                hdr[m.group(1).lower()] = m.group(2).strip()
        rules = sum(1 for ln in lines if ln.strip() and not ln.lstrip().startswith(("!", "#", "[", "//")))
        name, lic = KNOWN.get(key, (f"inherited list ({os.path.basename(path)})", "see the list header"))
        rows.append({"key": key, "name": name, "default_on": on, "bundled": True, "file": path,
                     "bytes": os.path.getsize(fp), "lines": len(lines), "rules": rules,
                     "version": hdr.get("version") or hdr.get("last modified") or hdr.get("updated") or "none in the header",
                     "licence": lic, "licence_header": hdr.get("license") or hdr.get("licence"),
                     "kind": v.get("content")})
    return rows


def manifest_md(rows):
    out = ["| List | On by default | File | Size | Rules | Version or date | Licence |", "|---|---|---|---|---|---|---|"]
    for r in rows:
        if not r["bundled"]:
            out.append(f"| {r['name']} | yes | not bundled | 0 | 0 | | {r['note']} |")
            continue
        out.append(f"| {r['name']} | {'yes' if r['default_on'] else 'no'} | `{r['file']}` | {r['bytes'] / 1024:.0f} KiB | "
                   f"{r['rules'] if r['kind'] == 'filters' else 'n/a'} | {r['version']} | {r['licence']} |")
    return "\n".join(out)


# ---------------------------------------------------------------------------------------------
# Run

def phase(iso, a, srv, name, extra, attempts, prefs=None):
    profile = iso.mkdir(f"profile-{name}")
    if prefs:
        L.seed_prefs(profile, prefs)
    pipe = L.CdpPipe()
    rules = f"--host-resolver-rules=MAP * 127.0.0.1, EXCLUDE localhost"
    b = L.Browser(L.browser_argv(a.browser), profile, prefix=iso.prefix(),
                  extra=shlex.split(a.browser_args) + [rules] + extra, urls=[srv.url("/blank")],
                  logfile=os.path.join(iso.tmp, f"browser-{name}.log"), env=L.person_env(iso), pipe=pipe)
    b.start()
    time.sleep(a.settle)
    tries = []
    for i in range(attempts):
        t0 = time.time()
        try:
            pipe.page_text(srv.url(f"/track.html?{name}-{i}"), wait=a.wait, expr="document.readyState")
        except RuntimeError as e:
            tries.append({"error": str(e)})
            continue
        got = [r for r in srv.taken() if r["t"] >= t0]
        loaded = sorted({r["host"].split(":")[0] for r in got} - {"127.0.0.1"})
        tries.append({"loaded_hosts": loaded,
                      "page_loaded": any(r["path"].startswith(f"/track.html?{name}-{i}") for r in got)})
        if name == "on" and not set(loaded) & {h for _, h, _ in TRACKERS}:
            break
        if name == "off":
            break
        time.sleep(a.wait)
    b.stop()
    return tries


def inner(a, iso):
    routes = {"/blank": ("text/html", b"<!doctype html><title>blank</title>")}
    srv = None

    def track(h, rec):
        return 200, "text/html", page(srv.port)

    def anything(h, rec):
        p = rec["path"].split("?")[0]
        return 200, ("text/javascript" if p.endswith(".js") else "image/gif"), (b"/* ok */" if p.endswith(".js") else GIF)
    routes["/track.html"] = track
    routes["/*"] = anything
    srv = L.Server(routes).start()
    on = phase(iso, a, srv, "on", [], a.attempts)
    off_prefs = json.loads(a.off_prefs) if a.off_prefs else (L.UBLOCK_OFF_PREFS if not a.off_args else None)
    off = phase(iso, a, srv, "off", shlex.split(a.off_args), 1, off_prefs)
    srv.stop()
    trackers = {h for _, h, _ in TRACKERS}
    fails = []
    first = on[0] if on else {}
    last = on[-1] if on else {}
    if not last.get("page_loaded"):
        fails.append("on: the tracker page did not load")
    leaked = sorted(set(last.get("loaded_hosts", [])) & trackers)
    if leaked:
        fails.append(f"on: trackers loaded on a fresh profile: {', '.join(leaked)}")
    if CONTROL[1] not in last.get("loaded_hosts", []):
        fails.append("on: the neutral control image did not load (the page or the mapping is broken)")
    o = off[0] if off else {}
    not_loaded = sorted(trackers - set(o.get("loaded_hosts", [])))
    if not_loaded:
        fails.append(f"off: with the blocker off these still did not load: {', '.join(not_loaded)} "
                     f"(the off stand-in {a.off_args or off_prefs!r} did not turn the blocker off, or the page broke)")
    rep = {"tool": "blocklist", "browser": a.browser, "isolation": iso.facts, "trackers": [h for _, h, _ in TRACKERS],
           "control": CONTROL[1], "on": on, "off": off, "off_args": a.off_args, "off_prefs": off_prefs,
           "blocked_from_first_load": bool(first) and not (set(first.get("loaded_hosts", [])) & trackers),
           "failures": fails, "verdict": "FAIL" if fails else "PASS"}
    if a.assets:
        rep["coverage"] = coverage(a.assets)
        missing = trackers - set(rep["coverage"])
        if missing:
            fails.append(f"no default-on bundled rule covers: {', '.join(sorted(missing))}")
            rep["verdict"] = "FAIL"
    L.write_json(a.out and os.path.join(a.out, "blocklist.json"), rep)
    print(f"BLOCKLIST {rep['verdict']}: on: {len(on)} load(s), last loaded {last.get('loaded_hosts')}; "
          f"off: loaded {o.get('loaded_hosts')}; blocked from the first load: {rep['blocked_from_first_load']}")
    for h, c in (rep.get("coverage") or {}).items():
        print(f"  covered {h}: {c['list']} {c['rule']}")
    for f in fails:
        print(f"  FAIL {f}")
    L.cleanup(iso, a.keep)
    return 0 if not fails else 1


def selftest():
    import subprocess
    import tempfile
    me = os.path.abspath(__file__)
    fails = []

    def check(c, what):
        print(f"  {'ok  ' if c else 'FAIL'} {what}")
        if not c:
            fails.append(what)
    print("blocklist selftest")
    check(rule_covers("||google-analytics.com^$script,third-party", "www.google-analytics.com", "/a.js", "script"), "||host^$script covers a script")
    check(not rule_covers("||google-analytics.com^$script", "www.google-analytics.com", "/a.gif", "img"), "...but not an image")
    check(not rule_covers("||doubleclick.net^$domain=x.de", "g.doubleclick.net", "/", "img"), "domain= rules are not counted")
    check(rule_covers("0.0.0.0 bat.bing.com", "bat.bing.com", "/", "img"), "hosts-file lines count")
    check(not rule_covers("||com^$doc,ipaddress=1.2.3.4", "bat.bing.com", "/", "img"), "options it does not model are not counted")
    for flags, want_rc, want in (("", 0, ""), ("--stub-blocker=off", 1, "trackers loaded")):
        out = tempfile.mkdtemp(prefix="blocklist-selftest-")
        rc = subprocess.call([sys.executable, "-I", me, "run", "--no-isolation", "--browser", "stub",
                              f"--browser-args={flags}", "--settle", "0.3", "--wait", "0.5", "--attempts", "1",
                              "--out", out], stdout=subprocess.DEVNULL)
        try:
            with open(os.path.join(out, "blocklist.json")) as f:
                rep = json.load(f)
        except (OSError, ValueError):
            rep = {"failures": ["no report"]}
        text = " ".join(rep["failures"])
        check(rc == want_rc and (want in text if want else not rep["failures"]),
              f"stub {flags or 'blocker on'}: exit {rc} (want {want_rc}){' naming ' + repr(want) if want else ''}")
    print(f"blocklist selftest: {'PASS' if not fails else 'FAIL'}")
    return 0 if not fails else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--selftest", action="store_true")
    sub = ap.add_subparsers(dest="cmd")
    r = sub.add_parser("run")
    r.add_argument("--browser", required=True)
    r.add_argument("--browser-args", default="--no-first-run --password-store=basic")
    r.add_argument("--off-args", default=DEFAULT_OFF, help="flags that turn the blocker off (stand-in for the switch)")
    r.add_argument("--off-prefs", help="JSON for the Preferences of the off phase (default: disable the blocker extension)")
    r.add_argument("--assets", help="assets directory of the bundled blocker, to show which rule covers each tracker")
    r.add_argument("--settle", type=float, default=15, help="seconds after start before the first load")
    r.add_argument("--wait", type=float, default=4)
    r.add_argument("--attempts", type=int, default=3)
    r.add_argument("--no-isolation", dest="isolate", action="store_false")
    r.add_argument("--user")
    r.add_argument("--out")
    r.add_argument("--keep", action="store_true")
    m = sub.add_parser("manifest")
    m.add_argument("assets", help="assets directory (contains assets.json)")
    m.add_argument("--json")
    c = sub.add_parser("coverage", help="which bundled default-on rule covers each tracker of the test page (no browser)")
    c.add_argument("assets")
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.cmd == "manifest":
        rows = manifest(a.assets)
        if a.json:
            L.write_json(a.json, rows)
        print(manifest_md(rows))
        on = [r for r in rows if r.get("default_on") and r.get("bundled") and r.get("kind") == "filters"]
        print(f"\n{len(on)} filter lists on by default and bundled, {sum(r['rules'] for r in on)} rules, "
              f"{sum(r['bytes'] for r in on) / 1048576:.1f} MiB; "
              f"{sum(1 for r in rows if not r['bundled'])} selected by default with nothing bundled")
        return 0
    if a.cmd == "coverage":
        try:
            cov = coverage(a.assets)
        except (OSError, ValueError) as e:
            print(f"blocklist: cannot read the assets: {e}", file=sys.stderr)
            return 2
        for _, h, _ in TRACKERS:
            c = cov.get(h)
            print(f"  {'covered' if c else 'NOT COVERED'} {h}" + (f": {c['list']} {c['rule']}" if c else ""))
        return 0 if all(h in cov for _, h, _ in TRACKERS) else 1
    if a.cmd != "run":
        ap.print_help()
        return 2
    if a.assets and not os.path.isfile(os.path.join(a.assets, "assets.json")):
        print(f"blocklist: --assets {a.assets}: no assets.json there", file=sys.stderr)
        return 2
    if a.out:
        a.out = os.path.abspath(a.out)
        os.makedirs(a.out, exist_ok=True)
    if a.assets:
        a.assets = os.path.abspath(a.assets)
    return L.run_isolated(__file__, L.fix_out(sys.argv[1:], a.out), a.isolate, a.user, lambda iso: inner(a, iso))


if __name__ == "__main__":
    sys.exit(main())
