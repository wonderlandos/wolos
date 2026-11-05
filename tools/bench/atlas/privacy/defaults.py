# SPDX-License-Identifier: GPL-2.0-or-later
"""defaults.py: the privacy defaults as the browser itself reports them (plan AT-7, decision D2,
promises P14, P15, P17, P20). Reads the internal preferences page (defaults included) and the Local
State page of a fresh profile over a DevTools pipe (no port, no network) and checks every value we
promise: address bar suggestions off and network prediction "never" (D2), Do Not Track and Global
Privacy Control on, every services switch off, usage statistics off, crash reports kept on the
machine. The fingerprint noise switch is reported, not judged (D4 keeps it on).

Keys are matched by their name or suffix so the check does not depend on the inherited pref
prefix. A key the page does not show is "not shown" (not a pass); --strict makes it a failure.

  python3 -I defaults.py run --browser 'CMD' [--scheme chrome] [--out DIR] [--strict]
  python3 -I defaults.py --selftest
Exit: 0 pass, 1 a default is wrong (or not shown with --strict), 2 cannot run.
"""
import argparse
import json
import os
import shlex
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import privlib as L  # noqa: E402

# (page, key or *suffix, expected value, what it means[, "absent ok"]). "absent ok": the feature is
# compiled out of the build (safe_browsing_mode=0), so a key that is not there is as good as off.
EXPECT = [
    ("prefs", "search.suggest_enabled", False, "address bar suggestions off (D2)"),
    ("prefs", "net.network_prediction_options", 2, "network prediction never (D2)"),
    ("prefs", "enable_do_not_track", True, "Do Not Track on (P14)"),
    ("prefs", "*.global_privacy_control", True, "Global Privacy Control on (P15)"),
    ("prefs", "*.services.enabled", False, "the inherited web services off (P20)"),
    ("prefs", "*.services.bangs", False, "search keyword shortcuts off (P20)"),
    ("prefs", "*.services.ext_proxy", False, "extension download proxy off (P20)"),
    ("prefs", "*.services.spellcheck_files", False, "spellcheck dictionary downloads off (P20)"),
    ("prefs", "*.services.browser_updates", False, "browser update checks off (P33)"),
    ("prefs", "*.services.ublock_assets", False, "filter list downloads off (P16, P20)"),
    ("prefs", "safebrowsing.enabled", False, "safe browsing lookups off (no service)", "absent ok"),
    ("prefs", "alternate_error_pages.enabled", False, "error page suggestions off"),
    ("local", "user_experience_metrics.reporting_enabled", False, "usage statistics off (P17)"),
    ("local", "*.crash_reporting.mode", -1, "crash reports stay on the machine (P17)"),
]
# words of the keys listed in the report next to the verdict (the evidence, and a way to fix EXPECT)
WORDS = ("suggest", "predict", "do_not_track", "privacy_control", "gpc", "services", "safebrowsing", "metrics",
         "crash", "noise", "report", "update", "translate", "variations")
REPORT_ONLY = [("prefs", "*.noise.enabled", "fingerprint noise (D4: kept on)"),
               ("prefs", "*.services.user_consented", "services consent given")]


def flatten(d, path="", out=None):
    """{"a": {"b": {"value": 1, "metadata": [...]}}} -> {"a.b": 1} (the internals pages wrap each pref)."""
    out = {} if out is None else out
    if isinstance(d, dict) and "value" in d and isinstance(d.get("metadata"), list):
        out[path] = d["value"]
    elif isinstance(d, dict):
        for k, v in d.items():
            flatten(v, f"{path}.{k}" if path else k, out)
    else:
        out[path] = d
    return out


def lookup(flat, key):
    if key.startswith("*"):
        hits = {k: v for k, v in flat.items() if k.endswith(key[1:])}
        return hits
    return {key: flat[key]} if key in flat else {}


def judge(pages, strict):
    rows, fails = [], []
    for page, key, want, what, *opt in EXPECT:
        hits = lookup(pages.get(page) or {}, key)
        if not hits and opt:
            rows.append({"key": key, "want": want, "got": None, "state": "absent", "what": what + " (not registered: compiled out)"})
            continue
        if not hits:
            rows.append({"key": key, "want": want, "got": None, "state": "not shown", "what": what})
            if strict:
                fails.append(f"{key}: not shown on the {page} page ({what})")
            continue
        for k, v in hits.items():
            ok = v == want and type(v) is type(want)
            rows.append({"key": k, "want": want, "got": v, "state": "ok" if ok else "WRONG", "what": what})
            if not ok:
                fails.append(f"{k} is {v!r}, want {want!r} ({what})")
    info = {}
    for page, key, what in REPORT_ONLY:
        for k, v in lookup(pages.get(page) or {}, key).items():
            info[k] = {"value": v, "what": what}
    return rows, fails, info


def read_page(pipe, schemes, name):
    for sch in schemes:
        try:
            text = pipe.page_text(f"{sch}://{name}", wait=2)
        except RuntimeError as e:
            L.log(f"defaults: {sch}://{name}: {e}")
            continue
        try:
            return sch, json.loads(text)
        except (TypeError, ValueError):
            L.log(f"defaults: {sch}://{name} did not show JSON ({str(text)[:80]!r})")
    return None, None


def inner(a, iso):
    profile = iso.mkdir("profile")
    # The internals pages are refused until this Local State key is set (the person's switch on the
    # chrome-urls page). It is the only value written into the profile; none of the keys judged below.
    with open(os.path.join(profile, "Local State"), "w", encoding="utf-8") as f:
        json.dump({"internal_only_uis_enabled": True}, f)
    pipe = L.CdpPipe()
    b = L.Browser(L.browser_argv(a.browser), profile, prefix=iso.prefix(), extra=shlex.split(a.browser_args),
                  urls=["about:blank"], logfile=os.path.join(iso.tmp, "browser.log"), env=L.person_env(iso), pipe=pipe)
    b.start()
    time.sleep(a.settle)
    schemes = [s for s in a.scheme.split(",") if s]
    pages, used = {}, {}
    for page, name in (("prefs", "prefs-internals"), ("local", "local-state")):
        used[page], pages[page] = read_page(pipe, schemes, name)
        pages[page] = flatten(pages[page]) if pages[page] is not None else None
    b.stop()
    rows, fails, info = judge(pages, a.strict)
    for page in ("prefs", "local"):
        if pages[page] is None:
            fails.append(f"the {page} page could not be read (schemes tried: {', '.join(schemes)})")
    if not any(r["state"] == "ok" for r in rows):
        fails.append("no expected key was found on the pages: nothing is proven (key names changed?)")
    out = {"tool": "defaults", "browser": a.browser, "isolation": iso.facts, "pages": used, "rows": rows,
           "report_only": info, "failures": fails, "verdict": "FAIL" if fails else "PASS",
           "keys_seen": {p: len(v or {}) for p, v in pages.items()},
           "privacy_keys": {p: {k: v for k, v in (pages[p] or {}).items() if any(w in k.lower() for w in WORDS)}
                            for p in pages}}
    L.write_json(a.out and os.path.join(a.out, "defaults.json"), out)
    print(f"DEFAULTS {out['verdict']}: {sum(1 for r in rows if r['state'] == 'ok')} ok, "
          f"{sum(1 for r in rows if r['state'] == 'WRONG')} wrong, {sum(1 for r in rows if r['state'] == 'not shown')} not shown")
    for r in rows:
        print(f"  {r['state']:9} {r['key']} = {r['got']!r} (want {r['want']!r}): {r['what']}")
    for k, v in info.items():
        print(f"  info      {k} = {v['value']!r}: {v['what']}")
    for f in fails:
        print(f"  FAIL {f}")
    L.cleanup(iso, a.keep)
    return 0 if not fails else 1


def selftest():
    import subprocess
    import tempfile
    me = os.path.abspath(__file__)
    fails = []
    print("defaults selftest: stub browser over a DevTools pipe (no isolation needed)")
    for mode, want_rc, want in (("clean", 0, ""), ("leaky", 1, "search.suggest_enabled")):
        out = tempfile.mkdtemp(prefix="defaults-selftest-")
        rc = subprocess.call([sys.executable, "-I", me, "run", "--no-isolation", "--browser", f"stub:{mode}",
                              "--settle", "0.5", "--out", out], stdout=subprocess.DEVNULL)
        try:
            with open(os.path.join(out, "defaults.json")) as f:
                rep = json.load(f)
        except (OSError, ValueError):
            rep = {"failures": ["no report"], "rows": []}
        text = " ".join(rep["failures"])
        ok = rc == want_rc and (want in text if want else not rep["failures"]) and rep["rows"]
        print(f"  {'ok  ' if ok else 'FAIL'} stub {mode}: exit {rc} (want {want_rc}){', names ' + want if want else ''}")
        if not ok:
            fails.append(mode)
    print(f"defaults selftest: {'PASS' if not fails else 'FAIL'}")
    return 0 if not fails else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--selftest", action="store_true")
    sub = ap.add_subparsers(dest="cmd")
    r = sub.add_parser("run")
    r.add_argument("--browser", required=True)
    r.add_argument("--browser-args", default="--no-first-run --password-store=basic")
    r.add_argument("--scheme", default="chrome,atlas", help="internal page schemes to try, in order")
    r.add_argument("--settle", type=float, default=6.0, help="seconds after start before reading")
    r.add_argument("--strict", action="store_true")
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
    return L.run_isolated(__file__, L.fix_out(sys.argv[1:], a.out), a.isolate, a.user, lambda iso: inner(a, iso))


if __name__ == "__main__":
    sys.exit(main())
