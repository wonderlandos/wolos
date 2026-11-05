# SPDX-License-Identifier: GPL-2.0-or-later
"""Shared code of the browser privacy harness (tools/bench/atlas/privacy/README.md; plan AT-7/AT-16,
ADR 0053 leak canaries). Python 3 standard library only; the tools run with `python3 -I`.

What it gives the tools:
- `Isolation`: re-executes the tool in a new network and mount namespace (`unshare`, as root or in a
  user namespace) and fails closed (exit 2) when the namespace shows any interface but loopback.
  Inside it adds one dummy interface with a default route, so a connection to any outside address
  leaves a packet we can see instead of failing silently, points DNS at our sink (resolv.conf and
  nsswitch.conf bind-mounted, the resolver's unix sockets masked) and drops to the person's uid
  to start the browser.
- `Capture`: an AF_PACKET reader of every outgoing frame on every interface of the namespace.
- `DnsSink`: answers every A query with its own loopback address (127.1.x.y, one per name) and
  logs the name; the address gets listeners on ports 80 and 443 that record the TLS server name or
  the HTTP Host and path, so a violation names the host and what was asked.
- `Server`: the local web server of the test pages; every request is recorded with its headers.
- `Browser`: starts the browser command (or the stub) and stops it with SIGTERM, then SIGKILL.
Exit codes of the tools: 0 pass, 1 a check failed, 2 the harness could not run (isolation missing).
"""
import html.parser
import http.server
import ipaddress
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

INNER_ENV = "ATLAS_PRIV_INNER"
DUMMY = "atlas0"
DUMMY_V4 = "10.254.0.1/24"
DUMMY_V6 = "fd00:a71a::1/64"
SINK_NET = ipaddress.ip_network("127.1.0.0/16")
NAMESERVER = "127.0.0.1"
CANARY_SUFFIX = ".egress.test"
# Canary addresses are documentation ranges (RFC 5737): never routed anywhere.
CANARY_IPS = {"192.0.2.10", "192.0.2.11", "192.0.2.12"}


def log(*a):
    print(*a, file=sys.stderr, flush=True)


# ---------------------------------------------------------------------------------------------
# Packet parsing (also used by the selftests on synthetic frames)

def dns_qname(payload, offset=12):
    """Name and type of the first question of a DNS message, or (None, None) on a bad message."""
    try:
        labels, i = [], offset
        while True:
            n = payload[i]
            if n == 0:
                i += 1
                break
            if n & 0xC0:
                return None, None
            labels.append(payload[i + 1:i + 1 + n].decode("ascii", "replace"))
            i += 1 + n
            if len(labels) > 64:
                return None, None
        qtype = struct.unpack("!H", payload[i:i + 2])[0]
        return ".".join(labels).lower(), qtype
    except (IndexError, struct.error):
        return None, None


def dns_query(name, qtype=1, qid=0x1234):
    """A DNS query message for NAME (selftests and the stub browser)."""
    q = b"".join(bytes([len(p)]) + p.encode() for p in name.split(".") if p) + b"\0"
    return struct.pack("!HHHHHH", qid, 0x0100, 1, 0, 0, 0) + q + struct.pack("!HH", qtype, 1)


def tls_sni(data):
    """Server name of a TLS ClientHello, or None."""
    try:
        if len(data) < 43 or data[0] != 0x16 or data[5] != 0x01:
            return None
        i = 43
        i += 1 + data[i]                                   # session id
        i += 2 + struct.unpack("!H", data[i:i + 2])[0]     # cipher suites
        i += 1 + data[i]                                   # compression
        end = i + 2 + struct.unpack("!H", data[i:i + 2])[0]
        i += 2
        while i + 4 <= end:
            et, el = struct.unpack("!HH", data[i:i + 4])
            if et == 0:
                n = struct.unpack("!H", data[i + 7:i + 9])[0]
                return data[i + 9:i + 9 + n].decode("ascii", "replace").lower()
            i += 4 + el
    except (IndexError, struct.error):
        return None
    return None


def parse_frame(frame):
    """Ethernet frame -> dict(ver, src, dst, proto, sport, dport, payload) or None (not IP)."""
    if len(frame) < 14:
        return None
    etype = struct.unpack("!H", frame[12:14])[0]
    ip = frame[14:]
    try:
        if etype == 0x0800:
            ihl = (ip[0] & 0x0F) * 4
            proto = ip[9]
            src, dst = socket.inet_ntop(socket.AF_INET, ip[12:16]), socket.inet_ntop(socket.AF_INET, ip[16:20])
            l4 = ip[ihl:]
            ver = 4
        elif etype == 0x86DD:
            proto = ip[6]
            src, dst = socket.inet_ntop(socket.AF_INET6, ip[8:24]), socket.inet_ntop(socket.AF_INET6, ip[24:40])
            l4 = ip[40:]
            for _ in range(8):                                 # extension headers
                if proto in (0, 43, 60):
                    proto, l4 = l4[0], l4[(l4[1] + 1) * 8:]
                elif proto == 44:
                    proto, l4 = l4[0], l4[8:]
                else:
                    break
            ver = 6
        else:
            return None
    except (IndexError, ValueError, OSError):
        return None
    sport = dport = None
    payload = b""
    flags = None
    icmp_type = l4[0] if proto in (1, 58) and l4 else None
    if proto in (6, 17) and len(l4) >= 8:
        sport, dport = struct.unpack("!HH", l4[:4])
        if proto == 6 and len(l4) >= 20:
            off = (l4[12] >> 4) * 4
            flags = l4[13]
            payload = l4[off:]
        elif proto == 17:
            payload = l4[8:]
    return {"ver": ver, "src": src, "dst": dst, "proto": {6: "tcp", 17: "udp", 1: "icmp", 58: "icmp6"}.get(proto, str(proto)),
            "sport": sport, "dport": dport, "flags": flags, "payload": payload, "icmp_type": icmp_type}


# Neighbour discovery and multicast listener reports: the kernel's own link chatter, not the browser's
KERNEL_ICMP6 = {133, 134, 135, 136, 143, 130, 131, 132}


def is_kernel_chatter(p):
    return p["proto"] == "icmp6" and p.get("icmp_type") in KERNEL_ICMP6


def is_loopback(addr):
    try:
        return ipaddress.ip_address(addr).is_loopback
    except ValueError:
        return False


def in_sink(addr):
    try:
        return ipaddress.ip_address(addr) in SINK_NET
    except ValueError:
        return False


def eth(payload_ip, v6=False):
    """Wrap an IP packet in an Ethernet header (selftests)."""
    return b"\0" * 12 + struct.pack("!H", 0x86DD if v6 else 0x0800) + payload_ip


def ipv4_udp(src, dst, sport, dport, data, proto=17):
    """A minimal IPv4 + UDP (or TCP header without options) packet (selftests)."""
    if proto == 17:
        l4 = struct.pack("!HHHH", sport, dport, 8 + len(data), 0) + data
    else:
        l4 = struct.pack("!HHIIBBHHH", sport, dport, 0, 0, 5 << 4, 0x02, 65535, 0, 0) + data
    hdr = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(l4), 0, 0, 64, proto, 0,
                      socket.inet_aton(src), socket.inet_aton(dst))
    return hdr + l4


# ---------------------------------------------------------------------------------------------
# Isolation

class Isolation:
    """Network + mount namespace for the run. `enter()` re-executes the tool inside; inside,
    `setup()` builds the namespace and `prefix()` is the argv prefix that drops to the person."""

    def __init__(self, user=None):
        self.user = user
        self.tmp = None
        self.facts = {}

    @staticmethod
    def inside():
        return os.environ.get(INNER_ENV) == "1"

    @staticmethod
    def available():
        """(True, how) when this process can create the namespace, else (False, why)."""
        if not shutil.which("unshare"):
            return False, "unshare is not installed"
        if not shutil.which("ip"):
            return False, "ip (iproute2) is not installed"
        args = ["unshare", "-nm"] if os.geteuid() == 0 else ["unshare", "-rnm"]
        try:
            r = subprocess.run(args + ["ip", "link", "add", "probe0", "type", "dummy"],
                               capture_output=True, text=True, timeout=20)
        except (OSError, subprocess.TimeoutExpired) as e:
            return False, f"unshare did not run: {e}"
        if r.returncode != 0:
            why = (r.stderr or r.stdout).strip().splitlines()[-1:] or ["no output"]
            return False, ("this host does not allow network namespaces for this user "
                           f"(AppArmor user-namespace restriction?): {why[0]}; run as root or in the wolvm guest")
        return True, "root" if os.geteuid() == 0 else "user namespace"

    def enter(self, script, argv):
        """Re-exec SCRIPT ARGV inside the namespaces; returns its exit code (2 when unavailable)."""
        ok, how = self.available()
        if not ok:
            log(f"isolation: {how}")
            return 2
        env = dict(os.environ)
        env[INNER_ENV] = "1"
        uid, gid = self.target_ids()
        env["ATLAS_PRIV_UID"], env["ATLAS_PRIV_GID"] = str(uid), str(gid)
        env["ATLAS_PRIV_OUTER_EUID"] = str(os.geteuid())
        base = ["unshare", "-nm", "--propagation", "private"]
        if os.geteuid() != 0:
            base.insert(1, "-r")
        cmd = base + [sys.executable, "-I", os.path.abspath(script)] + argv
        return subprocess.call(cmd, env=env)

    def target_ids(self):
        """uid/gid the browser runs as: --user, else the sudo caller, else the current user."""
        import pwd
        u = self.user or os.environ.get("SUDO_USER")
        if u:
            p = pwd.getpwuid(int(u)) if str(u).isdigit() else pwd.getpwnam(u)
            return p.pw_uid, p.pw_gid
        return os.getuid(), os.getgid()

    def _sh(self, *args):
        r = subprocess.run(list(args), capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit(f"isolation: {' '.join(args)}: {r.stderr.strip()}")

    def setup(self):
        """Build the namespace. Fails closed (exit 2) on any interface but loopback."""
        ifaces = sorted(n for _, n in socket.if_nameindex())
        self.facts["interfaces_at_start"] = ifaces
        if ifaces != ["lo"]:
            log(f"isolation: the namespace shows {ifaces}, not only lo; refusing to run")
            sys.exit(2)
        self._sh("ip", "link", "set", "lo", "up")
        # sysfs shows the namespace it was mounted in: remount it so /sys/class/net is ours
        r = subprocess.run(["mount", "-t", "sysfs", "sysfs", "/sys"], capture_output=True, text=True)
        self.facts["sysfs_remounted"] = r.returncode == 0
        self._sh("ip", "link", "add", DUMMY, "type", "dummy")
        for key, val in (("router_solicitations", "0"), ("accept_ra", "0"), ("mldv2_unsolicited_report_interval", "0")):
            try:
                with open(f"/proc/sys/net/ipv6/conf/{DUMMY}/{key}", "w") as f:
                    f.write(val)
            except OSError:
                pass
        self._sh("ip", "link", "set", DUMMY, "up")
        self._sh("ip", "addr", "add", DUMMY_V4, "dev", DUMMY)
        self._sh("ip", "route", "add", "default", "dev", DUMMY)
        try:
            self._sh("ip", "-6", "addr", "add", DUMMY_V6, "dev", DUMMY, "nodad")
            self._sh("ip", "-6", "route", "add", "default", "dev", DUMMY)
            self.facts["ipv6"] = True
        except SystemExit as e:
            log(f"isolation: no IPv6 on the dummy ({e}); IPv6 attempts fail before a packet")
            self.facts["ipv6"] = False
        self.tmp = tempfile.mkdtemp(prefix="atlas-priv-")
        os.chmod(self.tmp, 0o755)
        masked = []
        for d in ("/run/systemd/resolve", "/run/nscd", "/run/avahi-daemon", "/var/run/nscd"):
            if os.path.isdir(d) and not os.path.islink(d):
                self._sh("mount", "-t", "tmpfs", "-o", "size=64k,mode=755", "tmpfs", d)
                masked.append(d)
        self.facts["masked_resolver_dirs"] = masked
        # after the masks: resolv.conf is often a link into /run/systemd/resolve
        self._bind_file("/etc/resolv.conf", f"nameserver {NAMESERVER}\noptions attempts:1 timeout:1\n")
        self._bind_file("/etc/nsswitch.conf", "passwd: files\ngroup: files\nshadow: files\n"
                        "hosts: files myhostname dns\nnetworks: files\nprotocols: files\nservices: files\n")
        self.facts["routes"] = subprocess.run(["ip", "route"], capture_output=True, text=True).stdout.split("\n")[:-1]
        return self

    def _bind_file(self, target, text):
        real = os.path.realpath(target)
        src = os.path.join(self.tmp, os.path.basename(target))
        with open(src, "w") as f:
            f.write(text)
        os.chmod(src, 0o644)
        if not os.path.exists(real):
            # a dangling symlink (resolv.conf managed by a stopped resolver): bind over the link's dir
            os.makedirs(os.path.dirname(real), exist_ok=True)
            open(real, "w").close()
        self._sh("mount", "--bind", src, real)
        self.facts.setdefault("bound", []).append(real)

    def prefix(self):
        """argv prefix that runs a command as the person (not root) inside the namespace."""
        uid, gid = int(os.environ.get("ATLAS_PRIV_UID", os.getuid())), int(os.environ.get("ATLAS_PRIV_GID", os.getgid()))
        if uid == 0:
            return []
        if os.environ.get("ATLAS_PRIV_OUTER_EUID") == "0":
            return ["setpriv", f"--reuid={uid}", f"--regid={gid}", "--clear-groups", "--"]
        # user namespace: we are uid 0 mapped to the person; map back to the person's uid
        return ["unshare", "-U", f"--map-user={uid}", f"--map-group={gid}", "--"]

    def owner(self):
        return int(os.environ.get("ATLAS_PRIV_UID", os.getuid())), int(os.environ.get("ATLAS_PRIV_GID", os.getgid()))

    def mkdir(self, name):
        """A directory the person owns, under the run's temp dir."""
        p = os.path.join(self.tmp or tempfile.mkdtemp(prefix="atlas-priv-"), name)
        os.makedirs(p, exist_ok=True)
        uid, gid = self.owner()
        if os.geteuid() == 0 and os.environ.get("ATLAS_PRIV_OUTER_EUID") == "0":
            os.chown(p, uid, gid)
        return p


class NoIsolation(Isolation):
    """--no-isolation: same interface, nothing isolated (header and page tests on a host without
    namespaces; the report says so)."""

    def setup(self):
        self.tmp = tempfile.mkdtemp(prefix="atlas-priv-")
        self.facts = {"isolated": False}
        return self

    def prefix(self):
        return []

    def owner(self):
        return os.getuid(), os.getgid()


def run_isolated(script, argv, isolate, user, inner):
    """Common entry of the tools: INNER(iso) inside the namespace (or with NoIsolation)."""
    if not isolate:
        return inner(NoIsolation().setup())
    if Isolation.inside():
        return inner(Isolation().setup())
    return Isolation(user).enter(script, argv)


def cleanup(iso, keep=False):
    if iso.tmp and not keep:
        shutil.rmtree(iso.tmp, ignore_errors=True)


def fix_out(argv, out):
    """Make --out absolute in ARGV (the inner run may start in another directory)."""
    argv = list(argv)
    for i, x in enumerate(argv):
        if x == "--out" and i + 1 < len(argv):
            argv[i + 1] = out
        elif x.startswith("--out="):
            argv[i] = f"--out={out}"
    return argv


def person_env(iso):
    """Environment for the browser: the session's display variables, a fresh HOME the person owns."""
    env = dict(os.environ, HOME=iso.mkdir("home"))
    for k in ("XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME"):
        env.pop(k, None)
    return env


# ---------------------------------------------------------------------------------------------
# Capture

class Capture:
    """Every outgoing frame on every interface of this namespace (AF_PACKET, needs CAP_NET_RAW)."""

    def __init__(self):
        self.events = []
        self.lock = threading.Lock()
        self.stop_flag = False
        self.sock = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0003))
        self.sock.settimeout(0.5)
        self.t0 = time.time()
        self.th = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self.th.start()
        return self

    def _run(self):
        while not self.stop_flag:
            try:
                frame, addr = self.sock.recvfrom(65535)
            except socket.timeout:
                continue
            except OSError:
                break
            ifname, pkttype = addr[0], addr[2]
            # loopback frames are seen twice (outgoing and host): keep outgoing only
            if pkttype != socket.PACKET_OUTGOING:
                continue
            p = parse_frame(frame)
            if p is None:
                continue
            p["if"] = ifname
            p["t"] = round(time.time() - self.t0, 3)
            with self.lock:
                self.events.append(p)

    def stop(self):
        self.stop_flag = True
        self.th.join(timeout=3)
        self.sock.close()
        with self.lock:
            return list(self.events)


# ---------------------------------------------------------------------------------------------
# DNS sink + named listeners

class DnsSink:
    """UDP and TCP DNS on NAMESERVER:53. A queries get 127.1.x.y (one per name) with listeners on
    80 and 443 there; other types get an empty answer. `answer=False` returns NXDOMAIN instead."""

    def __init__(self, answer=True):
        self.answer = answer
        self.queries = []          # (t, name, qtype, transport)
        self.addr_of = {}
        self.name_of = {}
        self.hits = []             # (t, addr, port, name, sni_or_request)
        self.lock = threading.Lock()
        self.t0 = time.time()
        self.next = 1
        self.socks = []

    def start(self):
        u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        u.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        u.bind((NAMESERVER, 53))
        t = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        t.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        t.bind((NAMESERVER, 53))
        t.listen(16)
        self.socks += [u, t]
        threading.Thread(target=self._udp, args=(u,), daemon=True).start()
        threading.Thread(target=self._tcp, args=(t,), daemon=True).start()
        return self

    def _addr(self, name):
        with self.lock:
            if name not in self.addr_of:
                n = self.next
                self.next += 1
                a = f"127.1.{(n >> 8) & 0xFF}.{n & 0xFF}"
                self.addr_of[name], self.name_of[a] = a, name
                for port in (80, 443):
                    self._listen(a, port)
            return self.addr_of[name]

    def _listen(self, addr, port):
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            s.bind((addr, port))
            s.listen(16)
        except OSError as e:
            log(f"dns sink: no listener on {addr}:{port}: {e}")
            return
        self.socks.append(s)
        threading.Thread(target=self._accept, args=(s, addr, port), daemon=True).start()

    def _accept(self, s, addr, port):
        while True:
            try:
                c, _ = s.accept()
            except OSError:
                return
            threading.Thread(target=self._peek, args=(c, addr, port), daemon=True).start()

    def _peek(self, c, addr, port):
        c.settimeout(3)
        data = b""
        try:
            data = c.recv(4096)
        except OSError:
            pass
        finally:
            c.close()
        what = tls_sni(data)
        if what:
            what = f"tls server name {what}"
        elif data[:16].split(b" ")[0] in (b"GET", b"POST", b"HEAD", b"PUT", b"OPTIONS", b"CONNECT"):
            first = data.split(b"\r\n", 1)[0].decode("latin-1")[:300]
            host = ""
            for line in data.split(b"\r\n")[1:]:
                if line.lower().startswith(b"host:"):
                    host = line[5:].strip().decode("latin-1")
            what = f"http {first} host {host}"
        else:
            what = f"{len(data)} bytes"
        with self.lock:
            self.hits.append((round(time.time() - self.t0, 3), addr, port, self.name_of.get(addr), what))

    def _reply(self, q):
        name, qtype = dns_qname(q)
        if name is None or len(q) < 12:
            return None, None
        qend = 12 + sum(len(p) + 1 for p in name.split(".") if p) + 1 + 4
        question = q[12:qend]
        qid = q[:2]
        if not self.answer:
            return name, qtype, qid + struct.pack("!HHHHH", 0x8183, 1, 0, 0, 0) + question
        if qtype == 1:
            a = self._addr(name)
            rr = b"\xc0\x0c" + struct.pack("!HHIH", 1, 1, 5, 4) + socket.inet_aton(a)
            return name, qtype, qid + struct.pack("!HHHHH", 0x8180, 1, 1, 0, 0) + question + rr
        return name, qtype, qid + struct.pack("!HHHHH", 0x8180, 1, 0, 0, 0) + question

    def _log(self, name, qtype, transport):
        with self.lock:
            self.queries.append((round(time.time() - self.t0, 3), name, qtype, transport))

    def _udp(self, u):
        while True:
            try:
                q, peer = u.recvfrom(4096)
            except OSError:
                return
            r = self._reply(q)
            if r[0] is None:
                continue
            self._log(r[0], r[1], "udp")
            try:
                u.sendto(r[2], peer)
            except OSError:
                pass

    def _tcp(self, t):
        while True:
            try:
                c, _ = t.accept()
            except OSError:
                return
            try:
                c.settimeout(3)
                n = struct.unpack("!H", c.recv(2))[0]
                q = c.recv(n)
                r = self._reply(q)
                if r[0] is not None:
                    self._log(r[0], r[1], "tcp")
                    c.sendall(struct.pack("!H", len(r[2])) + r[2])
            except (OSError, struct.error):
                pass
            finally:
                c.close()

    def stop(self):
        for s in self.socks:
            try:
                s.close()
            except OSError:
                pass


# ---------------------------------------------------------------------------------------------
# Local web server

class Server:
    """ThreadingHTTPServer on 127.0.0.1:PORT. ROUTES maps a path to (content type, bytes) or to a
    callable(handler) -> (status, content type, bytes). Every request is recorded."""

    def __init__(self, routes, port=0):
        self.routes = routes
        self.requests = []
        self.lock = threading.Lock()
        outer = self

        class H(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *a):
                pass

            def _do(self):
                n = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(n) if n else b""
                path = urllib.parse.urlsplit(self.path).path
                rec = {"t": time.time(), "method": self.command, "path": self.path,
                       "host": self.headers.get("Host", ""),
                       "headers": {k.lower(): v for k, v in self.headers.items()}, "body": body.decode("utf-8", "replace")}
                with outer.lock:
                    outer.requests.append(rec)
                r = outer.routes.get(path)
                if r is None:
                    for k, v in outer.routes.items():
                        if k.endswith("*") and path.startswith(k[:-1]):
                            r = v
                            break
                if callable(r):
                    status, ctype, data = r(self, rec)
                elif r is not None:
                    status, (ctype, data) = 200, r
                else:
                    status, ctype, data = 404, "text/plain", b"not found"
                self.send_response(status)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Cache-Control", "no-store")
                self.send_header("Access-Control-Allow-Origin", "*")
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(data)

            do_GET = do_POST = do_HEAD = _do

        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), H)
        self.httpd.daemon_threads = True
        self.port = self.httpd.server_address[1]

    def url(self, path):
        return f"http://127.0.0.1:{self.port}{path}"

    def start(self):
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        return self

    def stop(self):
        self.httpd.shutdown()
        self.httpd.server_close()

    def taken(self):
        with self.lock:
            return list(self.requests)


# ---------------------------------------------------------------------------------------------
# Browser

STUB = os.path.join(os.path.dirname(os.path.abspath(__file__)), "stub_browser.py")


def browser_argv(spec):
    """'stub[:mode]' -> the stub browser; anything else is split like a shell word list."""
    import shlex
    if spec.startswith("stub"):
        mode = spec.split(":", 1)[1] if ":" in spec else "clean"
        return [sys.executable, "-I", STUB, f"--stub-mode={mode}"]
    return shlex.split(spec)


class CdpPipe:
    """DevTools protocol over --remote-debugging-pipe (fds 3 and 4 of the browser): no port, no
    network. Messages are JSON separated by NUL bytes."""

    def __init__(self):
        self.to_r, self.to_w = os.pipe()
        self.from_r, self.from_w = os.pipe()
        self.next_id = 0
        self.replies = {}
        self.cv = threading.Condition()
        self.closed = False

    def wrap(self, cmd):
        # the shell only moves our two pipe ends to fds 3 and 4, then execs the browser
        return ["sh", "-c", f'exec "$@" 3<&{self.to_r} 4>&{self.from_w}', "sh"] + list(cmd)

    def fds(self):
        return (self.to_r, self.from_w)

    def started(self):
        os.close(self.to_r)
        os.close(self.from_w)
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        buf = b""
        while True:
            try:
                chunk = os.read(self.from_r, 65536)
            except OSError:
                chunk = b""
            if not chunk:
                with self.cv:
                    self.closed = True
                    self.cv.notify_all()
                return
            buf += chunk
            while b"\0" in buf:
                msg, buf = buf.split(b"\0", 1)
                try:
                    m = json.loads(msg)
                except ValueError:
                    continue
                if "id" in m:
                    with self.cv:
                        self.replies[m["id"]] = m
                        self.cv.notify_all()

    def call(self, method, params=None, session=None, timeout=30):
        """Result of METHOD, or raises RuntimeError with the browser's error."""
        with self.cv:
            self.next_id += 1
            i = self.next_id
        m = {"id": i, "method": method, "params": params or {}}
        if session:
            m["sessionId"] = session
        try:
            os.write(self.to_w, json.dumps(m).encode() + b"\0")
        except OSError as e:
            raise RuntimeError(f"{method}: pipe closed: {e}")
        with self.cv:
            self.cv.wait_for(lambda: i in self.replies or self.closed, timeout)
            r = self.replies.pop(i, None)
        if r is None:
            raise RuntimeError(f"{method}: no reply in {timeout} s")
        if "error" in r:
            raise RuntimeError(f"{method}: {r['error'].get('message')}")
        return r.get("result", {})

    def page_text(self, url, wait=3.0, expr="document.body ? document.body.innerText : ''"):
        """Open URL in a new tab and return EXPR evaluated there (a string), or raise."""
        t = self.call("Target.createTarget", {"url": url})["targetId"]
        sess = self.call("Target.attachToTarget", {"targetId": t, "flatten": True})["sessionId"]
        end = time.time() + wait + 10
        val = ""
        time.sleep(wait)
        while time.time() < end:
            r = self.call("Runtime.evaluate", {"expression": expr, "returnByValue": True, "awaitPromise": True}, sess)
            val = r.get("result", {}).get("value")
            if val:
                break
            time.sleep(0.5)
        try:
            self.call("Target.closeTarget", {"targetId": t}, timeout=5)
        except RuntimeError:
            pass
        return val


class Browser:
    """One browser process group. ARGV is the command; PROFILE the user data dir; PIPE a CdpPipe."""

    def __init__(self, argv, profile, prefix=(), extra=(), urls=(), logfile=None, env=None, pipe=None):
        cmd = list(argv) + [f"--user-data-dir={profile}"] + list(extra)
        if pipe:
            cmd = pipe.wrap(cmd + ["--remote-debugging-pipe"])
        self.cmd = list(prefix) + cmd + list(urls)
        self.pipe = pipe
        self.profile = profile
        self.logfile = logfile
        self.env = env
        self.p = None

    def start(self):
        out = open(self.logfile, "ab") if self.logfile else subprocess.DEVNULL
        env = dict(self.env or os.environ)
        env.pop(INNER_ENV, None)
        for k in ("http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"):
            env.pop(k, None)
        self.p = subprocess.Popen(self.cmd, stdout=out, stderr=out, start_new_session=True, env=env,
                                  pass_fds=self.pipe.fds() if self.pipe else ())
        if self.pipe:
            self.pipe.started()
        return self

    def alive(self):
        return self.p is not None and self.p.poll() is None

    def stop(self, grace=20):
        if self.p is None:
            return None
        try:
            os.killpg(self.p.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            self.p.wait(timeout=grace)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(self.p.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.p.wait(timeout=10)
        # helpers that left the group (zygote children are in it; be thorough anyway)
        time.sleep(0.5)
        return self.p.returncode


def wait_for(pred, timeout, step=0.25):
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(step)
    return pred()


# ---------------------------------------------------------------------------------------------
# Raw-page reading (page_asis, stub)

class TextAndTags(html.parser.HTMLParser):
    """Text outside script/style and the start-tag sequence of a raw HTML document."""

    SKIP = {"script", "style", "template", "noscript", "title"}
    VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"}

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.text, self.tags, self.attrs, self.skip = [], [], [], 0

    def handle_starttag(self, tag, attrs):
        self.tags.append(tag)
        self.attrs.append((tag, dict(attrs)))
        if tag in self.SKIP:
            self.skip += 1

    def handle_endtag(self, tag):
        if tag in self.SKIP and self.skip:
            self.skip -= 1

    def handle_data(self, data):
        if not self.skip:
            self.text.append(data)


def norm_text(s):
    return " ".join(s.split())


def write_json(path, obj):
    if path:
        os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
        with open(path, "w") as f:
            json.dump(obj, f, indent=2, sort_keys=True)
            f.write("\n")


# The blocker is a component extension (id kUBlockOriginComponentId in the engine's
# components/*_services/extension_ids.h). Until the first-run switch exists (AT-20, pref atlas.tracker_blocking)
# the "off" phase disables that extension in the profile the way the person does in the extensions
# page (state 0, disable reason 1 = user action).
UBLOCK_ID = "blockjmkbacgjkknlgpkjjiijinjdanf"
UBLOCK_OFF_PREFS = {"extensions": {"settings": {UBLOCK_ID: {"state": 0, "disable_reasons": [1]}}}}


def seed_prefs(profile, prefs):
    """Write PREFS as the profile's Preferences before the browser first starts."""
    os.makedirs(os.path.join(profile, "Default"), exist_ok=True)
    with open(os.path.join(profile, "Default", "Preferences"), "w", encoding="utf-8") as f:
        json.dump(prefs, f)
