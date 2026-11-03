# SPDX-License-Identifier: GPL-2.0-or-later
# Web-app benchmark runner (tools/bench/webapps/README.md). `python3 webapps.py run` measures every
# app of apps.json inside a running WolOS session (as root, the session's user owns the apps);
# `python3 webapps.py report RESULT.json` turns the result into the markdown tables. Stdlib only.
"""Per app, with Memory Saver on and off: cold and warm start (window mapped, CPU settled), RSS,
PSS and swap of the whole process tree after launch, after 2 minutes idle in front and after 2
minutes in the background, input-to-frame latency and frame intervals under a scripted scroll and
typing run (each `wl_surface.commit` in the app's Wayland protocol log is a frame), idle CPU and
wakeups, and the first input after coming back from the background."""
import argparse
import http.server
import json
import os
import pwd
import re
import shlex
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SAVER_STATUS = '/var/run/wol-memsaver.json'
SETTLE_CPU_MS_PER_S = 50.0  # tree CPU below 5 % of one core for a whole second: start is over
LAT_WINDOW_MS = 600.0  # an input with no changed frame within this is counted as "no change"
ANIM_GAP_MS = 100.0  # consecutive changed frames closer than this belong to one animation


def mono_ms():
    return time.monotonic() * 1000.0


def log(*a):
    print(time.strftime('%H:%M:%S'), *a, file=sys.stderr, flush=True)


# ---------------------------------------------------------------- session ---------------------


class Session:
    """The logged-in desktop session: its user, environment and compositor sockets."""

    def __init__(self, user):
        pw = pwd.getpwnam(user)
        self.user, self.uid, self.home = user, pw.pw_uid, pw.pw_dir
        self.runtime = f'/run/user/{self.uid}'
        hyp = os.path.join(self.runtime, 'hypr')
        sigs = sorted(os.listdir(hyp), key=lambda s: os.stat(os.path.join(hyp, s)).st_mtime)
        if not sigs:
            raise SystemExit('webapps: no compositor instance for ' + user)
        self.sig = sigs[-1]
        self.sock = os.path.join(hyp, self.sig, '.socket.sock')
        self.events = os.path.join(hyp, self.sig, '.socket2.sock')
        way = sorted(f for f in os.listdir(self.runtime) if re.fullmatch(r'wayland-\d+', f))
        self.env = {
            'HOME': self.home, 'USER': user, 'LOGNAME': user,
            'PATH': '/usr/local/bin:/usr/bin', 'XDG_RUNTIME_DIR': self.runtime,
            'WAYLAND_DISPLAY': way[0] if way else 'wayland-1',
            'HYPRLAND_INSTANCE_SIGNATURE': self.sig,
            'DBUS_SESSION_BUS_ADDRESS': f'unix:path={self.runtime}/bus',
            'XDG_SESSION_TYPE': 'wayland', 'XDG_CURRENT_DESKTOP': 'Hyprland',
            'DISPLAY': ':0', 'LANG': 'en_US.UTF-8',
        }
        self.comp_pid = self._pid_of('Hyprland')

    @staticmethod
    def _pid_of(comm):
        for p in os.listdir('/proc'):
            if p.isdigit():
                try:
                    if open(f'/proc/{p}/comm').read().strip() == comm:
                        return int(p)
                except OSError:
                    pass
        return 0

    def ctl(self, cmd):
        """One request on the compositor's control socket (as hyprctl sends it)."""
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(5)
        s.connect(self.sock)
        s.sendall(cmd.encode())
        out = b''
        while True:
            b = s.recv(65536)
            if not b:
                break
            out += b
        s.close()
        return out.decode(errors='replace')

    def clients(self):
        try:
            return json.loads(self.ctl('j/clients'))
        except (OSError, ValueError):
            return []

    def exec(self, argv, env=None):
        """Start a program the way the Dock does: the compositor spawns it in the session."""
        pre = ' '.join(f'{k}={shlex.quote(v)}' for k, v in (env or {}).items())
        line = (f'env {pre} ' if pre else '') + ' '.join(shlex.quote(a) for a in argv)
        return self.ctl('dispatch exec ' + line)

    def as_user(self, argv, **kw):
        return subprocess.run(['runuser', '-u', self.user, '--', 'env'] +
                              [f'{k}={v}' for k, v in self.env.items()] + argv, **kw)

    def popen_user(self, argv, **kw):
        return subprocess.Popen(['runuser', '-u', self.user, '--', 'env'] +
                                [f'{k}={v}' for k, v in self.env.items()] + argv, **kw)


class Events(threading.Thread):
    """Compositor event socket: `openwindow>>ADDR,WS,CLASS,TITLE` with the time it arrived."""

    def __init__(self, path):
        super().__init__(daemon=True)
        self.path, self.lock, self.opened = path, threading.Lock(), []
        self.start()

    def run(self):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(self.path)
        buf = b''
        while True:
            b = s.recv(65536)
            if not b:
                return
            buf += b
            while b'\n' in buf:
                line, buf = buf.split(b'\n', 1)
                ev, _, data = line.decode(errors='replace').partition('>>')
                if ev == 'openwindow':
                    addr, _, rest = data.partition(',')
                    _, _, rest = rest.partition(',')
                    cls, _, _ = rest.partition(',')
                    with self.lock:
                        self.opened.append((mono_ms(), '0x' + addr, cls))

    def since(self, t):
        with self.lock:
            return [e for e in self.opened if e[0] >= t]


# ---------------------------------------------------------------- processes -------------------


def ptable():
    t = {}
    for p in os.listdir('/proc'):
        if p.isdigit():
            try:
                st = open(f'/proc/{p}/stat').read().rsplit(')', 1)[1].split()
                t[int(p)] = int(st[1])
            except (OSError, IndexError, ValueError):
                pass
    return t


def cgroup(pid):
    try:
        return open(f'/proc/{pid}/cgroup').read().strip().split('::', 1)[-1]
    except OSError:
        return ''


def app_scope(cg):
    """The `app-*.scope` a cgroup is in (wold's Memory Saver scope), or None."""
    m = re.match(r'(/user\.slice/.*?/app-[^/]*\.scope)', cg)
    return m.group(1) if m else None


def cmdline(pid):
    try:
        return open(f'/proc/{pid}/cmdline', 'rb').read().split(b'\0')
    except OSError:
        return []


def app_pids(pid, exe, old):
    """The launched app: processes this launch started (not in `old`): the window's process with
    its new ancestors and all their descendants, the process started from the app's command
    (`exe`, with its descendants: a Kit app whose window the shell's host draws), and new
    processes in the same app scope (helpers that left the tree). Never a process that ran
    before the launch, so the shell or a host shared with other apps is never counted or
    stopped."""
    t = ptable()
    roots = []
    top = pid
    while t.get(top, 0) not in old and t.get(top, 0) in t:
        top = t[top]
    roots.append(top)
    for p in t:
        if p not in old and any(os.path.basename(a.decode(errors='replace')) == exe
                                for a in cmdline(p)[:4]):
            roots.append(p)
    out, i = [r for r in roots if r not in old or r == pid], 0
    while i < len(out):
        out.extend(c for c, pp in t.items() if pp == out[i] and c not in out)
        i += 1
    scope = app_scope(cgroup(pid))
    if scope:
        out.extend(p for p in t if p not in old and p not in out and cgroup(p).startswith(scope))
    return sorted(set(out) - (old - {pid}))


def rollup(pids):
    """Sum of smaps_rollup over pids, in KiB: Rss, Pss, Swap, SwapPss."""
    tot = {'rss': 0, 'pss': 0, 'swap': 0, 'swap_pss': 0, 'procs': 0}
    keys = {'Rss:': 'rss', 'Pss:': 'pss', 'Swap:': 'swap', 'SwapPss:': 'swap_pss'}
    for p in pids:
        try:
            for line in open(f'/proc/{p}/smaps_rollup'):
                f = line.split()
                if f and f[0] in keys:
                    tot[keys[f[0]]] += int(f[1])
            tot['procs'] += 1
        except OSError:
            pass
    return {k: (round(v / 1024, 1) if k != 'procs' else v) for k, v in tot.items()}


def roles(pids):
    """PSS in MiB and process count per role: the engine's `--type=` word on the command line,
    `gpu` for the process that carries `--gpu-preferences`, `child` for a renamed helper without
    a type, else the executable's name (the main process). Shows where an app's memory sits."""
    out = {}
    for p in pids:
        try:
            cmd = open(f'/proc/{p}/cmdline', 'rb').read().replace(b'\0', b' ').decode('utf-8', 'replace')
            m = re.search(r'--type=([\w-]+)', cmd)
            if m:
                role = m.group(1)
            elif '--gpu-preferences' in cmd:
                role = 'gpu'
            elif '--shared-files' in cmd or '--field-trial-handle' in cmd:
                role = 'child'
            else:
                role = os.path.basename(os.readlink(f'/proc/{p}/exe')) or str(p)
            pss = sum(int(x.split()[1]) for x in open(f'/proc/{p}/smaps_rollup') if x.startswith('Pss:'))
        except (OSError, ValueError, IndexError):
            continue
        n, tot = out.get(role, (0, 0))
        out[role] = (n + 1, tot + pss)
    return {k: {'n': n, 'pss': round(t / 1024, 1)} for k, (n, t) in sorted(out.items())}


def cpu_switches(pids):
    """(CPU ns, context switches) summed over every thread of pids (design/tools/perf/idle.py)."""
    cpu = sw = 0
    for p in pids:
        try:
            tasks = os.listdir(f'/proc/{p}/task')
        except OSError:
            continue
        for t in tasks:
            try:
                cpu += int(open(f'/proc/{p}/task/{t}/schedstat').read().split()[0])
                for line in open(f'/proc/{p}/task/{t}/status'):
                    if line.startswith(('voluntary_ctxt_switches', 'nonvoluntary_ctxt_switches')):
                        sw += int(line.split()[1])
            except (OSError, ValueError, IndexError):
                pass
    return cpu, sw


def idle_cost(pids_fn, secs):
    """CPU ms/s and wakeups/s of the tree over `secs` (pids re-read at the end)."""
    p0 = pids_fn()
    a = cpu_switches(p0)
    time.sleep(secs)
    b = cpu_switches([p for p in pids_fn() if p in p0])
    return {'cpu_ms_s': round((b[0] - a[0]) / 1e6 / secs, 2),
            'wakeups_s': round((b[1] - a[1]) / secs, 2)}


def faults(pids):
    """Minor and major page faults summed over pids."""
    mi = ma = 0
    for p in pids:
        try:
            st = open(f'/proc/{p}/stat').read().rsplit(')', 1)[1].split()
            mi, ma = mi + int(st[7]), ma + int(st[9])
        except (OSError, IndexError, ValueError):
            pass
    return {'minor': mi, 'major': ma}


def settle(pids_fn, t0, limit_ms):
    """Start of the first whole second, after t0, with the tree under SETTLE_CPU_MS_PER_S."""
    last_cpu, last_t, quiet = cpu_switches(pids_fn())[0], mono_ms(), None
    while mono_ms() < t0 + limit_ms:
        time.sleep(0.25)
        cpu, now = cpu_switches(pids_fn())[0], mono_ms()
        rate = (cpu - last_cpu) / 1e6 / ((now - last_t) / 1000.0)
        last_cpu, last_t = cpu, now
        if rate < SETTLE_CPU_MS_PER_S:
            quiet = quiet or now - 250
            if now - quiet >= 1000:
                return quiet
        else:
            quiet = None
    return None


def meminfo():
    m = {}
    for line in open('/proc/meminfo'):
        k, v = line.split(':', 1)
        m[k] = int(v.split()[0])
    return {'available_mib': round(m['MemAvailable'] / 1024), 'swap_used_mib':
            round((m['SwapTotal'] - m['SwapFree']) / 1024)}


def saver_state(app_pids):
    try:
        st = json.load(open(SAVER_STATUS))
    except (OSError, ValueError):
        return {'status': 'unreadable'}
    return {'enabled': st.get('enabled'), 'saved_mib': round(st.get('saved', 0) / 2**20, 1),
            'apps': [{'app': a.get('app'), 'state': a.get('state'),
                      'saved_mib': round(a.get('saved', 0) / 2**20, 1)} for a in st.get('apps', [])]}


# ---------------------------------------------------------------- frames ----------------------


WL_TOD = re.compile(r'^\[(\d\d):(\d\d):(\d\d\.\d+)\]')
WL_OLD = re.compile(r'^\[\s*(\d+)\.(\d{3})\]')


def wl_ms(line):
    """Time of a protocol log line as wall-clock ms since the epoch. The system library prints
    the time of day (`[HH:MM:SS.micro]`); a client with its own older copy of the library
    (`electron42`) prints the wall clock in microseconds modulo 2^32 (`[ms.micro]`), which is
    unwrapped to the latest time not after now."""
    now = time.time()
    m = WL_TOD.match(line)
    if m:
        lt = time.localtime(now)
        midnight = time.mktime((lt.tm_year, lt.tm_mon, lt.tm_mday, 0, 0, 0, 0, 0, -1))
        t = midnight + int(m.group(1)) * 3600 + int(m.group(2)) * 60 + float(m.group(3))
        return (t - 86400 if t > now + 1 else t) * 1000.0
    m = WL_OLD.match(line)
    if m:
        logged = int(m.group(1)) * 1000 + int(m.group(2))
        now_us = int(now * 1e6)
        return (now_us - ((now_us - logged) % (1 << 32))) / 1000.0
    return None


class WlLog:
    """The app's Wayland protocol log (WAYLAND_DEBUG=client on its stderr): every
    `wl_surface.commit` is a frame the app handed to the compositor. Read from an offset, so
    one log serves several measurements."""

    def __init__(self, path):
        self.path = path
        self.mark()

    def mark(self):
        try:
            self.off = os.path.getsize(self.path)
        except OSError:
            self.off = 0

    def commits(self):
        out = []
        try:
            with open(self.path, 'rb') as f:
                f.seek(self.off)
                for raw in f:
                    if b'-> wl_surface#' in raw and b'.commit()' in raw:
                        t = wl_ms(raw.decode(errors='replace'))
                        if t is not None:
                            out.append(t)
        except OSError:
            pass
        return out


VK_PRESS = re.compile(r'zwp_virtual_keyboard_v1#\d+\.key\(\d+, \d+, 1\)')


def pct(v, p):
    if not v:
        return None
    s = sorted(v)
    return round(s[min(len(s) - 1, int(round((len(s) - 1) * p / 100.0)))], 1)


def frame_stats(stamps, changes, refresh_hz):
    lat, missed = [], 0
    for t in stamps:
        hit = [c for c in changes if t < c <= t + LAT_WINDOW_MS]
        if hit:
            lat.append(hit[0] - t)
        else:
            missed += 1
    gaps = [b - a for a, b in zip(changes, changes[1:]) if b - a < ANIM_GAP_MS]
    budget = 2000.0 / refresh_hz
    return {'inputs': len(stamps), 'no_change': missed,
            'latency_p50_ms': pct(lat, 50), 'latency_p95_ms': pct(lat, 95),
            'frames': len(changes), 'interval_p50_ms': pct(gaps, 50),
            'interval_p95_ms': pct(gaps, 95), 'interval_p99_ms': pct(gaps, 99),
            'long_frames_pct': round(100.0 * sum(g > budget for g in gaps) / len(gaps), 1)
            if gaps else None}


# ---------------------------------------------------------------- one app ---------------------


class Bench:
    def __init__(self, args):
        self.a = args
        self.sess = Session(args.user)
        self.events = Events(self.sess.events)
        self.payload = args.payload
        self.fixtures = os.path.join(args.payload, 'fixtures')
        self.url = f'http://127.0.0.1:{args.port}'
        self.probe = {'vk': os.path.join(args.payload, 'bin', 'vk')}
        self.wl_dir = os.path.join(args.payload, 'wl')
        os.makedirs(self.wl_dir, exist_ok=True)
        os.chmod(self.wl_dir, 0o1777)
        self.refresh = self._refresh()

    def _refresh(self):
        try:
            mons = json.loads(self.sess.ctl('j/monitors'))
            return float(mons[0].get('refreshRate', 60.0))
        except (OSError, ValueError, IndexError):
            return 60.0

    def subst(self, s):
        return (s.replace('{url}', self.url).replace('{fixtures}', self.fixtures)
                .replace('{payload}', self.payload))

    def _vk(self, args, stamps):
        r = self.sess.as_user(['env', 'WAYLAND_DEBUG=client', self.probe['vk']] + args,
                              capture_output=True, text=True)
        if stamps is not None:
            for line in r.stderr.splitlines():
                if VK_PRESS.search(line):
                    stamps.append(wl_ms(line))
                    break

    def text(self, ch, stamps=None):
        self._vk([ch], stamps)

    def keyst(self, name, stamps):
        self._vk(['-k', name], stamps)

    def drop_caches(self):
        os.sync()
        with open('/proc/sys/vm/drop_caches', 'w') as f:
            f.write('3\n')

    def workspace(self, n):
        self.sess.ctl(f'dispatch workspace {n}')
        time.sleep(0.5)

    def launch(self, app, cold, wl_log=None):
        """Start the app; returns (window dict, pids fn, timings) or raises RuntimeError.
        `wl_log`: write the app's protocol log there (frames under interaction)."""
        self.workspace(1)
        before = {c.get('address') for c in self.sess.clients()}
        if cold:
            self.drop_caches()
        cls = re.compile(app['class'], re.I)
        env = {k: self.subst(v) for k, v in app.get('env', {}).items()}
        base = app.get('wl_argv', app['argv']) if wl_log else app['argv']
        argv = [self.subst(x) for x in base]
        exe = os.path.basename(argv[0])
        if wl_log:
            env['WAYLAND_DEBUG'] = 'client'
            argv = ['sh', '-c', 'exec "$@" 2>>"$0"', wl_log] + argv
        old = set(ptable())
        t0 = mono_ms()
        self.sess.exec(argv, env)
        limit = t0 + 1000.0 * app.get('start_timeout_s', 90)
        win = None
        while mono_ms() < limit and win is None:
            for t, addr, c in self.events.since(t0):
                if addr not in before and cls.search(c):
                    win = (t, addr, c)
                    break
            time.sleep(0.01)
        if win is None:
            raise RuntimeError('no window within %ss' % app.get('start_timeout_s', 90))
        t_win, addr, c = win
        client = next((x for x in self.sess.clients() if x.get('address') == addr), None)
        if client is None:
            raise RuntimeError('window vanished')
        pid = client['pid']

        def pids():
            return app_pids(pid, exe, old)

        t_set = settle(pids, t_win, 60000.0)
        times = {'window_ms': round(t_win - t0), 'settled_ms': round(t_set - t0) if t_set else None,
                 'class': c}
        return client, pids, times

    def close(self, pids_fn):
        pids = pids_fn()
        for c in self.sess.clients():
            if c.get('pid') in pids:
                self.sess.ctl('dispatch closewindow address:' + c['address'])
        end = time.monotonic() + 10
        while time.monotonic() < end and any(os.path.exists(f'/proc/{p}') for p in pids):
            time.sleep(0.2)
        for sig in (signal.SIGTERM, signal.SIGKILL):
            alive = [p for p in pids if os.path.exists(f'/proc/{p}')]
            for p in alive:
                try:
                    os.kill(p, sig)
                except OSError:
                    pass
            if alive:
                time.sleep(3)
        time.sleep(2)

    def close_new(self, app):
        """After a failure: close every window whose class matches the row, and stop processes
        started from its command (so a failed row cannot leave windows for the next one)."""
        cls = re.compile(app['class'], re.I)
        for c in self.sess.clients():
            if cls.search(c.get('class', '')):
                self.sess.ctl('dispatch closewindow address:' + c['address'])
        time.sleep(3)
        exe = os.path.basename(self.subst(app['argv'][0]))
        for p in ptable():
            if p != os.getpid() and any(os.path.basename(a.decode(errors='replace')) == exe
                                        for a in cmdline(p)[:2]):
                try:
                    os.kill(p, signal.SIGTERM)
                except OSError:
                    pass

    def script(self, kind, scroll, typing):
        """The scripted interaction: 15 pages down and up (200 ms apart), then 43 characters
        typed 60 ms apart (after Tab, so a page's text field has the focus)."""
        if 'scroll' in kind:
            for k in ['Page_Down'] * 15 + ['Page_Up'] * 15:
                self.keyst(k, scroll)
                time.sleep(0.2)
        if 'type' in kind:
            if 'scroll' in kind:
                self.keyst('Tab', None)
                time.sleep(0.5)
            for ch in 'the quick brown fox jumps over the lazy dog':
                if ch == ' ':
                    self.keyst('space', typing)
                else:
                    self.text(ch, typing)
                time.sleep(0.06)
        time.sleep(1.0)

    def interact(self, app, wl, pids_fn):
        kind = app.get('interact', 'none')
        scroll, typing = [], []
        time.sleep(1.0)
        wl.mark()
        c0 = cpu_switches(pids_fn())[0]
        t0 = mono_ms()
        self.script(kind, scroll, typing)
        cpu_ms = (cpu_switches(pids_fn())[0] - c0) / 1e6
        dur = (mono_ms() - t0) / 1000.0
        commits = wl.commits()
        out = {'cpu_ms_s': round(cpu_ms / dur, 1), 'commits': len(commits)}
        if scroll:
            out['scroll'] = frame_stats(scroll, commits, self.refresh)
        if typing:
            out['type'] = frame_stats(typing, commits, self.refresh)
        try:
            out['compositor'] = json.loads(self.sess.ctl('wolstats'))
        except (OSError, ValueError):
            pass
        return out

    def come_back(self, app, win, pids, wl=None):
        """Back from the background: show the app, send one key. With a protocol log: key to
        the next frame; always: the time until the tree's CPU settles and the page faults."""
        self.workspace(1)
        win = next((c for c in self.sess.clients() if c.get('pid') in pids()), win)
        self.sess.ctl('dispatch focuswindow address:' + win['address'])
        time.sleep(0.3)
        if wl:
            wl.mark()
        f0, stamps = faults(pids()), []
        t0 = mono_ms()
        self.keyst('Page_Down' if 'scroll' in app.get('interact', '') else 'Shift_L', stamps)
        settled = settle(pids, t0, 30000.0)
        out = {'settled_ms': round(settled - t0) if settled else None,
               'faults': {k: v - f0[k] for k, v in faults(pids()).items()}}
        if wl:
            out.update(frame_stats(stamps, wl.commits(), self.refresh))
        return out

    def one(self, app, mode):
        r = {'id': app['id'], 'mode': mode}
        try:
            win, pids, r['cold'] = self.launch(app, cold=True)
            self.close(pids)
            warm = []
            for i in range(self.a.warm):
                win, pids, t = self.launch(app, cold=False)
                warm.append(t)
                if i < self.a.warm - 1:
                    self.close(pids)
            r['warm'] = warm
            r['warm_median'] = {k: statistics.median([w[k] for w in warm if w[k] is not None])
                                if any(w[k] is not None for w in warm) else None
                                for k in ('window_ms', 'settled_ms')}
            r['mem_launch'] = rollup(pids())
            r['procs'] = len(pids())
            self.sess.ctl('dispatch movecursor 1270 400')
            r['fg_idle'] = self.idle_phase(pids, self.a.idle)
            # Background: another workspace, so no window of the app shows (Memory Saver's cue).
            self.workspace(9)
            r['bg_idle'] = self.idle_phase(pids, self.a.idle)
            r['bg_idle']['saver'] = saver_state(pids())
            r['bg_idle']['system'] = meminfo()
            r['return'] = self.come_back(app, win, pids)
            r['mem_return'] = rollup(pids())
            self.close(pids)
            if app.get('interact', 'none') != 'none':
                # A second warm start with the protocol log: frames under interaction, and the
                # first frame after the background (the log costs CPU, so not for idle numbers).
                path = os.path.join(self.wl_dir, f"{app['id']}-{mode}.log")
                if os.path.exists(path):
                    os.unlink(path)
                win, pids, _ = self.launch(app, cold=False, wl_log=path)
                wl = WlLog(path)
                r['interact'] = self.interact(app, wl, pids)
                self.workspace(9)
                time.sleep(self.a.back)
                saver = saver_state(pids())
                r['return_frame'] = self.come_back(app, win, pids, wl)
                r['return_frame']['saver'] = saver
                self.close(pids)
                os.unlink(path)
            r['ok'] = True
        except (RuntimeError, OSError, KeyError) as e:
            r['ok'], r['error'] = False, str(e)
            log(app['id'], mode, 'failed:', e)
            self.close_new(app)
            self.workspace(1)
        return r

    def idle_phase(self, pids, secs):
        """`secs` without input; CPU and wakeups over the second half, memory at the end."""
        time.sleep(secs / 2)
        out = idle_cost(pids, secs / 2)
        ps = pids()
        out['mem'] = rollup(ps)
        out['mem']['roles'] = roles(ps)
        return out

    def features(self, app, reports):
        """Load features.html (features.js) and keep what it posts back."""
        out = os.path.join(reports, app['id'] + '.json')
        if os.path.exists(out):
            os.unlink(out)
        page = f"features.html?engine={app['id']}"
        probe = dict(app, argv=[x.replace('page.html', page) for x in app['argv']])
        try:
            _, pids, _ = self.launch(probe, cold=False)
        except RuntimeError as e:
            self.close_new(app)
            return {'error': str(e)}
        end = time.monotonic() + 30
        while time.monotonic() < end and not os.path.exists(out):
            time.sleep(0.5)
        time.sleep(0.5)
        self.close(pids)
        try:
            return json.load(open(out))
        except (OSError, ValueError):
            return {'error': 'the page posted nothing within 30 s'}

    def prime(self, app):
        """First run (profiles, caches, first-run pages) so cold and warm runs measure starts."""
        try:
            _, pids, t = self.launch(app, cold=False)
            time.sleep(self.a.prime)
            self.close(pids)
            return t
        except RuntimeError as e:
            self.close_new(app)
            return {'error': str(e)}


# ---------------------------------------------------------------- set-up ----------------------


def fixtures(d):
    os.makedirs(d, exist_ok=True)
    paras = []
    for i in range(400):
        paras.append(f'<section><h2>Section {i}</h2><p>' + ' '.join(
            f'word{(i * 7 + j) % 97}' for j in range(90)) + '</p><div class="card c%d"></div>'
            '</section>' % (i % 5))
    page = ('<!doctype html><meta charset="utf-8"><title>bench</title><style>body{font:16px '
            'sans-serif;margin:0 auto;max-width:860px;padding:24px}.card{height:80px;border-radius'
            ':12px;margin:12px 0}.c0{background:linear-gradient(90deg,#f6a,#a6f)}.c1{background:'
            'linear-gradient(90deg,#6af,#6fa)}.c2{background:#eee;box-shadow:0 4px 16px #0003}.c3'
            '{background:radial-gradient(#fa6,#f66)}.c4{background:#333}textarea{position:fixed;'
            'right:16px;bottom:16px;width:320px;height:80px}</style><textarea aria-label="t">'
            '</textarea>' + ''.join(paras) + '<script>let n=0;document.addEventListener("scroll",'
            '()=>{n++});setInterval(()=>{document.title="bench "+n},1000)</script>')
    open(os.path.join(d, 'page.html'), 'w').write(page)
    src = []
    for i in range(4000):
        src.append(f'fn item_{i}(x: u64) -> u64 {{ x.wrapping_mul({i * 2654435761 % 1000003})'
                   f' ^ (x >> {i % 31}) }} // line {i}')
    open(os.path.join(d, 'long.rs'), 'w').write('\n'.join(src) + '\n')
    shutil.copy(os.path.join(HERE, 'features.js'), d)
    open(os.path.join(d, 'features.html'), 'w').write(
        '<!doctype html><meta charset="utf-8"><title>features</title><body>'
        '<script src="features.js"></script>')


def serve(d, port, reports):
    """The fixtures over HTTP; a POST to /report?engine=ID (features.js) lands in `reports`."""
    os.makedirs(reports, exist_ok=True)

    class H(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *a, **k):
            super().__init__(*a, directory=d, **k)

        def do_POST(self):
            m = re.search(r'engine=([a-z0-9-]+)', self.path)
            body = self.rfile.read(min(int(self.headers.get('Content-Length', 0)), 1 << 20))
            if m:
                open(os.path.join(reports, m.group(1) + '.json'), 'wb').write(body)
            self.send_response(204)
            self.end_headers()

        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(('127.0.0.1', port), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def identity(sess, apps):
    def sh(cmd):
        try:
            return subprocess.run(cmd, shell=True, capture_output=True, text=True,
                                  timeout=30).stdout.strip()
        except subprocess.TimeoutExpired:
            return ''
    pkgs = sorted({p for a in apps for p in a.get('packages', [])} |
                  {'hyprland-wolos', 'linux-wolos', 'wolos-system', 'mesa'})
    osr = dict(line.split('=', 1) for line in open('/etc/os-release').read().split('\n') if '=' in line)
    return {
        'os': osr.get('PRETTY_NAME', '').strip('"') + ' ' + osr.get('VERSION_ID', ''),
        'kernel': os.uname().release,
        'base_version': sh('cat /usr/share/wolos/base.version 2>/dev/null'),
        'cpu': sh("grep -m1 'model name' /proc/cpuinfo | cut -d: -f2").strip(),
        'cpus': os.cpu_count(),
        'mem_mib': round(int(open('/proc/meminfo').readline().split()[1]) / 1024),
        'gpu_driver': sh('for d in /sys/class/drm/card*/device/driver; do basename "$(readlink '
                         '"$d")"; done | sort -u | tr "\\n" " "'),
        'swap': sh("tail -n +2 /proc/swaps | awk '{print $1, $3/1024 \" MiB\"}'"),
        'zram_algo': sh('cat /sys/block/zram0/comp_algorithm 2>/dev/null'),
        'thp': sh('cat /sys/kernel/mm/transparent_hugepage/enabled'),
        'packages': sh('pacman -Q ' + ' '.join(pkgs) + ' 2>/dev/null').split('\n'),
        'monitor': sh(f'runuser -u {sess.user} -- env XDG_RUNTIME_DIR={sess.runtime} '
                      f'HYPRLAND_INSTANCE_SIGNATURE={sess.sig} hyprctl monitors | sed -n 2p').strip(),
    }


def ensure_saver_session(sess):
    """The Memory Saver session side (`wol-realtime` with no arguments) must run for saving to
    happen at all; start it in the session when nothing did."""
    if Session._pid_of('wol-realtime'):
        return 'running'
    sess.exec(['wol-realtime'])
    time.sleep(3)
    return 'started-by-harness' if Session._pid_of('wol-realtime') else 'not-running'


def hold_idle(sess, password):
    """No screen lock, display-off or power-off during the run: stop the idle daemon (restarted
    afterwards with the same command line) and unlock a locked session with `password`."""
    cmds = []
    for p in os.listdir('/proc'):
        if p.isdigit():
            try:
                if open(f'/proc/{p}/comm').read().strip() == 'hypridle':
                    cmds.append(open(f'/proc/{p}/cmdline').read().split('\0')[:-1])
                    os.kill(int(p), signal.SIGTERM)
            except OSError:
                pass
    if Session._pid_of('wol-lock'):
        if not password:
            raise SystemExit('webapps: the session is locked; set WOLBENCH_PASSWORD to unlock it')
        vk = os.path.join(sess.payload, 'bin', 'vk')
        for _ in range(64):
            sess.as_user([vk, '-k', 'BackSpace'])
        sess.as_user([vk, password])
        sess.as_user([vk, '-k', 'Return'])
        time.sleep(3)
        if Session._pid_of('wol-lock'):
            raise SystemExit('webapps: could not unlock the session')
    return cmds


def release_idle(sess, cmds):
    for argv in cmds:
        sess.exec(argv)


def set_saver(sess, on):
    d = os.path.join(sess.home, '.config/wolos')
    os.makedirs(d, exist_ok=True)
    f = os.path.join(d, 'memory-saver.json')
    open(f, 'w').write(json.dumps({'enabled': on, 'nap': False, 'exclude': []}) + '\n')
    os.chown(d, sess.uid, -1)
    os.chown(f, sess.uid, -1)
    time.sleep(5)  # the session side re-reads the file every 2 s


def cmd_run(a):
    spec = json.load(open(a.apps))
    apps = [x for x in spec['apps'] if not a.only or x['id'] in a.only.split(',')]
    sess = Session(a.user)
    sess.payload = a.payload
    idle_cmds = hold_idle(sess, os.environ.get('WOLBENCH_PASSWORD', ''))
    try:
        run_all(a, apps, sess)
    finally:
        set_saver(sess, True)
        release_idle(sess, idle_cmds)


def run_all(a, apps, sess):
    fixtures(os.path.join(a.payload, 'fixtures'))
    reports = os.path.join(a.payload, 'features')
    serve(os.path.join(a.payload, 'fixtures'), a.port, reports)
    b = Bench(a)
    res = {'schema': 1, 'started': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
           'identity': identity(sess, apps), 'refresh_hz': b.refresh,
           'settings': {'warm_runs': a.warm, 'idle_s': a.idle, 'prime_s': a.prime,
                        'back_s': a.back},
           'saver_session': ensure_saver_session(sess), 'apps': [], 'prime': {}}
    usable = []
    for app in apps:
        exe = b.subst(app['argv'][0])
        if not (os.path.exists(exe) or shutil.which(exe, path='/usr/local/bin:/usr/bin')):
            res['apps'].append({'id': app['id'], 'ok': False, 'error': 'not installed: ' + exe})
            continue
        usable.append(app)
        log('prime', app['id'])
        res['prime'][app['id']] = b.prime(app)
    res['features'] = {}
    for app in usable:
        if app.get('features'):
            log('features', app['id'])
            res['features'][app['id']] = b.features(app, reports)
    for mode in a.modes.split(','):
        set_saver(sess, mode == 'on')
        for app in usable:
            log(mode, app['id'])
            res['apps'].append(b.one(app, mode))
            json.dump(res, open(a.out, 'w'), indent=1)
    res['finished'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    res['idle_daemon'] = 'stopped for the run'
    json.dump(res, open(a.out, 'w'), indent=1)
    log('wrote', a.out)


# ---------------------------------------------------------------- report ----------------------


def g(d, *path):
    for p in path:
        if not isinstance(d, dict):
            return None
        d = d.get(p)
    return d


def fmt(v):
    return '-' if v is None else (f'{v:g}' if isinstance(v, (int, float)) else str(v))


def cmd_report(a):
    res = json.load(open(a.result))
    rows = {}
    for r in res['apps']:
        rows.setdefault(r['id'], {})[r.get('mode', '-')] = r
    out = []
    head = ['App', 'Saver', 'Cold window / settled ms', 'Warm window / settled ms',
            'PSS launch MiB', 'PSS 2 min front MiB', 'PSS 2 min back MiB', 'Swap back MiB',
            'Idle front CPU ms/s', 'Idle front wakeups/s', 'Idle back CPU ms/s',
            'Scroll key to frame p50/p95 ms', 'Scroll frame interval p50/p95 ms',
            'Type key to frame p50/p95 ms', 'Back: settled ms / faults', 'Back: key to frame ms']
    out.append('| ' + ' | '.join(head) + ' |')
    out.append('|' + '---|' * len(head))
    for app_id, modes in rows.items():
        for mode, r in modes.items():
            if not r.get('ok'):
                out.append(f'| `{app_id}` | {mode} | not measured: {r.get("error")} |' +
                           ' |' * (len(head) - 3))
                continue
            cells = [
                f'`{app_id}`', mode,
                f"{fmt(g(r, 'cold', 'window_ms'))} / {fmt(g(r, 'cold', 'settled_ms'))}",
                f"{fmt(g(r, 'warm_median', 'window_ms'))} / {fmt(g(r, 'warm_median', 'settled_ms'))}",
                fmt(g(r, 'mem_launch', 'pss')), fmt(g(r, 'fg_idle', 'mem', 'pss')),
                fmt(g(r, 'bg_idle', 'mem', 'pss')), fmt(g(r, 'bg_idle', 'mem', 'swap_pss')),
                fmt(g(r, 'fg_idle', 'cpu_ms_s')), fmt(g(r, 'fg_idle', 'wakeups_s')),
                fmt(g(r, 'bg_idle', 'cpu_ms_s')),
                f"{fmt(g(r, 'interact', 'scroll', 'latency_p50_ms'))} / "
                f"{fmt(g(r, 'interact', 'scroll', 'latency_p95_ms'))}",
                f"{fmt(g(r, 'interact', 'scroll', 'interval_p50_ms'))} / "
                f"{fmt(g(r, 'interact', 'scroll', 'interval_p95_ms'))}",
                f"{fmt(g(r, 'interact', 'type', 'latency_p50_ms'))} / "
                f"{fmt(g(r, 'interact', 'type', 'latency_p95_ms'))}",
                f"{fmt(g(r, 'return', 'settled_ms'))} / "
                f"{fmt((g(r, 'return', 'faults', 'minor') or 0) + (g(r, 'return', 'faults', 'major') or 0))}",
                fmt(g(r, 'return_frame', 'latency_p50_ms')),
            ]
            out.append('| ' + ' | '.join(cells) + ' |')
    print('\n'.join(out))


def main():
    ap = argparse.ArgumentParser(prog='webapps.py')
    sub = ap.add_subparsers(dest='cmd', required=True)
    r = sub.add_parser('run')
    r.add_argument('--apps', default=os.path.join(HERE, 'apps.json'))
    r.add_argument('--payload', default=HERE, help='directory with bin/vk (and lib/)')
    r.add_argument('--user', default=os.environ.get('WOLBENCH_USER', 'alice'))
    r.add_argument('--out', default='/var/tmp/wolbench-result.json')
    r.add_argument('--only', default='', help='comma-separated app ids')
    r.add_argument('--modes', default='on,off')
    r.add_argument('--warm', type=int, default=3)
    r.add_argument('--idle', type=float, default=120.0, help='seconds idle in front and in back')
    r.add_argument('--prime', type=float, default=20.0)
    r.add_argument('--back', type=float, default=60.0,
                   help='seconds in the background before the first-frame-back measurement')
    r.add_argument('--port', type=int, default=8765)
    p = sub.add_parser('report')
    p.add_argument('result')
    a = ap.parse_args()
    if a.cmd == 'run':
        if os.geteuid() != 0:
            raise SystemExit('webapps.py run: run as root (drop caches, read every app process)')
        cmd_run(a)
    else:
        cmd_report(a)


if __name__ == '__main__':
    main()
