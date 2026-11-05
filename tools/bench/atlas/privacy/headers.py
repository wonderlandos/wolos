# SPDX-License-Identifier: GPL-2.0-or-later
"""headers.py: Do Not Track and Global Privacy Control are on from the start, for every profile
(plan AT-7, reel scene 5 / R05, promises P14 and P15; overlay patches privacy/0001 and 0002).

How: a local echo server records the headers of every request. The browser opens one page, which
fetches a resource, starts a dedicated worker that fetches another, and posts what
navigator.globalPrivacyControl says in the document and in the worker. Done twice: on a fresh
profile, then on an existing one (the fresh profile reopened, or one made first by --old-browser,
or a copy of --profile-dir). PASS when every request carries `DNT: 1` and `Sec-GPC: 1` and both
navigator values are true. By default inside the egress namespace (no network at all);
--no-isolation runs on the host as is.

  python3 -I headers.py run --browser 'CMD' [--old-browser 'CMD'] [--out DIR]
  python3 -I headers.py --selftest
Exit: 0 pass, 1 a header or value is missing, 2 cannot run.
"""
import argparse
import json
import os
import shlex
import shutil
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import privlib as L  # noqa: E402

PROBE = {"report": "report", "fetch": ["probe-doc"], "worker": "w.js", "worker_fetch": ["probe-worker"]}
DOC_JS = """
(async () => {
  const rep = {doc_gpc: navigator.globalPrivacyControl, doc_dnt: navigator.doNotTrack};
  try { await fetch('probe-doc', {cache: 'no-store'}); } catch (e) { rep.doc_fetch_error = String(e); }
  const w = new Worker('w.js');
  const wr = await new Promise(res => { w.onmessage = e => res(e.data); setTimeout(() => res({timeout: true}), 8000); });
  rep.worker_gpc = wr.gpc; rep.worker_dnt = wr.dnt; rep.worker_timeout = !!wr.timeout;
  rep.agent = navigator.userAgent;
  await fetch('report', {method: 'POST', body: JSON.stringify(rep)});
  document.getElementById('s').textContent = 'reported';
})();
"""
WORKER_JS = b"""
fetch('probe-worker', {cache: 'no-store'}).catch(() => {}).finally(() =>
  postMessage({gpc: navigator.globalPrivacyControl, dnt: navigator.doNotTrack}));
"""
CHECKED = ("doc", "probe-doc", "w.js", "probe-worker")


def routes(reports):
    def page(h, rec):
        last = rec["path"].split("?")[0].rsplit("/", 1)[-1]
        phase = rec["path"].split("/")[2] if rec["path"].count("/") >= 3 else "?"
        if last == "doc":
            body = ("<!doctype html><meta charset=utf-8><title>Header probe " + phase + "</title>"
                    "<script type=application/json id=probe>" + json.dumps(PROBE) + "</script>"
                    "<h1>Header probe</h1><p id=s>probing</p><script>" + DOC_JS + "</script>").encode()
            return 200, "text/html", body
        if last == "w.js":
            return 200, "text/javascript", WORKER_JS
        if last == "report":
            try:
                reports[phase] = json.loads(rec["body"])
            except ValueError:
                reports[phase] = {"bad_report": rec["body"][:200]}
            return 200, "text/plain", b"ok"
        return 200, "text/plain", b"echo"
    return {"/p/*": page, "/blank": ("text/html", b"<!doctype html><title>blank</title>")}


def judge(phase, reqs, rep):
    """Failures of one phase: requests without the headers, navigator values that are not true."""
    bad = []
    seen = {}
    for r in reqs:
        last = r["path"].split("?")[0].rsplit("/", 1)[-1]
        if not r["path"].startswith(f"/p/{phase}/") or last not in CHECKED:
            continue
        seen[last] = r
        if r["headers"].get("dnt") != "1":
            bad.append(f"{phase}: request {last} has DNT {r['headers'].get('dnt')!r}, not '1'")
        if r["headers"].get("sec-gpc") != "1":
            bad.append(f"{phase}: request {last} has Sec-GPC {r['headers'].get('sec-gpc')!r}, not '1'")
    for want in CHECKED:
        if want not in seen:
            bad.append(f"{phase}: no request for {want} (page or worker did not run)")
    if rep is None:
        bad.append(f"{phase}: the page did not report")
    else:
        for k in ("doc_gpc", "worker_gpc"):
            if rep.get(k) is not True:
                bad.append(f"{phase}: navigator.globalPrivacyControl in the {'document' if k == 'doc_gpc' else 'worker'} is {rep.get(k)!r}, not true")
    return bad, {k: {"dnt": v["headers"].get("dnt"), "sec-gpc": v["headers"].get("sec-gpc")} for k, v in seen.items()}


def run_phase(iso, a, srv, reports, phase, argv, profile, env):
    b = L.Browser(argv, profile, prefix=iso.prefix(), extra=shlex.split(a.browser_args),
                  urls=[srv.url(f"/p/{phase}/doc")], logfile=os.path.join(iso.tmp, f"browser-{phase}.log"), env=env)
    b.start()
    ok = L.wait_for(lambda: phase in reports, a.timeout)
    time.sleep(1)
    b.stop()
    return ok


def inner(a, iso):
    reports = {}
    srv = L.Server(routes(reports)).start()
    env = L.person_env(iso)
    argv = L.browser_argv(a.browser)
    fresh = iso.mkdir("profile-fresh")
    out = {"tool": "headers", "browser": a.browser, "isolation": iso.facts, "phases": {}}
    fails = []
    run_phase(iso, a, srv, reports, "fresh", argv, fresh, env)
    if a.profile_dir:
        existing = iso.mkdir("profile-existing")
        shutil.rmtree(existing)
        shutil.copytree(a.profile_dir, existing, symlinks=True)
        how = f"copy of {a.profile_dir}"
    elif a.old_browser:
        existing = iso.mkdir("profile-existing")
        b = L.Browser(L.browser_argv(a.old_browser), existing, prefix=iso.prefix(), extra=shlex.split(a.browser_args),
                      urls=[srv.url("/blank")], logfile=os.path.join(iso.tmp, "browser-old.log"), env=env).start()
        L.wait_for(lambda: any(r["path"] == "/blank" for r in srv.taken()), a.timeout)
        time.sleep(3)
        b.stop()
        how = f"made by {a.old_browser}"
    else:
        existing = fresh
        how = "the fresh profile, reopened"
    run_phase(iso, a, srv, reports, "existing", argv, existing, env)
    reqs = srv.taken()
    srv.stop()
    for phase in ("fresh", "existing"):
        bad, hdrs = judge(phase, reqs, reports.get(phase))
        out["phases"][phase] = {"report": reports.get(phase), "headers": hdrs, "failures": bad}
        fails += bad
    out["existing_profile"] = how
    out["verdict"] = "FAIL" if fails else "PASS"
    out["failures"] = fails
    L.write_json(a.out and os.path.join(a.out, "headers.json"), out)
    print(f"HEADERS {out['verdict']}: fresh and existing profile ({how}), document and worker")
    for ph in ("fresh", "existing"):
        p = out["phases"][ph]
        r = p["report"] or {}
        print(f"  {ph}: " + ", ".join(f"{k} DNT={v['dnt']} Sec-GPC={v['sec-gpc']}" for k, v in sorted(p["headers"].items()))
              + f"; navigator.globalPrivacyControl document={r.get('doc_gpc')!r} worker={r.get('worker_gpc')!r}")
    for f in fails:
        print(f"  FAIL {f}")
    L.cleanup(iso, a.keep)
    return 0 if not fails else 1


def selftest():
    me = os.path.abspath(__file__)
    import subprocess
    fails = []
    print("headers selftest: stub browser over loopback (no isolation needed)")
    for flags, want_rc, want in (("", 0, []), ("--stub-dnt=0", 1, ["DNT"]), ("--stub-gpc=0", 1, ["Sec-GPC", "globalPrivacyControl"])):
        out = tempfile.mkdtemp(prefix="headers-selftest-")
        rc = subprocess.call([sys.executable, "-I", me, "run", "--no-isolation", "--browser", "stub",
                              f"--browser-args={flags}", "--timeout", "20", "--out", out], stdout=subprocess.DEVNULL)
        try:
            with open(os.path.join(out, "headers.json")) as f:
                rep = json.load(f)
        except (OSError, ValueError):
            rep = {"failures": ["no report"]}
        text = " ".join(rep["failures"])
        ok = rc == want_rc and all(w in text for w in want) and (want or not rep["failures"])
        print(f"  {'ok  ' if ok else 'FAIL'} stub {flags or 'defaults'}: exit {rc} (want {want_rc}), "
              f"{len(rep['failures'])} failure(s){' naming ' + '/'.join(want) if want else ''}")
        if not ok:
            fails.append(flags)
        shutil.rmtree(out, ignore_errors=True)
    print(f"headers selftest: {'PASS' if not fails else 'FAIL'}")
    return 0 if not fails else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--selftest", action="store_true")
    sub = ap.add_subparsers(dest="cmd")
    r = sub.add_parser("run")
    r.add_argument("--browser", required=True)
    r.add_argument("--browser-args", default="--no-first-run --password-store=basic")
    r.add_argument("--old-browser", help="make the existing profile with this browser first")
    r.add_argument("--profile-dir", help="existing profile to copy for the second phase")
    r.add_argument("--timeout", type=int, default=60)
    r.add_argument("--no-isolation", dest="isolate", action="store_false")
    r.add_argument("--user")
    r.add_argument("--out")
    r.add_argument("--keep", action="store_true", help="keep the temporary profiles")
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
