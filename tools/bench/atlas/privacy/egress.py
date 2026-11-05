# SPDX-License-Identifier: GPL-2.0-or-later
"""egress.py: "It talks to the sites you open. Nothing else." as a test that fails loudly (plan
AT-7, reel scene 6 / R06, promise P20; the canaries are ADR 0053's J-T1 leak canaries).

How: the browser runs in its own network namespace (privlib.Isolation) where the only interfaces
are loopback and a dummy with the default route. Every outgoing frame is captured; DNS goes to our
sink, which answers with a loopback address per name and records the TLS server name or HTTP request
sent there. Scenario: 10 local tabs (the last one is the canary page), typing in the address bar
(--omnibox-cmd, or --omnibox-handshake for a driver outside the guest), then --idle seconds.
PASS when nothing but loopback was sent and no DNS query was made, except the canaries the page
asked for; the canaries are also the positive control (a run that does not see them is blind and
fails). Any other target fails the run and is named.

  python3 -I egress.py run --browser 'CMD ARGS' [--idle 1800] [--user 1000] [--out DIR] ...
  python3 -I egress.py --selftest [--require-netns]
Exit: 0 pass, 1 egress found (or blind), 2 cannot isolate (fails closed).
"""
import argparse
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import privlib as L  # noqa: E402

TABS = 10
CANARIES = [
    {"kind": "fetch", "target": "https://canary-fetch.egress.test/c.js", "control": "dns"},
    {"kind": "fetch", "target": "http://192.0.2.11/c.gif", "control": "packet"},
    {"kind": "stun", "target": "stun:canary-stun.egress.test:3478"},
    {"kind": "stun", "target": "stun:192.0.2.12:3478"},
    {"kind": "webtransport", "target": "https://canary-wt.egress.test:4433/"},
    {"kind": "webtransport", "target": "https://192.0.2.10:4433/"},
    {"kind": "dns-prefetch", "target": "//canary-prefetch.egress.test"},
    {"kind": "preconnect", "target": "https://canary-preconnect.egress.test"},
]
CANARY_JS = """
const spec = JSON.parse(document.getElementById('canaries').textContent);
for (const c of spec.canaries) {
  try {
    if (c.kind === 'fetch') fetch(c.target, {mode: 'no-cors'}).catch(() => {});
    if (c.kind === 'stun') {
      const pc = new RTCPeerConnection({iceServers: [{urls: c.target}]});
      pc.createDataChannel('canary');
      pc.createOffer().then(o => pc.setLocalDescription(o)).catch(() => {});
    }
    if (c.kind === 'webtransport' && typeof WebTransport === 'function') {
      const w = new WebTransport(c.target);
      w.ready.catch(() => {}); w.closed.catch(() => {});
    }
  } catch (e) {}
}
document.getElementById('s').textContent = 'canaries sent';
"""
WORDS = ["harbour", "lantern", "meridian", "tidewater", "compass", "parallax", "orchard", "basalt", "cirrus", "quarry"]


def pages():
    gif = bytes.fromhex("47494638396101000100800000000000ffffff21f90401000000002c00000000010001000002024401003b")
    css = b"body{font:18px/1.5 sans-serif;margin:3em;max-width:40em}h1{font-size:28px}"
    r = {"/style.css": ("text/css", css), "/dot.gif": ("image/gif", gif)}
    for i in range(1, TABS + 1):
        w = WORDS[i - 1]
        r[f"/tab/{i}"] = ("text/html", (
            f"<!doctype html><meta charset=utf-8><title>Local tab {i}: {w}</title>"
            f"<link rel=stylesheet href=/style.css><h1>Local tab {i}</h1>"
            f"<p>The {w} page is served from loopback. <a href=/tab/{i % TABS + 1}>Next tab</a></p>"
            f"<img src=/dot.gif width=40 height=40 alt=''>").encode())
    links = "".join(f"<link rel={c['kind']} href='{c['target']}'>" for c in CANARIES
                    if c["kind"] in ("dns-prefetch", "preconnect"))
    r["/canary.html"] = ("text/html", (
        "<!doctype html><meta charset=utf-8><title>Egress canaries</title>" + links +
        "<script type=application/json id=canaries>" + json.dumps({"canaries": CANARIES}) + "</script>"
        "<h1>Egress canaries</h1><p id=s>sending</p><script>" + CANARY_JS + "</script>").encode())
    return r


def canary_hosts():
    import urllib.parse
    out = {}
    for c in CANARIES:
        t = c["target"]
        h = t.split(":")[1] if t.startswith("stun:") else urllib.parse.urlsplit(t if "//" in t else "//" + t).hostname
        out[h] = c["kind"]
    return out


def classify(events, queries, hits, name_of, canaries_on, must_not_leak=False):
    """Turn raw observations into the report: counts, violations by host, canary sightings."""
    chosts = canary_hosts()

    def is_canary(host):
        return host is not None and (host in chosts or host.endswith(L.CANARY_SUFFIX) or host in L.CANARY_IPS)

    v, seen = {}, {}

    def add(host, kind, t, detail):
        if is_canary(host) and not must_not_leak:
            seen.setdefault(host, set()).add(kind)
            return
        e = v.setdefault(host or "?", {"host": host or "?", "kinds": set(), "count": 0, "first_t": t, "details": []})
        e["kinds"].add(kind)
        e["count"] += 1
        if detail and detail not in e["details"] and len(e["details"]) < 5:
            e["details"].append(detail)

    n = {"total": len(events), "loopback": 0, "non_loopback": 0, "dns_packets": 0, "kernel_link_chatter": 0,
         "non_loopback_canary": 0}
    for p in events:
        if L.is_kernel_chatter(p):
            n["kernel_link_chatter"] += 1
            continue
        lo = L.is_loopback(p["dst"]) and p["if"] != L.DUMMY
        if not lo:
            n["non_loopback"] += 1
            if p["dst"] in L.CANARY_IPS:
                n["non_loopback_canary"] += 1
            detail = f"{p['proto']} {p['dst']}:{p['dport']} via {p['if']}"
            host = p["dst"]
            if p["dport"] == 53 and p["proto"] in ("udp", "tcp"):
                q, _ = L.dns_qname(p["payload"][2:] if p["proto"] == "tcp" else p["payload"])
                host, detail = q or host, f"DNS query to {p['dst']} " + detail
            add(host, "packet", p["t"], detail)
            continue
        n["loopback"] += 1
        if p["dst"] == L.NAMESERVER and p["dport"] == 53:
            n["dns_packets"] += 1
        elif L.in_sink(p["dst"]):
            name = name_of.get(p["dst"])
            if p["proto"] == "tcp" and p["flags"] is not None and p["flags"] & 0x02 and not p["flags"] & 0x10:
                add(name, "connect", p["t"], f"tcp {p['dport']}")
            elif p["proto"] == "udp":
                add(name, "udp", p["t"], f"udp {p['dport']}")
    for t, name, qtype, transport in queries:
        add(name, "dns", t, f"DNS type {qtype} over {transport}")
    for t, addr, port, name, what in hits:
        add(name, "request", t, f"port {port}: {what}")
    viol = sorted(({**e, "kinds": sorted(e["kinds"])} for e in v.values()), key=lambda e: e["first_t"])
    canary = {h: sorted(k) for h, k in seen.items()}
    controls = []
    if canaries_on and not must_not_leak:
        for c in CANARIES:
            if c.get("control"):
                h = [x for x in chosts if x in c["target"]][0]
                ok = h in seen
                controls.append({"canary": c["target"], "seen": ok, "kinds": sorted(seen.get(h, []))})
    dns_total = len(queries)
    dns_bad = sum(1 for q in queries if not is_canary(q[1]) or must_not_leak)
    return {"packets": n, "dns_queries": dns_total, "dns_queries_not_canary": dns_bad,
            "violations": viol, "canaries_seen": canary, "positive_controls": controls,
            "prediction": {"dns_prefetch_seen": "canary-prefetch.egress.test" in seen,
                           "preconnect_seen": "canary-preconnect.egress.test" in seen}}


def state_facts(profile):
    """Privacy-relevant scalars of Local State and Preferences (R06), and the failures among them."""
    want = ("metrics", "crash", "report", "services", "update", "safebrowsing", "variations", "suggest", "prediction",
            "do_not_track", "privacy_control")
    out, bad = {}, []
    for fname in ("Local State", os.path.join("Default", "Preferences")):
        try:
            with open(os.path.join(profile, fname)) as f:
                d = json.load(f)
        except (OSError, ValueError):
            continue

        def walk(x, path):
            if isinstance(x, dict):
                for k, val in x.items():
                    walk(val, f"{path}.{k}" if path else k)
            elif isinstance(x, (bool, int, str)) and any(w in path.lower() for w in want) and len(str(x)) < 120:
                out[f"{fname}:{path}"] = x
        walk(d, "")
    if out.get("Local State:user_experience_metrics.reporting_enabled") is True:
        bad.append("Local State says usage statistics are on (user_experience_metrics.reporting_enabled)")
    return out, bad


def inner(a):
    iso = L.Isolation().setup()
    sink = L.DnsSink(answer=not a.nxdomain).start()
    srv = L.Server(pages()).start()
    cap = L.Capture().start()
    profile = a.profile_dir or iso.mkdir("profile")
    urls = [srv.url(f"/tab/{i}") for i in range(1, TABS + (0 if a.canaries else 1))]
    if a.canaries:
        urls.append(srv.url("/canary.html"))
    env = L.person_env(iso)
    b = L.Browser(L.browser_argv(a.browser), profile, prefix=iso.prefix(), extra=shlex.split(a.browser_args),
                  urls=urls, logfile=os.path.join(iso.tmp, "browser.log"), env=env)
    t0 = time.time()
    timeline = []

    def mark(what):
        timeline.append({"t": round(time.time() - t0, 1), "what": what})
        L.log(f"egress: {timeline[-1]['t']:>7.1f}s {what}")

    b.start()
    mark(f"browser started: {' '.join(b.cmd[:6])}{' ...' if len(b.cmd) > 6 else ''}")
    got = L.wait_for(lambda: all(any(r["path"] == u.split(str(srv.port), 1)[1] for r in srv.taken()) for u in urls),
                     a.load_timeout)
    mark(f"{sum(1 for u in urls if any(r['path'] == u.split(str(srv.port), 1)[1] for r in srv.taken()))}/{len(urls)} tabs loaded")
    if a.omnibox_cmd:
        r = subprocess.run(iso.prefix() + shlex.split(a.omnibox_cmd), env=env)
        mark(f"omnibox typing command exited {r.returncode}")
    if a.omnibox_handshake:
        open(a.omnibox_handshake + ".ready", "w").close()
        ok = L.wait_for(lambda: os.path.exists(a.omnibox_handshake + ".done"), 600, 1)
        mark("omnibox typed by the driver" if ok else "omnibox driver did not answer in 600 s")
    end = time.time() + a.idle
    while time.time() < end and b.alive():
        time.sleep(min(5, max(0.1, end - time.time())))
    mark(f"idle {a.idle} s done (browser {'alive' if b.alive() else 'EXITED EARLY'})")
    alive = b.alive()
    rc = b.stop()
    time.sleep(2)
    events = cap.stop()
    sink.stop()
    srv.stop()
    mark(f"browser stopped (exit {rc})")
    rep = classify(events, sink.queries, sink.hits, dict(sink.name_of), a.canaries, a.canaries_must_not_leak)
    facts, bad = state_facts(profile)
    rep.update({"tool": "egress", "browser": a.browser, "idle_s": a.idle, "tabs": len(urls), "tabs_loaded": got,
                "browser_alive_at_end": alive, "isolation": iso.facts, "timeline": timeline, "state": facts,
                "state_failures": bad})
    blind = [c for c in rep["positive_controls"] if not c["seen"]]
    reasons = []
    if rep["violations"]:
        reasons.append(f"{len(rep['violations'])} host(s) contacted: " + ", ".join(v["host"] for v in rep["violations"]))
    if blind:
        reasons.append("blind: the canaries the page sent were not seen: " + ", ".join(c["canary"] for c in blind))
    if not got:
        reasons.append("not every local tab loaded")
    if not alive:
        reasons.append("the browser exited before the end of the scenario")
    reasons += bad
    rep["verdict"] = "FAIL" if reasons else "PASS"
    rep["reasons"] = reasons
    if a.keep_log:
        try:
            with open(os.path.join(iso.tmp, "browser.log"), "rb") as f:
                rep["browser_log_tail"] = f.read()[-4000:].decode("utf-8", "replace")
        except OSError:
            pass
    L.write_json(a.out and os.path.join(a.out, "egress.json"), rep)
    print(summary(rep))
    L.cleanup(iso, a.keep or bool(a.profile_dir))
    return 0 if rep["verdict"] == "PASS" else 1


def summary(rep):
    p = rep["packets"]
    lines = [f"EGRESS {rep['verdict']}: {p['total']} packets ({p['loopback']} loopback, {p['non_loopback']} not loopback, "
             f"{p['non_loopback_canary']} of them to the canary page's addresses), "
             f"{rep['dns_queries']} DNS queries ({rep['dns_queries_not_canary']} not from the canary page), "
             f"{rep['tabs']} tabs, idle {rep['idle_s']} s"]
    for v in rep["violations"]:
        lines.append(f"  VIOLATION {v['host']}: {','.join(v['kinds'])} x{v['count']} from {v['first_t']} s; "
                     + "; ".join(v["details"]))
    for c in rep["positive_controls"]:
        lines.append(f"  control {c['canary']}: {'seen' if c['seen'] else 'NOT SEEN'} {c['kinds']}")
    if rep["canaries_seen"]:
        lines.append("  canaries seen: " + ", ".join(f"{h} {k}" for h, k in sorted(rep["canaries_seen"].items())))
    lines.append(f"  prediction: dns-prefetch {'seen' if rep['prediction']['dns_prefetch_seen'] else 'not seen'}, "
                 f"preconnect {'seen' if rep['prediction']['preconnect_seen'] else 'not seen'}")
    for r in rep["reasons"]:
        lines.append(f"  reason: {r}")
    return "\n".join(lines)


# ---------------------------------------------------------------------------------------------

def selftest(require_netns):
    fails = []

    def check(cond, what):
        print(f"  {'ok  ' if cond else 'FAIL'} {what}")
        if not cond:
            fails.append(what)

    print("egress selftest: parsers and classifier")
    q = L.dns_query("update.example.org", 28)
    check(L.dns_qname(q) == ("update.example.org", 28), "DNS question parses (name, AAAA)")
    f = L.eth(L.ipv4_udp("10.254.0.1", "8.8.8.8", 5353, 53, q))
    p = L.parse_frame(f)
    check(p and p["dst"] == "8.8.8.8" and p["dport"] == 53 and p["proto"] == "udp", "IPv4/UDP frame parses")
    p["if"], p["t"] = L.DUMMY, 1.0
    syn = L.parse_frame(L.eth(L.ipv4_udp("127.0.0.1", "127.1.0.1", 40000, 443, b"", proto=6)))
    syn["if"], syn["t"] = "lo", 2.0
    dnsp = L.parse_frame(L.eth(L.ipv4_udp("127.0.0.1", "127.0.0.1", 40001, 53, q)))
    dnsp["if"], dnsp["t"] = "lo", 0.5
    web = L.parse_frame(L.eth(L.ipv4_udp("127.0.0.1", "127.0.0.1", 40002, 8080, b"GET / HTTP/1.1", proto=6)))
    web["if"], web["t"] = "lo", 0.4
    ctl = L.parse_frame(L.eth(L.ipv4_udp("10.254.0.1", "192.0.2.11", 40003, 80, b"", proto=6)))
    ctl["if"], ctl["t"] = L.DUMMY, 0.6
    r = classify([p, syn, dnsp, web, ctl], [(0.5, "svc.example.net", 1, "udp"), (0.7, "canary-fetch.egress.test", 1, "udp")],
                 [(2.1, "127.1.0.1", 443, "svc.example.net", "tls server name svc.example.net")],
                 {"127.1.0.1": "svc.example.net"}, True)
    hosts = {v["host"] for v in r["violations"]}
    check(hosts == {"update.example.org", "svc.example.net"}, f"violations name the hosts ({sorted(hosts)})")
    check(r["packets"]["non_loopback"] == 2 and r["dns_queries_not_canary"] == 1, "counts: 2 not loopback, 1 DNS query not canary")
    check(all(c["seen"] for c in r["positive_controls"]), "both positive controls seen")
    r2 = classify([web], [], [], {}, True)
    check(not r2["violations"] and not all(c["seen"] for c in r2["positive_controls"]), "a run without canaries is blind")
    hello = (b"\x16\x03\x01\x00\x00\x01" + b"\0" * 37 + b"\0" + b"\0\x02\x13\x01" + b"\x01\0")
    name = b"a.example"
    import struct
    ext = struct.pack("!HHHBH", 0, len(name) + 5, len(name) + 3, 0, len(name)) + name
    check(L.tls_sni(hello + struct.pack("!H", len(ext)) + ext) == "a.example", "TLS server name parses")

    ok, how = L.Isolation.available()
    if not ok:
        print(f"egress selftest: namespace part SKIPPED: {how}")
        if require_netns:
            fails.append("network namespace unavailable")
    else:
        print(f"egress selftest: stub browser in a network namespace ({how})")
        me = os.path.abspath(__file__)
        for mode, want_rc, want_hosts in (("clean", 0, set()), ("leaky", 1, {"update.stub-leak.example", "198.51.100.7"})):
            out = tempfile.mkdtemp(prefix="egress-selftest-")
            os.chmod(out, 0o777)
            rc = subprocess.call([sys.executable, "-I", me, "run", "--browser", f"stub:{mode}", "--idle", "2",
                                  "--load-timeout", "20", "--out", out], stdout=subprocess.DEVNULL)
            try:
                with open(os.path.join(out, "egress.json")) as fh:
                    rep = json.load(fh)
            except (OSError, ValueError):
                rep = {"violations": [], "positive_controls": []}
            hosts = {v["host"] for v in rep["violations"]}
            check(rc == want_rc, f"stub {mode}: exit {rc} (want {want_rc})")
            check(hosts == want_hosts, f"stub {mode}: violations {sorted(hosts)} (want {sorted(want_hosts)})")
            check(all(c["seen"] for c in rep["positive_controls"]) and rep["positive_controls"],
                  f"stub {mode}: canaries seen (harness not blind)")
            if mode == "leaky":
                details = " ".join(d for v in rep["violations"] for d in v["details"])
                check("tls server name update.stub-leak.example" in details, "stub leaky: the TLS server name is recorded")
    print(f"egress selftest: {'PASS' if not fails else 'FAIL'} ({len(fails)} failed)")
    return 0 if not fails else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--require-netns", action="store_true", help="selftest fails when no namespace can be made")
    sub = ap.add_subparsers(dest="cmd")
    r = sub.add_parser("run")
    r.add_argument("--browser", required=True, help="browser command ('stub', 'stub:leaky' for the stub)")
    r.add_argument("--browser-args", default="--no-first-run --password-store=basic",
                   help="extra flags (the product's own flags come from the launcher in --browser)")
    r.add_argument("--idle", type=int, default=1800, help="idle seconds after typing (R06: 1800)")
    r.add_argument("--load-timeout", type=int, default=90)
    r.add_argument("--no-canaries", dest="canaries", action="store_false")
    r.add_argument("--canaries-must-not-leak", action="store_true", help="ADR 0053 mode: canary traffic is a leak too")
    r.add_argument("--nxdomain", action="store_true", help="the sink refuses names instead of answering")
    r.add_argument("--omnibox-cmd", help="command run as the person after the tabs load (types into the address bar)")
    r.add_argument("--omnibox-handshake", help="PATH: create PATH.ready, wait for PATH.done (a driver outside types)")
    r.add_argument("--profile-dir", help="use this profile (an existing one) instead of a fresh one")
    r.add_argument("--user", help="uid or name the browser runs as (default: the sudo caller)")
    r.add_argument("--out", help="directory for egress.json")
    r.add_argument("--keep", action="store_true", help="keep the temporary profile and logs")
    r.add_argument("--keep-log", action="store_true", help="put the tail of the browser's output in the report")
    a = ap.parse_args()
    if a.selftest:
        return selftest(a.require_netns)
    if a.cmd != "run":
        ap.print_help()
        return 2
    if L.Isolation.inside():
        return inner(a)
    if a.out:
        os.makedirs(a.out, exist_ok=True)
        a.out = os.path.abspath(a.out)
    return L.Isolation(a.user).enter(__file__, L.fix_out(sys.argv[1:], a.out))


if __name__ == "__main__":
    sys.exit(main())
