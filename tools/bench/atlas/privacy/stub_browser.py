# SPDX-License-Identifier: GPL-2.0-or-later
"""stub_browser.py: a stand-in browser for the harness selftests (README.md). It takes the flags a
real browser takes (--user-data-dir=, --host-resolver-rules=, URLs) and acts out what the test pages
ask through their JSON blocks, so every tool can be proven without a browser build:

  --stub-mode=clean     loads the pages and their subresources, nothing else
  --stub-mode=leaky     also calls home: a DNS query for update.stub-leak.example, a TLS hello to
                        the answer, and a UDP packet to 198.51.100.7:443 (TEST-NET-2)
  --stub-dnt=0|1, --stub-gpc=0|1   header and navigator values it reports (default 1)
  --stub-rewrite        changes the page text it reports (page_asis must catch it)
  --stub-blocker=on|off a fake blocker: requests to hosts in STUB_BLOCKED are not made when on
                        (the default); --disable-extensions or a profile that disables the blocker extension turns it off

Runs until SIGTERM, then writes "Local State" in the profile like a browser does. Stdlib only.
"""
import html.parser
import http.client
import json
import os
import signal
import socket
import struct
import sys
import time
import urllib.parse

STUB_BLOCKED = {"static.criteo.net", "googleads.g.doubleclick.net", "match.adsrvr.org", "static.hotjar.com",
                "pixel.quantserve.com", "sb.scorecardresearch.com"}
args = sys.argv[1:]
opt = {a.split("=", 1)[0]: (a.split("=", 1)[1] if "=" in a else "1") for a in args if a.startswith("--")}
urls = [a for a in args if not a.startswith("--")]
MODE = opt.get("--stub-mode", "clean")
DNT, GPC = opt.get("--stub-dnt", "1"), opt.get("--stub-gpc", "1")
BLOCKER = opt.get("--stub-blocker", "on") == "on" and "--disable-extensions" not in opt \
    and "--disable-component-extensions-with-background-pages" not in opt
profile = opt.get("--user-data-dir")


def blocker_disabled_in_profile():
    """The profile disables the blocker extension (the stand-in for the first-run switch)."""
    try:
        with open(os.path.join(profile, "Default", "Preferences")) as f:
            d = json.load(f)
        return d["extensions"]["settings"]["blockjmkbacgjkknlgpkjjiijinjdanf"]["state"] == 0
    except (OSError, ValueError, KeyError, TypeError):
        return False


if BLOCKER and profile and blocker_disabled_in_profile():
    BLOCKER = False
MAP = None
rules = opt.get("--host-resolver-rules", "")
if rules.startswith("MAP * "):
    MAP = rules.split()[2].rstrip(",")
stop = False


def on_term(*_):
    global stop
    stop = True


signal.signal(signal.SIGTERM, on_term)


def headers():
    h = {"User-Agent": "stub-browser/1"}
    if DNT == "1":
        h["DNT"] = "1"
    if GPC == "1":
        h["Sec-GPC"] = "1"
    return h


def request(url, method="GET", body=None):
    u = urllib.parse.urlsplit(url)
    if u.scheme not in ("http",):
        return None
    host = u.hostname
    if BLOCKER and host in STUB_BLOCKED:
        return None
    target = host
    if MAP is None and host not in ("127.0.0.1", "localhost"):
        target = socket.getaddrinfo(host, u.port or 80, socket.AF_INET)[0][4][0]
    elif MAP is not None:
        target = MAP
    try:
        c = http.client.HTTPConnection(target, u.port or 80, timeout=5)
        h = headers()
        h["Host"] = u.netloc
        if body is not None:
            h["Content-Type"] = "application/json"
        c.request(method, u.path + (("?" + u.query) if u.query else ""), body=body, headers=h)
        r = c.getresponse()
        data = r.read()
        c.close()
        return data
    except OSError:
        return None


class Page(html.parser.HTMLParser):
    def __init__(self):
        super().__init__()
        self.subs, self.blocks, self.cur = [], {}, None

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag in ("img", "script", "iframe") and a.get("src"):
            self.subs.append(a["src"])
        if tag == "link" and a.get("href") and a.get("rel") == "stylesheet":
            self.subs.append(a["href"])
        if tag == "script" and a.get("type") == "application/json":
            self.cur = a.get("id")

    def handle_data(self, data):
        if self.cur:
            self.blocks[self.cur] = self.blocks.get(self.cur, "") + data

    def handle_endtag(self, tag):
        if tag == "script":
            self.cur = None


def resolve(name):
    try:
        return socket.getaddrinfo(name, 443, socket.AF_INET)[0][4][0]
    except OSError:
        return None


def canaries(spec):
    for c in spec.get("canaries", []):
        kind, target = c["kind"], c["target"]
        host = urllib.parse.urlsplit(target).hostname if "://" in target else target.split(":")[1] if target.startswith("stun:") else target
        if kind in ("dns-prefetch", "preconnect"):
            continue  # the stub does no speculative work, like a browser with prediction off
        addr = host
        if not host.replace(".", "").isdigit():
            addr = resolve(host)
        if not addr:
            continue
        if kind in ("fetch",):
            s = socket.socket()
            s.settimeout(1)
            try:
                su = urllib.parse.urlsplit(target)
                s.connect((addr, su.port or (80 if su.scheme == "http" else 443)))
            except OSError:
                pass
            s.close()
        else:
            u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            try:
                u.sendto(b"\0\1\0\0stub", (addr, 3478 if kind == "stun" else 4433))
            except OSError:
                pass
            u.close()


def load(url):
    data = request(url)
    if data is None:
        return
    p = Page()
    p.feed(data.decode("utf-8", "replace"))
    for s in p.subs:
        request(urllib.parse.urljoin(url, s))
    if "canaries" in p.blocks:
        canaries(json.loads(p.blocks["canaries"]))
    if "probe" in p.blocks:
        spec = json.loads(p.blocks["probe"])
        for f in spec.get("fetch", []):
            request(urllib.parse.urljoin(url, f))
        if spec.get("worker"):
            request(urllib.parse.urljoin(url, spec["worker"]))
            for f in spec.get("worker_fetch", []):
                request(urllib.parse.urljoin(url, f))
        rep = {"doc_gpc": GPC == "1", "worker_gpc": GPC == "1", "doc_dnt": "1" if DNT == "1" else None,
               "worker_dnt": "1" if DNT == "1" else None, "agent": "stub"}
        request(urllib.parse.urljoin(url, spec["report"]), "POST", json.dumps(rep).encode())
    if "asis" in p.blocks:
        spec = json.loads(p.blocks["asis"])
        raw = data.decode("utf-8", "replace")
        text = Text()
        text.feed(raw)
        t = " ".join(" ".join(text.text).split())
        if "--stub-rewrite" in opt:
            t = t.replace("harbour", "harbor")
        rep = {"text": t, "tags": text.tags[:-2], "layout": "stub-layout", "canvas_diff": 0, "agent": "stub"}
        request(urllib.parse.urljoin(url, spec["report"]), "POST", json.dumps(rep).encode())


class Text(html.parser.HTMLParser):
    SKIP = {"script", "style", "template", "noscript", "title"}

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.text, self.tags, self.skip = [], [], 0

    def handle_starttag(self, tag, attrs):
        self.tags.append(tag)
        if tag in self.SKIP:
            self.skip += 1

    def handle_endtag(self, tag):
        if tag in self.SKIP and self.skip:
            self.skip -= 1

    def handle_data(self, d):
        if not self.skip:
            self.text.append(d)


def leak():
    try:
        a = resolve("update.stub-leak.example")
        if a:
            s = socket.create_connection((a, 443), timeout=2)
            hello = (b"\x16\x03\x01\x00\x00\x01" + b"\0" * 37 + b"\0" + b"\0\x02\x13\x01" + b"\x01\0")
            name = b"update.stub-leak.example"
            ext = struct.pack("!HHHBH", 0, len(name) + 5, len(name) + 3, 0, len(name)) + name
            s.sendall(hello + struct.pack("!H", len(ext)) + ext)
            s.close()
    except OSError:
        pass
    u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        u.sendto(b"stub-call-home", ("198.51.100.7", 443))
    except OSError:
        pass
    u.close()


def fake_pages():
    good = MODE != "leaky"
    prefs = {"search": {"suggest_enabled": not good}, "net": {"network_prediction_options": 2 if good else 0},
             "enable_do_not_track": DNT == "1", "stub": {"global_privacy_control": GPC == "1",
                                                          "services": {"enabled": not good, "bangs": False,
                                                                       "ext_proxy": False, "spellcheck_files": False,
                                                                       "browser_updates": False, "ublock_assets": False},
                                                          "noise": {"enabled": True}}}
    local = {"user_experience_metrics": {"reporting_enabled": not good}, "stub": {"crash_reporting": {"mode": -1}}}

    def wrap(d):  # the shape of the real internals pages: every pref is {"value": ..., "metadata": [...]}
        return {k: wrap(v) if isinstance(v, dict) else {"value": v, "metadata": ["default", "user_modifiable"]}
                for k, v in d.items()}
    return {"prefs-internals": wrap(prefs), "local-state": wrap(local)}


def cdp():
    """--remote-debugging-pipe: enough of the protocol for the harness (createTarget, evaluate)."""
    import threading
    pages, targets, buf = fake_pages(), {}, b""

    def reply(m, result):
        os.write(4, json.dumps({"id": m["id"], "result": result}).encode() + b"\0")

    def run():
        nonlocal buf
        while True:
            try:
                chunk = os.read(3, 65536)
            except OSError:
                return
            if not chunk:
                return
            buf += chunk
            while b"\0" in buf:
                raw, buf = buf.split(b"\0", 1)
                m = json.loads(raw)
                meth, par = m.get("method"), m.get("params", {})
                if meth == "Target.createTarget":
                    t = f"t{len(targets) + 1}"
                    targets[t] = par.get("url", "")
                    reply(m, {"targetId": t})
                    if targets[t].startswith("http"):
                        threading.Thread(target=load, args=(targets[t],), daemon=True).start()
                elif meth == "Target.attachToTarget":
                    reply(m, {"sessionId": "s-" + par["targetId"]})
                elif meth == "Runtime.evaluate":
                    url = targets.get(m.get("sessionId", "")[2:], "")
                    name = url.split("://", 1)[-1]
                    val = json.dumps(pages[name]) if name in pages else ("complete" if url.startswith("http") else "")
                    reply(m, {"result": {"type": "string", "value": val}})
                else:
                    reply(m, {})
    threading.Thread(target=run, daemon=True).start()


if "--remote-debugging-pipe" in opt:
    cdp()
if profile:
    os.makedirs(profile, exist_ok=True)
for url in urls:
    load(url)
if MODE == "leaky":
    leak()
while not stop:
    time.sleep(0.2)
if profile:
    with open(os.path.join(profile, "Local State"), "w") as f:
        json.dump({"user_experience_metrics": {"reporting_enabled": MODE == "leaky"}}, f)
