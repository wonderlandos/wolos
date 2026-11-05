#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Orchestrator of the Atlas Drive vs Playwright benchmark (docs/bench/atlas-drive.md).

Runs both tools N times, interleaved (so drift in the machine hits both), each run with a fresh browser
profile, and writes every raw run plus medians and [min-max] to a JSON file.
  bench.py --runs 10 --browser BIN --drive ATLAS_DRIVE --bench BENCH_EXAMPLE --pw PW_DIR --out results.json
"""
import argparse, json, statistics, subprocess, sys, time

# wolos.org (port 8092) and the hub (port 8093) are the same files served from two origins. Navigating
# between two pages of ONE origin (127.0.0.1:8092) was cancelled by the browser (net::ERR_ABORTED) in 8 of
# 10 Playwright runs and made Drive wait 3 s for the retry; the cause was not found (no service worker, no
# self-redirect in that page). It is the browser build or the site, not the driver, so it is kept out.
PAGES = {
    "app": "http://127.0.0.1:8091/app.html",
    "wolos": "http://127.0.0.1:8092/index.html",
    "hub": "http://127.0.0.1:8093/hub/index.html",
    "heavy": "http://127.0.0.1:8091/wiki.html",
    "fed": "fed://home.fed/",
    "chrome": "chrome://version/",
}

def run_drive(a):
    spec = ",".join(f"{k}={v}" for k, v in PAGES.items())
    r = subprocess.run([a.bench, "--bin", a.drive, "--browser", a.browser, "--pages", spec, "--app", "app"], capture_output=True, text=True, timeout=300)
    if r.returncode: raise SystemExit("drive run failed: " + r.stderr[-500:])
    return json.loads(r.stdout.strip().splitlines()[-1])

def run_pw(a):
    spec = ",".join(f"{k}={v}" for k, v in PAGES.items())
    r = subprocess.run(["node", f"{a.pw}/pw.mjs", a.browser, spec, "app"], capture_output=True, text=True, timeout=300, cwd=a.pw)
    if r.returncode: raise SystemExit("pw run failed: " + r.stderr[-800:])
    return json.loads(r.stdout.strip().splitlines()[-1])

def stat(xs):
    xs = [x for x in xs if x is not None]
    if not xs: return None
    return {"median": round(statistics.median(xs), 2), "min": round(min(xs), 2), "max": round(max(xs), 2), "n": len(xs)}

def flat(run):
    """one number per metric from one run"""
    m = {"cold_ms": run["cold_ms"], "click_ms": statistics.median(run["click_ms"]), "click_locator_ms": statistics.median(run["click_locator_ms"]),
         "clicks100_ms": run["clicks100_ms"], "shot_ms": run["shot_ms"], "shot_bytes": run["shot_bytes"],
         "task_ms": run["task"]["ms"], "task_tokens": run["task"]["tokens"], "task_toast_seen": run["task"]["toast_seen"],
         "browser_rss_mb": run["browser_rss_mb"], "driver_rss_mb": run["driver_rss_mb"], "idle_wakeups_per_s": run["idle_wakeups_per_s"]}
    for k in ("goto_warm_ms", "warm_first_ms"):
        if k in run: m[k] = run[k]
    if "task_known" in run:
        m["task_known_ms"] = run["task_known"]["ms"]; m["task_known_tokens"] = run["task_known"]["tokens"]
    for page, p in run["pages"].items():
        for k, v in p.items():
            if isinstance(v, (int, float)): m[f"{page}.{k}"] = v
        # a load the browser itself aborted (net::ERR_ABORTED): Playwright's goto fails (no number),
        # Drive retries (goto_retried). goto_clean_ms is the load of runs the browser did not abort.
        if "goto_ms" in p and not p.get("goto_retried"): m[f"{page}.goto_clean_ms"] = p["goto_ms"]
        m[f"{page}.goto_aborted"] = 1 if (p.get("goto_retried") or "error" in p) else 0
    return m

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=10)
    ap.add_argument("--browser", required=True); ap.add_argument("--drive", required=True)
    ap.add_argument("--bench", required=True); ap.add_argument("--pw", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    raw = {"drive": [], "pw": []}
    for i in range(a.runs):
        order = ["drive", "pw"] if i % 2 == 0 else ["pw", "drive"]
        for who in order:
            t = time.time()
            raw[who].append(run_drive(a) if who == "drive" else run_pw(a))
            print(f"run {i+1}/{a.runs} {who} {time.time()-t:.0f}s", file=sys.stderr, flush=True)
    summary = {}
    for who in raw:
        rows = [flat(r) for r in raw[who]]
        summary[who] = {k: stat([r.get(k) for r in rows]) for k in sorted({k for r in rows for k in r})}
    try: pwv = json.load(open(f"{a.pw}/node_modules/playwright/package.json"))["version"]
    except Exception: pwv = ""
    json.dump({"runs": a.runs, "pages": PAGES, "pw_version": pwv, "raw": raw, "summary": summary}, open(a.out, "w"), indent=1)
    print("wrote", a.out)

main()
